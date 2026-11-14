// Calling a friend and answering one, shared by the friends list and the
// ringing card. The room itself is the backend's: it starts one if needed
// and rings with a short-lived invite to it.

import { api } from "./api";
import { go } from "./nav.svelte";
import { showToast } from "./toast.svelte";
import type { RingOutcome } from "./types";

function describe(outcome: RingOutcome, name: string): string | null {
  switch (outcome.outcome) {
    case "answered":
      return `${name} is joining…`;
    case "declined":
      return `${name} declined the call.`;
    case "no_answer":
      return `${name} didn't answer.`;
    case "hung_up":
      return null;
    case "busy":
      return `${name} is busy right now.`;
    case "not_friends":
      return `${name} doesn't have you as a friend any more.`;
    case "unreachable":
      return `Couldn't reach ${name}. They may be offline.`;
  }
}

/** Rings `id`, showing the call (Home) while it rings. */
export async function callFriend(id: string, name: string): Promise<void> {
  go("home");
  try {
    const message = describe(await api.callFriend(id), name);
    if (message) showToast(message);
  } catch (e) {
    showToast(String(e));
  }
}

/** Answers a ring; yes joins their room and shows it. */
export async function answerRing(id: string, accept: boolean): Promise<void> {
  try {
    await api.answerRing(id, accept);
    if (accept) go("home");
  } catch (e) {
    showToast(String(e));
  }
}
