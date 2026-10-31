#!/usr/bin/env sh
# Installs Noxid on Linux and adds it to the application menu.
#
#   ./install.sh                  latest GitHub release
#   ./install.sh --version v0.1.0 a specific release
#   ./install.sh --local          build from this checkout, then install
#   ./install.sh --uninstall
#
# Or straight from the web:
#   curl -fsSL https://raw.githubusercontent.com/neozmmv/noxid/master/scripts/install.sh | sh
#
# On Debian and Ubuntu it installs the .deb through apt, which pulls in the
# libraries it needs (asks for sudo). Elsewhere it installs the AppImage for
# the current user under ~/.local, with no sudo.

set -eu

REPO="${NOXID_REPO:-neozmmv/noxid}"
VERSION=""
LOCAL=0
UNINSTALL=0

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
APPIMAGE="$BIN_DIR/noxid.AppImage"
DESKTOP="$APP_DIR/noxid.desktop"

info() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?--version needs a tag}"; shift 2 ;;
        --local) LOCAL=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) die "unknown option $1" ;;
    esac
done

[ "$(uname -s)" = "Linux" ] || die "this script is for Linux; on Windows use install.ps1"
[ "$(uname -m)" = "x86_64" ] || die "only x86_64 builds are published so far"

has() { command -v "$1" >/dev/null 2>&1; }

sudo_cmd() {
    if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi
}

use_deb() { has apt-get && has dpkg; }

if [ "$UNINSTALL" -eq 1 ]; then
    if use_deb && dpkg -s noxid >/dev/null 2>&1; then
        info "removing the noxid package"
        sudo_cmd apt-get remove -y noxid
    fi
    rm -f "$APPIMAGE" "$DESKTOP" "$ICON_DIR/noxid.png"
    info "done. Your identity and logs in ~/.config/noxid were kept; delete that folder to remove them too."
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if use_deb; then KIND=deb; else KIND=AppImage; fi
ASSET="noxid-linux-x86_64.$KIND"

fetch() {
    if has curl; then curl -fsSL "$1" -o "$2"
    elif has wget; then wget -q "$1" -O "$2"
    else die "needs curl or wget"
    fi
}

if [ "$LOCAL" -eq 1 ]; then
    ROOT="$(cd "$(dirname "$0")/.." && pwd)"
    [ -f "$ROOT/app/package.json" ] || die "--local has to be run from a Noxid checkout"
    has cargo || die "--local needs cargo"
    has npm || die "--local needs npm"
    info "building (the first build takes a while)"
    if [ "$KIND" = deb ]; then BUNDLE=deb; else BUNDLE=appimage; fi
    (cd "$ROOT/app" && npm ci && npx tauri build --bundles "$BUNDLE")
    if [ "$KIND" = deb ]; then
        cp "$(ls -t "$ROOT"/target/release/bundle/deb/*.deb | head -n1)" "$WORK/$ASSET"
    else
        cp "$(ls -t "$ROOT"/target/release/bundle/appimage/*.AppImage | head -n1)" "$WORK/$ASSET"
    fi
else
    if [ -n "$VERSION" ]; then
        BASE="https://github.com/$REPO/releases/download/$VERSION"
    else
        BASE="https://github.com/$REPO/releases/latest/download"
    fi
    info "downloading $BASE/$ASSET"
    fetch "$BASE/$ASSET" "$WORK/$ASSET"
    fetch "$BASE/$ASSET.sha256" "$WORK/$ASSET.sha256"
    (cd "$WORK" && sha256sum -c "$ASSET.sha256" >/dev/null) || die "checksum mismatch for $ASSET; not installing it"
    info "checksum verified"
fi

if [ "$KIND" = deb ]; then
    info "installing the package (apt pulls in WebKitGTK and PipeWire if missing)"
    sudo_cmd apt-get install -y "$WORK/$ASSET"
    info "done. Open Noxid from your application menu, or run: noxid"
    exit 0
fi

info "installing the AppImage to $APPIMAGE"
mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 "$WORK/$ASSET" "$APPIMAGE"

# The AppImage carries its own icon; extract it for the menu entry.
(cd "$WORK" && "$APPIMAGE" --appimage-extract '.DirIcon' >/dev/null 2>&1) &&
    cp -L "$WORK/squashfs-root/.DirIcon" "$ICON_DIR/noxid.png" 2>/dev/null || true

cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=Noxid
Comment=Voice calls and screen sharing
Exec=$APPIMAGE
Icon=noxid
Categories=Network;Chat;AudioVideo;
Terminal=false
EOF
has update-desktop-database && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) info "note: $BIN_DIR is not on your PATH; the menu entry works regardless" ;;
esac
info "done. Open Noxid from your application menu."
