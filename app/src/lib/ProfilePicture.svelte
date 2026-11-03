<script lang="ts">
  import ImagePlus from "@lucide/svelte/icons/image-plus";
  import ZoomIn from "@lucide/svelte/icons/zoom-in";
  import ZoomOut from "@lucide/svelte/icons/zoom-out";
  import { onMount } from "svelte";
  import { api } from "./api";
  import { avatarUrl } from "./avatars.svelte";
  import type { Snapshot } from "./types";

  // Choosing a profile picture: pick an image, then move and zoom it until
  // the part you want sits inside the circle. What leaves is a small square
  // WebP of just that part, which is all an orb ever shows.
  interface Props {
    snapshot: Snapshot;
    onclose: () => void;
  }

  let { snapshot, onclose }: Props = $props();

  /** The stage's side, and the circle's inset in it, in CSS pixels. The
   *  circle is what gets saved; the margin shows a little of what is cut. */
  const VIEW = 260;
  const PAD = 12;
  const CROP = VIEW - 2 * PAD;
  /** The saved picture's side: twice the large orb, for sharp edges. */
  const OUT = 256;
  const MAX_ZOOM = 5;
  // Formats every webview here can decode. SVG is left out: it is a document
  // more than a picture, and a crop of it would be rasterized anyway.
  const ACCEPT = ["image/png", "image/jpeg", "image/webp", "image/gif", "image/avif", "image/bmp"];

  let dialog: HTMLElement;
  let input: HTMLInputElement;
  let stage = $state<HTMLElement>();
  let image = $state<HTMLImageElement | null>(null);
  let size = $state({ w: 0, h: 0 });
  let zoom = $state(1);
  // Where the picture's centre sits, from the circle's centre.
  let x = $state(0);
  let y = $state(0);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const current = $derived(avatarUrl(snapshot.avatar));
  // At zoom 1 the picture just covers the circle, whatever its shape.
  const cover = $derived(size.w ? Math.max(CROP / size.w, CROP / size.h) : 1);
  const scale = $derived(cover * zoom);

  /** Keeps the picture covering the whole circle: no empty corner can be
   *  dragged into it. */
  function clampOffset() {
    const maxX = (size.w * scale - CROP) / 2;
    const maxY = (size.h * scale - CROP) / 2;
    x = Math.max(-maxX, Math.min(maxX, x));
    y = Math.max(-maxY, Math.min(maxY, y));
  }

  /** Zooms to `next`, keeping the point at (`px`, `py`) from the centre where
   *  it is, so zooming with the wheel closes in on what is under the pointer. */
  function setZoom(next: number, px = 0, py = 0) {
    const before = scale;
    zoom = Math.max(1, Math.min(MAX_ZOOM, next));
    const ratio = cover * zoom / before;
    x = px - (px - x) * ratio;
    y = py - (py - y) * ratio;
    clampOffset();
  }

  async function pick() {
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    if (!ACCEPT.includes(file.type)) {
      error = "Choose a PNG, JPEG, WebP, GIF, AVIF or BMP picture.";
      return;
    }
    const url = URL.createObjectURL(file);
    const next = new Image();
    next.src = url;
    try {
      await next.decode();
    } catch {
      URL.revokeObjectURL(url);
      error = "That picture could not be opened.";
      return;
    }
    if (image) URL.revokeObjectURL(image.src);
    image = next;
    size = { w: next.naturalWidth, h: next.naturalHeight };
    zoom = 1;
    x = 0;
    y = 0;
    error = null;
  }

  let drag = $state<{ id: number; startX: number; startY: number; fromX: number; fromY: number } | null>(null);

  function pointerdown(event: PointerEvent) {
    if (!image || event.button !== 0) return;
    stage?.setPointerCapture(event.pointerId);
    drag = { id: event.pointerId, startX: event.clientX, startY: event.clientY, fromX: x, fromY: y };
  }

  function pointermove(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.id) return;
    x = drag.fromX + event.clientX - drag.startX;
    y = drag.fromY + event.clientY - drag.startY;
    clampOffset();
  }

  function pointerup(event: PointerEvent) {
    if (drag?.id === event.pointerId) drag = null;
  }

  // Not a template handler: it has to be able to stop the page scrolling.
  $effect(() => {
    const el = stage;
    if (!el) return;
    const wheel = (event: WheelEvent) => {
      if (!image) return;
      event.preventDefault();
      const rect = el.getBoundingClientRect();
      const px = event.clientX - rect.left - VIEW / 2;
      const py = event.clientY - rect.top - VIEW / 2;
      setZoom(zoom * Math.exp(-event.deltaY * 0.0015), px, py);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });

  function stageKey(event: KeyboardEvent) {
    if (!image) return;
    const step = event.shiftKey ? 32 : 8;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [step, 0],
      ArrowRight: [-step, 0],
      ArrowUp: [0, step],
      ArrowDown: [0, -step],
    };
    if (event.key in moves) {
      const [dx, dy] = moves[event.key]!;
      x += dx;
      y += dy;
      clampOffset();
    } else if (event.key === "+" || event.key === "=") {
      setZoom(zoom * 1.1);
    } else if (event.key === "-") {
      setZoom(zoom / 1.1);
    } else {
      return;
    }
    event.preventDefault();
  }

  async function save() {
    if (!image) return;
    busy = true;
    error = null;
    try {
      const canvas = document.createElement("canvas");
      canvas.width = OUT;
      canvas.height = OUT;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("no canvas");
      ctx.imageSmoothingQuality = "high";
      const r = OUT / CROP;
      const w = size.w * scale * r;
      const h = size.h * scale * r;
      ctx.drawImage(image, (CROP / 2 + x) * r - w / 2, (CROP / 2 + y) * r - h / 2, w, h);
      // WebP where the webview can write it; PNG otherwise, which is fine too.
      const blob = await new Promise<Blob | null>((done) => canvas.toBlob(done, "image/webp", 0.9));
      if (!blob) throw new Error("the picture could not be encoded");
      await api.setAvatar(new Uint8Array(await blob.arrayBuffer()));
      onclose();
    } catch (e) {
      error = `The picture could not be saved: ${e}`;
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      await api.clearAvatar();
      onclose();
    } catch (e) {
      error = `The picture could not be removed: ${e}`;
    } finally {
      busy = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }

  onMount(() => {
    dialog.focus();
    return () => {
      if (image) URL.revokeObjectURL(image.src);
    };
  });
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="picture" role="dialog" aria-modal="true" aria-label="Profile picture" tabindex="-1" bind:this={dialog}>
  <h2>Profile picture</h2>

  <input
    bind:this={input}
    class="visually-hidden"
    type="file"
    accept={ACCEPT.join(",")}
    tabindex="-1"
    aria-hidden="true"
    onchange={pick}
  />

  {#if image}
    <div
      class="stage"
      class:dragging={drag !== null}
      role="slider"
      tabindex="0"
      aria-label="Picture position. Arrow keys move it, plus and minus zoom."
      aria-valuenow={Math.round(zoom * 100)}
      aria-valuemin={100}
      aria-valuemax={MAX_ZOOM * 100}
      aria-valuetext={`${Math.round(zoom * 100)}% zoom`}
      bind:this={stage}
      onpointerdown={pointerdown}
      onpointermove={pointermove}
      onpointerup={pointerup}
      onpointercancel={pointerup}
      onkeydown={stageKey}
    >
      <img
        src={image.src}
        alt=""
        draggable="false"
        style:width={`${size.w * scale}px`}
        style:height={`${size.h * scale}px`}
        style:transform={`translate(calc(-50% + ${x}px), calc(-50% + ${y}px))`}
      />
    </div>

    <label class="zoom">
      <span class="visually-hidden">Zoom</span>
      <button class="step" aria-label="Zoom out" onclick={() => setZoom(zoom / 1.25)}>
        <ZoomOut size={18} aria-hidden="true" />
      </button>
      <input
        type="range"
        min="1"
        max={MAX_ZOOM}
        step="0.01"
        value={zoom}
        oninput={(e) => setZoom(Number(e.currentTarget.value))}
      />
      <button class="step" aria-label="Zoom in" onclick={() => setZoom(zoom * 1.25)}>
        <ZoomIn size={18} aria-hidden="true" />
      </button>
    </label>
    <p class="hint">Drag to move it, scroll or use the slider to zoom.</p>
  {:else}
    <button class="stage empty" onclick={() => input.click()}>
      {#if current}
        <img class="current" src={current} alt="Yours now" />
        <span class="change">Choose a new picture</span>
      {:else}
        <span class="placeholder">
          <ImagePlus size={28} aria-hidden="true" />
          Choose a picture
        </span>
      {/if}
    </button>
    <p class="hint">PNG, JPEG, WebP, GIF, AVIF or BMP. You'll crop it next.</p>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    {#if snapshot.avatar && !image}
      <button class="button quiet remove" onclick={remove} disabled={busy}>Remove picture</button>
    {/if}
    {#if image}
      <button class="button quiet" onclick={() => input.click()} disabled={busy}>Choose another</button>
    {/if}
    <span class="spacer"></span>
    <button class="button quiet" onclick={onclose}>Cancel</button>
    {#if image}
      <button class="button primary" onclick={save} disabled={busy}>{busy ? "Saving…" : "Save"}</button>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .picture {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(400px, calc(100vw - 2rem));
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    outline: none;
    display: grid;
    justify-items: center;
    gap: 0.9rem;
  }

  h2 {
    justify-self: start;
    margin: 0;
    font-size: var(--step-2);
  }

  .stage {
    position: relative;
    width: 260px;
    height: 260px;
    overflow: hidden;
    border-radius: 14px;
    background: var(--ink);
    touch-action: none;
    cursor: grab;
  }

  .stage.dragging {
    cursor: grabbing;
  }

  .stage:focus-visible {
    outline-offset: 3px;
  }

  .stage img {
    position: absolute;
    left: 50%;
    top: 50%;
    max-width: none;
    user-select: none;
    pointer-events: none;
  }

  /* Everything outside the circle is dimmed, and the circle carries the
     orb's own lamp ring, so you see the crop as others will. */
  .stage::after {
    content: "";
    position: absolute;
    inset: 12px;
    border-radius: 50%;
    box-shadow:
      0 0 0 2px var(--lamp),
      0 0 0 999px rgb(4 2 12 / 0.68);
    pointer-events: none;
  }

  .stage.empty {
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    cursor: pointer;
    color: var(--mist);
  }

  .stage.empty::after {
    box-shadow: none;
    border: 2px dashed var(--line);
    transition: border-color 150ms ease;
  }

  .stage.empty:hover::after {
    border-color: var(--lamp);
  }

  .stage .current {
    inset: 12px;
    left: 12px;
    top: 12px;
    width: calc(100% - 24px);
    height: calc(100% - 24px);
    border-radius: 50%;
    object-fit: cover;
  }

  .placeholder {
    display: grid;
    justify-items: center;
    gap: 0.5rem;
    font-weight: 600;
  }

  .change {
    position: absolute;
    bottom: 20px;
    padding: 0.3rem 0.7rem;
    border-radius: 999px;
    background: rgb(4 2 12 / 0.75);
    color: var(--moon);
    font-size: var(--step--1);
    font-weight: 600;
  }

  .zoom {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 260px;
  }

  .zoom input {
    flex: 1;
    accent-color: var(--lamp);
  }

  .step {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--mist);
  }

  .step:hover {
    background: var(--dusk-raised);
    color: var(--moon);
  }

  .hint {
    margin: 0;
    color: var(--mist);
    font-size: var(--step--1);
    text-align: center;
  }

  .error {
    margin: 0;
    color: var(--signal);
  }

  .actions {
    justify-self: stretch;
    display: flex;
    gap: 0.6rem;
  }

  .spacer {
    flex: 1;
  }

  .remove:hover:not(:disabled) {
    background: var(--signal);
    color: var(--ink);
  }
</style>
