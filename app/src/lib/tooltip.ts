// A label that rises above a control after the pointer rests on it. One
// element serves every control, so there is never more than one on screen.

export type TooltipOptions = string | { label: string; key?: string };

const DELAY = 450;
// Moving from one control to its neighbour within this window skips the
// delay, so scanning a row of icons reads them off one after another.
const WARM_WINDOW = 300;
const GAP = 8;
const EDGE = 8;

let tip: HTMLDivElement | null = null;
let owner: HTMLElement | null = null;
let showTimer: ReturnType<typeof setTimeout> | undefined;
let coolTimer: ReturnType<typeof setTimeout> | undefined;
let warm = false;

function element(): HTMLDivElement {
  if (tip) return tip;
  tip = document.createElement("div");
  tip.className = "tooltip";
  tip.setAttribute("aria-hidden", "true");
  document.body.append(tip);
  return tip;
}

function fill(el: HTMLDivElement, options: TooltipOptions) {
  const { label, key } = typeof options === "string" ? { label: options, key: undefined } : options;
  el.replaceChildren(document.createTextNode(label));
  if (key) {
    const kbd = document.createElement("kbd");
    kbd.textContent = key;
    el.append(kbd);
  }
}

function place(el: HTMLDivElement, anchor: HTMLElement) {
  const a = anchor.getBoundingClientRect();
  const t = el.getBoundingClientRect();
  const below = a.top - t.height - GAP < EDGE;
  const top = below ? a.bottom + GAP : a.top - t.height - GAP;
  const centre = a.left + a.width / 2;
  const left = Math.min(Math.max(centre - t.width / 2, EDGE), window.innerWidth - t.width - EDGE);
  el.style.top = `${top}px`;
  el.style.left = `${left}px`;
  // The notch keeps pointing at the control even when the label is pushed
  // in from a window edge.
  el.style.setProperty("--notch-x", `${centre - left}px`);
  el.dataset.side = below ? "below" : "above";
}

function show(anchor: HTMLElement, options: TooltipOptions) {
  clearTimeout(coolTimer);
  const el = element();
  owner = anchor;
  fill(el, options);
  el.classList.remove("shown");
  place(el, anchor);
  // Next frame, so the rise starts from its resting offset.
  requestAnimationFrame(() => owner === anchor && el.classList.add("shown"));
  warm = true;
}

function hide(anchor: HTMLElement) {
  clearTimeout(showTimer);
  if (owner !== anchor) return;
  owner = null;
  tip?.classList.remove("shown");
  coolTimer = setTimeout(() => (warm = false), WARM_WINDOW);
}

export function tooltip(node: HTMLElement, initial: TooltipOptions) {
  let options = initial;

  const schedule = () => {
    clearTimeout(showTimer);
    if (warm) show(node, options);
    else showTimer = setTimeout(() => show(node, options), DELAY);
  };
  const enter = (e: PointerEvent) => e.pointerType !== "touch" && schedule();
  const focus = () => node.matches(":focus-visible") && schedule();
  const leave = () => hide(node);
  const key = (e: KeyboardEvent) => e.key === "Escape" && hide(node);

  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", leave);
  node.addEventListener("pointerdown", leave);
  node.addEventListener("focus", focus);
  node.addEventListener("blur", leave);
  node.addEventListener("keydown", key);

  return {
    update(next: TooltipOptions) {
      options = next;
      // A toggle's label changes under the pointer; keep it truthful.
      if (owner === node && tip) {
        fill(tip, options);
        place(tip, node);
      }
    },
    destroy() {
      hide(node);
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", leave);
      node.removeEventListener("pointerdown", leave);
      node.removeEventListener("focus", focus);
      node.removeEventListener("blur", leave);
      node.removeEventListener("keydown", key);
    },
  };
}
