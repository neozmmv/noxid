// The sound of a friend calling: two soft notes, again every two seconds,
// made on the spot with Web Audio rather than shipped as a file (see
// controls.ts for why Web Audio at all).

let context: AudioContext | null = null;
let timer: ReturnType<typeof setInterval> | undefined;

function chime(ctx: AudioContext) {
  const start = ctx.currentTime;
  [659.25, 987.77].forEach((frequency, i) => {
    const at = start + i * 0.18;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = frequency;
    gain.gain.setValueAtTime(0, at);
    gain.gain.linearRampToValueAtTime(0.18, at + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.001, at + 0.6);
    osc.connect(gain).connect(ctx.destination);
    osc.start(at);
    osc.stop(at + 0.65);
  });
}

export function startRinging() {
  if (timer) return;
  try {
    context ??= new AudioContext();
  } catch {
    return; // No output: ring silently.
  }
  const ctx = context;
  void ctx.resume().catch(() => {});
  chime(ctx);
  timer = setInterval(() => chime(ctx), 2000);
}

export function stopRinging() {
  clearInterval(timer);
  timer = undefined;
}
