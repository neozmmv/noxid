<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api";
  import Select from "./Select.svelte";
  import type { DisplayInfo, Quality, Snapshot } from "./types";

  // Choosing what to share: a whole screen, or one window, each shown as a
  // still of what it looks like right now. The bitrate is chosen here too,
  // since this is where a share is set up (and changed).
  interface Props {
    snapshot: Snapshot;
    targets: DisplayInfo[];
    onpick: (id: string) => void;
    onclose: () => void;
  }

  let { snapshot, targets, onpick, onclose }: Props = $props();

  // Bitrates on offer for the top quality, in kbit/s, around its default.
  // A choice made earlier stays listed even if it is not one of these.
  const BITRATES: Record<Quality, number[]> = {
    low: [300, 400, 800, 1000, 1500],
    mid: [1500, 2000, 3000, 4000, 6000],
    high: [3000, 4000, 6000, 8000, 10000, 12000],
  };
  const bitrateChoices = $derived.by(() => {
    const list = [...BITRATES[snapshot.max_quality]];
    const chosen = snapshot.bitrate === null ? null : snapshot.bitrate / 1000;
    if (chosen !== null && !list.includes(chosen)) list.push(chosen);
    return list.sort((a, b) => a - b);
  });
  const kbps = (bits: number) => `${Math.round(bits / 1000).toLocaleString("en-US")} kbps`;

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
        <p>
          {#if snapshot.screen_audio && snapshot.window_audio_supported}
            Only that window and its app's sound are sent, even when other windows cover it.
          {:else}
            Only that window is sent, even when other windows cover it.
          {/if}
        </p>
        <div class="grid">
          {#each windows as t (t.id)}
            {@render card(t, t.name, t.app)}
          {/each}
        </div>
      </section>
    {/if}
  </div>

  <div class="actions">
    <label class="bitrate">
      <span>Bitrate</span>
      <Select
        compact
        value={snapshot.bitrate === null ? "auto" : String(snapshot.bitrate / 1000)}
        onchange={(e) => {
          const v = e.currentTarget.value;
          api.setBitrate(v === "auto" ? null : Number(v));
        }}
      >
        <option value="auto">Default ({kbps(snapshot.default_bitrate)})</option>
        {#each bitrateChoices as choice (choice)}
          <option value={String(choice)}>{kbps(choice * 1000)}</option>
        {/each}
      </Select>
    </label>
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
    aspect-ratio: 16 / 10;
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
    align-items: center;
    gap: 0.6rem;
  }

  .bitrate {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-right: auto;
    color: var(--mist);
  }

  .bitrate :global(select.field) {
    background-color: var(--dusk-raised);
  }
</style>
