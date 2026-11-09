// The mic and deafen toggles, with the short sound each one makes. Every
// button that flips them goes through here, so they all sound the same.

import deafenUrl from "../../audio/deafen.mp3";
import exitUrl from "../../audio/exit.mp3";
import joinUrl from "../../audio/join.mp3";
import muteUrl from "../../audio/mute.mp3";
import undeafenUrl from "../../audio/undeafen.mp3";
import unmuteUrl from "../../audio/unmute.mp3";
import { api } from "./api";

type Cue = "mute" | "unmute" | "deafen" | "undeafen" | "join" | "exit";

const urls: Record<Cue, string> = {
  mute: muteUrl,
  unmute: unmuteUrl,
  deafen: deafenUrl,
  undeafen: undeafenUrl,
  join: joinUrl,
  exit: exitUrl,
};

// Played through Web Audio rather than <audio> elements. On Linux an <audio>
// element has WebKitGTK's media player stream the file from the app's own URL
// scheme, which it fails to load (a FormatError, and silence). Fetching the
// bytes ourselves and decoding them once works the same on every platform.
let context: AudioContext | null = null;
try {
  context = new AudioContext();
} catch {
  // No audio output at all: the toggles still work, silently.
}

// Decoded once up front, so the first click is not silent while a file loads.
const cues = new Map<Cue, Promise<AudioBuffer | null>>(
  (Object.keys(urls) as Cue[]).map((cue) => [cue, decode(urls[cue])]),
);

async function decode(url: string): Promise<AudioBuffer | null> {
  if (!context) return null;
  try {
    const bytes = await (await fetch(url)).arrayBuffer();
    return await context.decodeAudioData(bytes);
  } catch {
    return null;
  }
}

let playing: AudioBufferSourceNode | null = null;

export function play(cue: Cue) {
  // A sound that cannot play (no output device, say) is no reason to fail
  // the toggle itself, so every failure here ends in silence.
  void cues.get(cue)?.then((buffer) => {
    if (!context || !buffer) return;
    // Quick toggles stop the sound before this one rather than overlapping it.
    try {
      playing?.stop();
    } catch {
      // Already finished.
    }
    // A context made before any click starts suspended in some webviews.
    void context.resume().catch(() => {});
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    source.start();
    playing = source;
  });
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
