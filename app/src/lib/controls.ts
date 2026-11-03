// The mic and deafen toggles, with the short sound each one makes. Every
// button that flips them goes through here, so they all sound the same.

import deafenUrl from "../../audio/deafen.mp3";
import muteUrl from "../../audio/mute.mp3";
import undeafenUrl from "../../audio/undeafen.mp3";
import unmuteUrl from "../../audio/unmute.mp3";
import { api } from "./api";

type Cue = "mute" | "unmute" | "deafen" | "undeafen";

// Loaded once up front, so the first click is not silent while a file loads.
const cues: Record<Cue, HTMLAudioElement> = {
  mute: new Audio(muteUrl),
  unmute: new Audio(unmuteUrl),
  deafen: new Audio(deafenUrl),
  undeafen: new Audio(undeafenUrl),
};
for (const audio of Object.values(cues)) audio.preload = "auto";

function play(cue: Cue) {
  // Quick toggles stop the sound before this one rather than overlapping it.
  for (const audio of Object.values(cues)) {
    audio.pause();
    audio.currentTime = 0;
  }
  // A sound that cannot play (no output device, say) is no reason to fail
  // the toggle itself.
  cues[cue].play().catch(() => {});
}

/** Turns the mic on or off. Turning it on while deafened also undeafens
 *  (the backend does that), so that is the sound it makes. */
export function setMic(on: boolean, deafened: boolean) {
  play(on ? (deafened ? "undeafen" : "unmute") : "mute");
  return api.setMic(on);
}

export function setDeafened(on: boolean) {
  play(on ? "deafen" : "undeafen");
  return api.setDeafened(on);
}
