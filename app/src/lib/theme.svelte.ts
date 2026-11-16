// The colour theme. A preference of this machine only, so it lives in local
// storage. Applied as data-theme on the root before the app first renders,
// so a stored theme never flashes the default first.

export type Theme = "night" | "smoke" | "midnight";

export const THEMES: { value: Theme; label: string }[] = [
  { value: "night", label: "Night" },
  { value: "smoke", label: "Smoke" },
  { value: "midnight", label: "Midnight" },
];

const KEY = "noxid.theme";
const DEFAULT: Theme = "night";

function load(): Theme {
  try {
    const stored = localStorage.getItem(KEY);
    if (THEMES.some((t) => t.value === stored)) return stored as Theme;
  } catch {
    // Storage unavailable: the default it is.
  }
  return DEFAULT;
}

export const appearance = $state<{ theme: Theme }>({ theme: load() });

function apply(theme: Theme) {
  document.documentElement.dataset.theme = theme;
}

apply(appearance.theme);

export function setTheme(theme: Theme) {
  if (theme === appearance.theme) return;
  appearance.theme = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Not remembered, but still applied for this session.
  }
  // A crossfade between the two looks where the webview can, an instant
  // switch where it cannot.
  if (document.startViewTransition) document.startViewTransition(() => apply(theme));
  else apply(theme);
}
