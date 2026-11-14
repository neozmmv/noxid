// The friends list as the backend last sent it. Filled once at start and
// then by the `friends` event, both registered in App.svelte only, so no
// component can subscribe twice.

import type { FriendsView } from "./types";

export const friends = $state<{ view: FriendsView | null }>({ view: null });

/** "3 min ago", "2 h ago", "yesterday"…: for last seen and missed calls. */
export function ago(at: number, now = Date.now()): string {
  const minutes = Math.round((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}
