// The statuses someone can show friends, in the order they are offered.
// Only a hint: nobody is kept from reaching you by it.

import { api } from "./api";
import { showToast } from "./toast.svelte";
import type { Status } from "./types";

export const STATUSES: { value: Status; label: string; detail: string }[] = [
  { value: "online", label: "Online", detail: "Friends see you're around." },
  { value: "away", label: "Away", detail: "You're around, but not at the screen." },
  { value: "busy", label: "Do not disturb", detail: "Calls from friends still show, but don't ring." },
  { value: "invisible", label: "Invisible", detail: "Friends see you as offline." },
];

export function statusLabel(status: Status): string {
  return STATUSES.find((s) => s.value === status)?.label ?? "Online";
}

export async function setStatus(status: Status): Promise<void> {
  try {
    await api.setStatus(status);
  } catch (e) {
    showToast(String(e));
  }
}
