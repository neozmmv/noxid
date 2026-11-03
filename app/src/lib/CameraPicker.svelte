<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api";
  import type { CameraInfo } from "./types";

  // Choosing a camera to turn on, each shown as a picture from it right now,
  // the way the share picker shows screens and windows.
  interface Props {
    cameras: CameraInfo[];
    onpick: (id: string) => void;
    onclose: () => void;
  }

  let { cameras, onpick, onclose }: Props = $props();

  // Stills, by camera id; `null` once a still turned out to be unavailable
  // (a camera in use by another app, or a virtual one that is not running).
  let stills = $state<Record<string, ImageData | null>>({});

  onMount(() => {
    let open = true;
    // Asked for together; the backend opens the cameras one after another.
    for (const camera of cameras) {
      api
        .cameraPreview(camera.id)
        .then((buffer) => {
          const view = new DataView(buffer);
          const width = view.getUint32(0, true);
          const height = view.getUint32(4, true);
          const pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
          if (open) stills[camera.id] = new ImageData(pixels, width, height);
        })
        .catch(() => {
          if (open) stills[camera.id] = null;
        });
    }
    return () => {
      open = false;
    };
  });

  // Paints a still into its canvas once it arrives.
  function still(canvas: HTMLCanvasElement, image: ImageData) {
    const paint = (image: ImageData) => {
      canvas.width = image.width;
      canvas.height = image.height;
      canvas.getContext("2d")?.putImageData(image, 0, 0);
    };
    paint(image);
    return { update: paint };
  }

  let dialog: HTMLElement;
  onMount(() => dialog.querySelector<HTMLElement>("button.target")?.focus());

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="picker" role="dialog" aria-modal="true" aria-label="Choose a camera" tabindex="-1" bind:this={dialog}>
  <h2>Camera</h2>

  <div class="grid">
    {#each cameras as camera (camera.id)}
      <button class="target" onclick={() => onpick(camera.id)} title={camera.name}>
        <span class="still">
          {#if stills[camera.id]}
            <canvas use:still={stills[camera.id]!}></canvas>
          {:else if stills[camera.id] === null}
            <span class="note">No picture. It may be in use by another app.</span>
          {:else}
            <span class="note loading">Loading…</span>
          {/if}
        </span>
        <span class="name">{camera.name}</span>
      </button>
    {/each}
  </div>

  <div class="actions">
    <button class="button quiet" onclick={onclose}>Cancel</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .picker {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(760px, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    gap: 1rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
  }

  .grid {
    min-height: 0;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 0.75rem;
    padding-right: 0.25rem;
  }

  .target {
    display: grid;
    gap: 0.2rem;
    min-width: 0;
    padding: 0.5rem 0.5rem 0.6rem;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--ink);
    text-align: left;
  }

  .target:hover {
    border-color: var(--lamp);
  }

  .still {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    margin-bottom: 0.3rem;
    overflow: hidden;
    border-radius: 8px;
    background: var(--dusk);
  }

  .still canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .note {
    padding: 0 0.75rem;
    color: var(--mist);
    font-size: var(--step--1);
    text-align: center;
  }

  .loading {
    opacity: 0.6;
  }

  .name {
    overflow-wrap: anywhere;
    font-weight: 600;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
