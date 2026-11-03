<script lang="ts">
  import ScreenView from "./ScreenView.svelte";

  // A camera floating over a screen share: dragged anywhere inside the
  // picture, resized from its corner. Position and size are kept as fractions
  // of the picture, so it stays put when the stage grows (fullscreen) or
  // shrinks. With the keyboard: arrows move it, + and - resize it.
  let { id, label }: { id: string; label: string } = $props();

  /** The camera box is 16:9; a camera of another shape is letterboxed. */
  const ASPECT = 16 / 9;
  /** Width limits, as fractions of the picture's width. */
  const MIN_WIDTH = 0.12;
  const MAX_WIDTH = 0.6;
  const STEP = 0.02;

  let box: HTMLElement;
  // Top-left corner and width, as fractions of the picture. Starts in the
  // bottom-right corner, out of the way of most content.
  let x = $state(0.75);
  let y = $state(0.7);
  let width = $state(0.22);

  let area = $state({ width: 0, height: 0 });

  // The picture the camera floats in; watched so fractions turn into pixels.
  function track(node: HTMLElement) {
    const parent = node.parentElement!;
    const observer = new ResizeObserver(() => {
      area = { width: parent.clientWidth, height: parent.clientHeight };
      clamp();
    });
    observer.observe(parent);
    return { destroy: () => observer.disconnect() };
  }

  const heightFraction = $derived(area.height ? (width * area.width) / ASPECT / area.height : 0);

  /** Keeps the box whole and inside the picture. */
  function clamp() {
    if (!area.width || !area.height) return;
    // Never taller than the picture, whatever the width limit says.
    const tallest = (area.height * ASPECT) / area.width;
    width = Math.min(Math.max(width, MIN_WIDTH), MAX_WIDTH, tallest);
    const h = (width * area.width) / ASPECT / area.height;
    x = Math.min(Math.max(x, 0), 1 - width);
    y = Math.min(Math.max(y, 0), 1 - h);
  }

  // One gesture at a time: moving the box, or resizing it from the corner.
  let gesture = $state<{
    kind: "move" | "resize";
    startX: number;
    startY: number;
    x: number;
    y: number;
    width: number;
  } | null>(null);

  function begin(kind: "move" | "resize", event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    box.setPointerCapture(event.pointerId);
    gesture = { kind, startX: event.clientX, startY: event.clientY, x, y, width };
  }

  function pointermove(event: PointerEvent) {
    if (!gesture || !area.width) return;
    const dx = (event.clientX - gesture.startX) / area.width;
    const dy = (event.clientY - gesture.startY) / area.height;
    if (gesture.kind === "move") {
      x = gesture.x + dx;
      y = gesture.y + dy;
    } else {
      // The top-left corner stays; the corner under the pointer follows it.
      width = gesture.width + dx;
    }
    clamp();
  }

  function end(event: PointerEvent) {
    if (!gesture) return;
    gesture = null;
    if (box.hasPointerCapture(event.pointerId)) box.releasePointerCapture(event.pointerId);
  }

  function keydown(event: KeyboardEvent) {
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-STEP, 0],
      ArrowRight: [STEP, 0],
      ArrowUp: [0, -STEP],
      ArrowDown: [0, STEP],
    };
    const move = moves[event.key];
    if (move) {
      x += move[0];
      y += move[1];
    } else if (event.key === "+" || event.key === "=") {
      width += STEP;
    } else if (event.key === "-") {
      width -= STEP;
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    clamp();
  }
</script>

<!-- Dragging is the pointer's way of moving it; the keyboard has the arrows,
     announced in the label. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="floating"
  class:dragging={gesture?.kind === "move"}
  role="group"
  aria-label={`${label}. Arrow keys move it, plus and minus resize it.`}
  tabindex="0"
  bind:this={box}
  use:track
  style={`left: ${x * 100}%; top: ${y * 100}%; width: ${width * 100}%; height: ${heightFraction * 100}%`}
  onpointerdown={(e) => begin("move", e)}
  onpointermove={pointermove}
  onpointerup={end}
  onpointercancel={end}
  ondblclick={(e) => e.stopPropagation()}
  onkeydown={keydown}
>
  <ScreenView {id} {label} feed="camera" />
  <span
    class="handle"
    aria-hidden="true"
    onpointerdown={(e) => begin("resize", e)}
  ></span>
</div>

<style>
  .floating {
    position: absolute;
    z-index: 2;
    overflow: hidden;
    border-radius: 10px;
    background: #000;
    box-shadow:
      0 0 0 1px rgb(255 255 255 / 0.12),
      0 10px 28px rgb(0 0 0 / 0.5);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }

  .floating.dragging {
    cursor: grabbing;
  }

  /* The corner to resize from: a quarter circle that shows on hover, so the
     camera is not framed by a control while nobody is touching it. */
  .handle {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 22px;
    height: 22px;
    border-top-left-radius: 22px;
    background: rgb(243 241 250 / 0.85);
    cursor: nwse-resize;
    opacity: 0;
    transition: opacity 120ms ease;
  }

  .floating:hover .handle,
  .floating:focus-visible .handle {
    opacity: 1;
  }
</style>
