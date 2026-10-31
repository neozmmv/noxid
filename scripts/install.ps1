<#
.SYNOPSIS
Installs Noxid for the current user and makes it findable in Windows Search.

.EXAMPLE
    .\install.ps1                   # latest GitHub release
    .\install.ps1 -Version v0.1.0   # a specific release
    .\install.ps1 -Local            # build from this checkout, then install
    .\install.ps1 -Uninstall

Or straight from the web:
    irm https://raw.githubusercontent.com/neozmmv/noxid/master/scripts/install.ps1 | iex

Installs per user, under %LOCALAPPDATA%, so no Administrator prompt. The
installer adds a Start Menu shortcut, which is what Windows Search indexes;
this script checks the shortcut is there and creates it if not.
#>
[CmdletBinding()]
param(
    [switch]$Local,
    [switch]$Uninstall,
    [string]$Version,
    [string]$Repo = 'neozmmv/noxid',
    # Start Noxid once it is installed.
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is many times slower with it
$Asset = 'noxid-windows-x86_64-setup.exe'
$ShortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'Noxid.lnk'

function Info($msg) { Write-Host "==> $msg" }

# The uninstall entry the installer wrote, which says where Noxid lives.
function Get-Installation {
    $roots = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
             'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        $entry = Get-ChildItem $root | ForEach-Object { Get-ItemProperty $_.PSPath } |
            Where-Object { $_.DisplayName -eq 'Noxid' } | Select-Object -First 1
        if ($entry) { return $entry }
    }
    return $null
}

function Get-InstalledExe {
    $entry = Get-Installation
    if (-not $entry) { return $null }
    $dir = $entry.InstallLocation
    if (-not $dir -and $entry.UninstallString) {
        $dir = Split-Path ($entry.UninstallString.Trim('"'))
    }
    if (-not $dir) { return $null }
    $exe = Join-Path $dir.Trim('"') 'noxid.exe'
    if (Test-Path $exe) { return $exe }
    return $null
}

function Invoke-Uninstall {
    $entry = Get-Installation
    if (-not $entry) {
        Info 'Noxid is not installed'
        return
    }
    $uninstaller = $entry.UninstallString.Trim('"')
    Info "uninstalling ($uninstaller)"
    Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait
    Remove-Item $ShortcutPath -Force -ErrorAction SilentlyContinue
    Info 'done. Your identity and logs in %APPDATA%\noxid were kept; delete that folder to remove them too.'
}

# Downloads the release installer and checks it against its published hash.
function Get-ReleaseInstaller($dir) {
    $base = if ($Version) {
        "https://github.com/$Repo/releases/download/$Version"
    } else {
        "https://github.com/$Repo/releases/latest/download"
    }
    $setup = Join-Path $dir $Asset
    Info "downloading $base/$Asset"
    Invoke-WebRequest -Uri "$base/$Asset" -OutFile $setup -UseBasicParsing
    Invoke-WebRequest -Uri "$base/$Asset.sha256" -OutFile "$setup.sha256" -UseBasicParsing

    $expected = ((Get-Content "$setup.sha256" -Raw).Trim() -split '\s+')[0].ToLower()
    $actual = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) {
        throw "checksum mismatch for $Asset (expected $expected, got $actual); not installing it"
    }
    Info 'checksum verified'
    return $setup
}

# Builds the installer from this checkout.
function Get-LocalInstaller {
    $root = Split-Path $PSScriptRoot
    if (-not (Test-Path (Join-Path $root 'app\package.json'))) {
        throw '-Local has to be run from the scripts folder of a Noxid checkout'
    }
    foreach ($tool in 'cargo', 'npm') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "-Local needs $tool on PATH"
        }
    }
    Push-Location (Join-Path $root 'app')
    # npm and tauri report progress on stderr. Windows PowerShell turns that into
    # error records, and under 'Stop' the first one would abort a healthy build
    # whenever output is redirected; exit codes are what decide failure here.
    $ErrorActionPreference = 'Continue'
    try {
        Info 'installing frontend dependencies'
        # Straight to the console: anything left on the pipeline would become
        # part of this function's return value.
        npm ci | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
        Info 'building the installer (the first build takes a while)'
        npx tauri build --bundles nsis | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }
    }
    finally {
        Pop-Location
    }
    $setup = Get-ChildItem (Join-Path $root 'target\release\bundle\nsis\*-setup.exe') |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $setup) { throw 'the build produced no installer' }
    return $setup.FullName
}

# Windows Search finds apps through their Start Menu shortcut. The installer
# makes one; this covers an installer that did not.
function Confirm-Shortcut($exe) {
    $existing = Get-ChildItem ([Environment]::GetFolderPath('Programs')) -Recurse -Filter 'Noxid.lnk' -ErrorAction SilentlyContinue
    if ($existing) {
        Info "Start Menu shortcut: $($existing[0].FullName)"
        return
    }
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($ShortcutPath)
    $link.TargetPath = $exe
    $link.WorkingDirectory = Split-Path $exe
    $link.Description = 'Voice calls and screen sharing'
    $link.Save()
    Info "created Start Menu shortcut: $ShortcutPath"
}

if ($Uninstall) {
    Invoke-Uninstall
    exit 0
}

if (Get-Process -Name 'noxid' -ErrorAction SilentlyContinue) {
    throw 'Noxid is running. Close it (leaving any call) and run this again.'
}

$work = Join-Path ([IO.Path]::GetTempPath()) "noxid-install-$([Guid]::NewGuid())"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $setup = if ($Local) { Get-LocalInstaller } else { Get-ReleaseInstaller $work }

    Info 'installing'
    # /S is the NSIS silent switch; the installer is per user, so no prompt.
    $run = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "the installer exited with code $($run.ExitCode)" }

    $exe = Get-InstalledExe
    if (-not $exe) { throw 'the installer finished but Noxid was not found afterwards' }
    Info "installed to $exe"
    Confirm-Shortcut $exe

    Info 'done. Search for "Noxid" in the Start menu to open it.'
    if ($Launch) { Start-Process $exe }
}
finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
