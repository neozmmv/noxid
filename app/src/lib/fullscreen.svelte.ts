import { getCurrentWindow } from "@tauri-apps/api/window";

// Whether the share on stage fills the screen. Shared, because the stage's
// own button and the right-click menus on every share all toggle it.
export const view = $state({ fullscreen: false });

export async function setFullscreen(on: boolean): Promise<void> {
  view.fullscreen = on;
  // The window itself goes fullscreen, not just the element: WebKitGTK and
  // WebView2 disagree on element fullscreen, and this behaves the same on both.
  try {
    await getCurrentWindow().setFullscreen(on);
  } catch {
    // Without the window permission the stage still fills the window.
  }
}
