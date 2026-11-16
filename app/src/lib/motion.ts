import type { TransitionConfig } from "svelte/transition";

// The app's motion, after the transitions.dev library (https://transitions.dev):
// its modal, dropdown, toast and page timings and its one easing curve, as
// Svelte transitions so they run when a block mounts and unmounts. Arriving
// runs on the slower clock and leaving on the faster one, so opening feels
// deliberate and closing snappy.
//
// They move with the individual `scale` property and with `transform`, so an
// element centred with `translate` keeps its place while it animates.

/** cubic-bezier(0.22, 1, 0.36, 1), the curve in app.css as --ease. */
function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const at = (a: number, b: number, t: number) => ((1 - 3 * b + 3 * a) * t + (3 * b - 6 * a)) * t * t + 3 * a * t;
  const slope = (a: number, b: number, t: number) => 3 * (1 - 3 * b + 3 * a) * t * t + 2 * (3 * b - 6 * a) * t + 3 * a;
  return (x) => {
    if (x <= 0 || x >= 1) return x;
    let t = x;
    for (let i = 0; i < 8; i++) {
      const d = slope(x1, x2, t);
      if (Math.abs(d) < 1e-6) break;
      t -= (at(x1, x2, t) - x) / d;
    }
    return at(y1, y2, Math.min(1, Math.max(0, t)));
  };
}

export const ease = bezier(0.22, 1, 0.36, 1);

/** Whether the OS asks for less motion; the CSS guard in app.css does not
 *  reach these, which run as Web Animations. */
const still = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

type Direction = { direction?: "in" | "out" | "both" };

const leaving = (options?: Direction) => options?.direction === "out";

/** A dialog over the page: up from 0.96, and down to it on close. */
export function modal(_node: Element, params: { duration?: number } = {}, options?: Direction): TransitionConfig {
  const out = leaving(options);
  return {
    duration: still() ? 0 : (params.duration ?? (out ? 150 : 250)),
    easing: ease,
    css: (t) => `opacity: ${t}; scale: ${0.96 + 0.04 * t};`,
  };
}

/** The night thickening behind a dialog. */
export function scrim(_node: Element, params: { duration?: number } = {}, options?: Direction): TransitionConfig {
  const out = leaving(options);
  return {
    duration: still() ? 0 : (params.duration ?? (out ? 150 : 200)),
    easing: ease,
    css: (t) => `opacity: ${t};`,
  };
}

/** A menu or list growing from what opened it; set transform-origin on the
 *  element to the trigger's side. */
export function dropdown(_node: Element, params: { duration?: number } = {}, options?: Direction): TransitionConfig {
  const out = leaving(options);
  const from = out ? 0.99 : 0.97;
  return {
    duration: still() ? 0 : (params.duration ?? (out ? 150 : 250)),
    easing: ease,
    css: (t) => `opacity: ${t}; scale: ${from + (1 - from) * t};`,
  };
}

/** Something announcing itself: rises `y` pixels into place with a slight
 *  scale and a cross-blur. A negative `y` drops in from above. */
export function rise(
  _node: Element,
  params: { y?: number; blur?: number; scale?: number; duration?: number } = {},
  options?: Direction,
): TransitionConfig {
  const out = leaving(options);
  const { y = 16, blur = 2, scale = 0.97 } = params;
  return {
    duration: still() ? 0 : (params.duration ?? (out ? 250 : 350)),
    easing: ease,
    css: (t, u) =>
      `opacity: ${t}; transform: translateY(${u * y}px); scale: ${scale + (1 - scale) * t};` +
      (blur ? ` filter: blur(${u * blur}px);` : ""),
  };
}

/** A view coming in: a short fade and an 8px rise, no blur, since a whole
 *  page blurring stutters. */
export function page(_node: Element, params: { duration?: number } = {}): TransitionConfig {
  return {
    duration: still() ? 0 : (params.duration ?? 250),
    easing: ease,
    css: (t, u) => `opacity: ${t}; transform: translateY(${u * 8}px);`,
  };
}

/** A count arriving on an icon: grows from small with a cross-blur. */
export function badge(_node: Element, params: { duration?: number } = {}, options?: Direction): TransitionConfig {
  const out = leaving(options);
  return {
    duration: still() ? 0 : (params.duration ?? (out ? 180 : 260)),
    easing: ease,
    css: (t, u) => `opacity: ${t}; scale: ${0.4 + 0.6 * t}; filter: blur(${u * 2}px);`,
  };
}
