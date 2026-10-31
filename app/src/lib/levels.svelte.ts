// Voice levels, pushed by the backend ten times a second. Kept apart from the
// snapshot so the speaking indicators update without redrawing anything else.

import type { Levels } from "./types";

export const voice = $state<{ levels: Levels }>({ levels: {} });
