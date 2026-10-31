<script>
  import { onMount } from "svelte";
  import { onFrames } from "./api.js";
  import { I420Renderer } from "./renderer.js";

  // Draws one participant's screen share. Frames arrive over the socket as
  // fast as they decode; drawing happens at most once per display refresh.
  let { id, label } = $props();

  let canvas;
  let failed = $state(null);
  let waiting = $state(true);

  onMount(() => {
    let renderer;
    try {
      renderer = new I420Renderer(canvas);
    } catch (e) {
      failed = String(e);
      return;
    }
    let pending = null;
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
  .screen {
    position: relative;
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    min-height: 0;
  }

  canvas {
    max-width: 100%;
    max-height: 100%;
    width: auto;
    height: auto;
    object-fit: contain;
    border-radius: 8px;
    background: #000;
  }

  .overlay {
    position: absolute;
    margin: 0;
    color: var(--mist);
  }
</style>
