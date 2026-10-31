// Voice levels, pushed by the backend ten times a second. Kept apart from the
// snapshot so the speaking indicators update without redrawing anything else.

import type { Levels } from "./types";

export const voice = $state<{ levels: Levels }>({ levels: {} });

/** The bottom of the meter scale, in dBFS. Quieter than this shows as empty. */
export const METER_FLOOR_DB = -60;

/** Peak level (0 to 1) as dBFS, clamped to the meter scale. */
export function toDb(level: number): number {
  if (level <= 0) return METER_FLOOR_DB;
  return Math.max(METER_FLOOR_DB, 20 * Math.log10(level));
}

/** Peak level (0 to 1) for a dBFS value. */
export function fromDb(db: number): number {
  return 10 ** (db / 20);
}

// Input sensitivity: how loud our own mic has to be for us to count as
// speaking. A preference of this machine only, so it lives in local storage.
const SENSITIVITY_KEY = "noxid.sensitivityDb";
const DEFAULT_SENSITIVITY_DB = -40;

function loadSensitivity(): number {
  try {
    const raw = localStorage.getItem(SENSITIVITY_KEY);
    const stored = raw === null ? NaN : Number(raw);
    if (Number.isFinite(stored) && stored >= METER_FLOOR_DB && stored <= 0) return stored;
  } catch {
    // Storage unavailable: the default it is.
  }
  return DEFAULT_SENSITIVITY_DB;
}

export const prefs = $state({ sensitivityDb: loadSensitivity() });

export function setSensitivity(db: number) {
  prefs.sensitivityDb = db;
  try {
    localStorage.setItem(SENSITIVITY_KEY, String(db));
  } catch {
    // Not remembered, but still applied for this session.
  }
}

/** Fixed threshold for other participants. Their level is measured after
 *  decoding, so it is cleaner than a raw mic and needs no tuning. */
export const REMOTE_THRESHOLD = 0.03;
