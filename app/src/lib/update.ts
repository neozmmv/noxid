// Whether a newer release than this build is out. Release builds carry their
// tag's version (the release workflow stamps it into the config), so this
// compares that against the latest release on GitHub.

import { getVersion } from "@tauri-apps/api/app";
import { api } from "./api";

type Version = [number, number, number];

function parse(text: string): Version | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(text.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

function newer(a: Version, b: Version): boolean {
  const [major, minor, patch] = a.map((n, i) => n - b[i]!) as Version;
  return major > 0 || (major === 0 && (minor > 0 || (minor === 0 && patch > 0)));
}

/** The newer release's tag (like `v0.5.0`), or null when up to date or the
 *  check could not be made. Offline or rate-limited, it just stays quiet. */
export async function newerRelease(): Promise<string | null> {
  try {
    const current = parse(await getVersion());
    // Rust makes the request: the webview's TLS can't find the system's
    // certificates inside the AppImage on every distro.
    const tag = await api.latestRelease();
    if (!current) return null;
    const latest = tag ? parse(tag) : null;
    return latest && newer(latest, current) ? tag! : null;
  } catch {
    return null;
  }
}
