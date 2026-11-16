// The colour theme and the accent. Preferences of this machine only, so they
// live in local storage. Applied as data-theme and data-accent on the root
// before the app first renders, so a stored choice never flashes the default
// first. The accent is its own choice: changing theme keeps it.

import { flushSync } from "svelte";

export type Theme = "night" | "smoke" | "midnight";
export type Accent = "gold" | "sky" | "lilac" | "rose" | "teal" | "sand";

export const THEMES: { value: Theme; label: string }[] = [
  { value: "night", label: "Night" },
  { value: "smoke", label: "Smoke" },
  { value: "midnight", label: "Midnight" },
];

export const ACCENTS: { value: Accent; label: string }[] = [
  { value: "gold", label: "Gold" },
  { value: "sky", label: "Sky" },
  { value: "lilac", label: "Lilac" },
  { value: "rose", label: "Rose" },
  { value: "teal", label: "Teal" },
  { value: "sand", label: "Sand" },
];

const THEME_KEY = "noxid.theme";
const ACCENT_KEY = "noxid.accent";

function load<T extends string>(key: string, options: { value: T }[], fallback: T): T {
  try {
    const stored = localStorage.getItem(key);
    if (options.some((o) => o.value === stored)) return stored as T;
  } catch {
    // Storage unavailable: the default it is.
  }
  return fallback;
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not remembered, but still applied for this session.
  }
}

export const appearance = $state<{ theme: Theme; accent: Accent }>({
  theme: load(THEME_KEY, THEMES, "night"),
  accent: load(ACCENT_KEY, ACCENTS, "gold"),
});

function apply() {
  document.documentElement.dataset.theme = appearance.theme;
  document.documentElement.dataset.accent = appearance.accent;
}

apply();

/** Applies a change with a crossfade between the two looks where the
 *  webview can, at once where it cannot. */
function change(update: () => void) {
  if (document.startViewTransition) {
    // Flushed inside the callback, so the picker's new selection is part of
    // the "after" picture rather than popping in once the fade is done.
    document.startViewTransition(() => {
      flushSync(update);
      apply();
    });
  } else {
    update();
    apply();
  }
}

export function setTheme(theme: Theme) {
  if (theme === appearance.theme) return;
  store(THEME_KEY, theme);
  change(() => (appearance.theme = theme));
}

export function setAccent(accent: Accent) {
  if (accent === appearance.accent) return;
  store(ACCENT_KEY, accent);
  change(() => (appearance.accent = accent));
}
