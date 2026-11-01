<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api";
  import type { DisplayInfo } from "./types";

  // Choosing what to share: a whole screen, or one window, each shown as a
  // still of what it looks like right now.
  interface Props {
    targets: DisplayInfo[];
    onpick: (id: string) => void;
    onclose: () => void;
  }

  let { targets, onpick, onclose }: Props = $props();

  const screens = $derived(targets.filter((t) => t.kind === "display"));
  const windows = $derived(targets.filter((t) => t.kind === "window"));

  // Stills, by target id; `null` once a still turned out to be unavailable.
  let stills = $state<Record<string, ImageData | null>>({});

  onMount(() => {
    let open = true;
    // All at once: each is its own short capture, and the backend runs them
    // side by side.
    for (const t of targets) {
      api
        .sharePreview(t.id)
        .then((buffer) => {
          const view = new DataView(buffer);
          const width = view.getUint32(0, true);
          const height = view.getUint32(4, true);
          const pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
          if (open) stills[t.id] = new ImageData(pixels, width, height);
        })
        .catch(() => {
          if (open) stills[t.id] = null;
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

{#snippet card(t: DisplayInfo, name: string, meta: string)}
  <button class="target" onclick={() => onpick(t.id)} title={name}>
    <span class="still">
      {#if stills[t.id]}
        <canvas use:still={stills[t.id]!}></canvas>
      {:else if stills[t.id] === null}
        <span class="note">No preview</span>
      {:else}
        <span class="note loading">Loading…</span>
      {/if}
    </span>
    <span class="name">{name}</span>
    <span class="meta">{meta}</span>
  </button>
{/snippet}

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="picker" role="dialog" aria-modal="true" aria-label="Share your screen" tabindex="-1" bind:this={dialog}>
  <h2>Share</h2>

  <div class="scroll">
    <section aria-label="Screens">
      <h3>Whole screen</h3>
      <div class="grid">
        {#each screens as t, i (t.id)}
          {@render card(t, screens.length > 1 ? `Screen ${i + 1}` : "Your screen", `${t.width}×${t.height}`)}
        {/each}
      </div>
    </section>

    {#if windows.length}
      <section aria-label="Windows">
        <h3>One window</h3>
        <p>Only that window is sent, even when other windows cover it.</p>
        <div class="grid">
          {#each windows as t (t.id)}
            {@render card(t, t.name, t.app)}
          {/each}
        </div>
      </section>
    {/if}
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
    background: rgb(10 12 28 / 0.55);
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
    border: 1px solid var(--dusk-raised);
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    gap: 1rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
  }

  h3 {
    margin: 0 0 0.6rem;
    font-size: var(--step-0);
    color: var(--mist);
    font-weight: 600;
  }

  .scroll {
    min-height: 0;
    overflow-y: auto;
    display: grid;
    gap: 1.25rem;
    padding-right: 0.25rem;
  }

  p {
    margin: -0.35rem 0 0.7rem;
    color: var(--mist);
    font-size: var(--step--1);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 0.75rem;
  }

  .target {
    display: grid;
    gap: 0.2rem;
    min-width: 0;
    padding: 0.5rem 0.5rem 0.6rem;
    border: 1px solid var(--dusk-raised);
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
    aspect-ratio: 16 / 10;
    margin-bottom: 0.3rem;
    overflow: hidden;
    border-radius: 8px;
    background: #0d1024;
  }

  .still canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .note {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .loading {
    opacity: 0.6;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }

  .meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--mist);
    font-size: var(--step--1);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
