<script lang="ts">
  import { onMount } from "svelte";
  import { onFrames } from "./api";
  import { I420Renderer } from "./renderer";
  import type { Frame } from "./types";

  // Draws one participant's screen share. Frames arrive over the socket as
  // fast as they decode; drawing happens at most once per display refresh.
  let { id, label }: { id: string; label: string } = $props();

  let canvas: HTMLCanvasElement;
  let failed = $state<string | null>(null);
  let waiting = $state(true);

  onMount(() => {
    let renderer: I420Renderer;
    try {
      renderer = new I420Renderer(canvas);
    } catch (e) {
      failed = String(e);
      return;
    }
    let pending: Frame | null = null;
    let scheduled = 0;
    const draw = () => {
      scheduled = 0;
      if (pending) {
        renderer.draw(pending);
        pending = null;
        waiting = false;
      }
    };
    const stop = onFrames(id, (frame) => {
      pending = frame;
      if (!scheduled) scheduled = requestAnimationFrame(draw);
    });
    return () => {
      stop();
      cancelAnimationFrame(scheduled);
      renderer.dispose();
    };
  });
</script>

<div class="screen">
  <canvas bind:this={canvas} aria-label={label}></canvas>
  {#if failed}
    <p class="overlay">This screen can't be shown: {failed}</p>
  {:else if waiting}
    <p class="overlay">Waiting for the picture…</p>
  {/if}
</div>

<style>
  /* The canvas is taken out of layout and letterboxed with object-fit, so a
     1080p picture can never push the box it sits in (and everything after
     it) out of the window. The box's size comes from the layout alone. */
  .screen {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    display: grid;
    place-items: center;
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .overlay {
    position: relative;
    margin: 0;
    padding: 0 1rem;
    color: var(--mist);
    text-align: center;
  }
</style>
