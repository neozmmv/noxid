<script lang="ts">
  import { onMount } from "svelte";
  import type { DisplayInfo } from "./types";

  // Choosing what to share: a whole screen, or one window.
  interface Props {
    targets: DisplayInfo[];
    onpick: (id: string) => void;
    onclose: () => void;
  }

  let { targets, onpick, onclose }: Props = $props();

  const screens = $derived(targets.filter((t) => t.kind === "display"));
  const windows = $derived(targets.filter((t) => t.kind === "window"));

  let dialog: HTMLElement;
  onMount(() => dialog.querySelector<HTMLElement>("button.target")?.focus());

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="picker" role="dialog" aria-modal="true" aria-label="Share your screen" tabindex="-1" bind:this={dialog}>
  <h2>Share</h2>

  <section aria-label="Screens">
    <h3>Whole screen</h3>
    <div class="list">
      {#each screens as t, i (t.id)}
        <button class="target" onclick={() => onpick(t.id)}>
          <span class="name">{screens.length > 1 ? `Screen ${i + 1}` : "Your screen"}</span>
          <span class="meta">{t.width}×{t.height}</span>
        </button>
      {/each}
    </div>
  </section>

  {#if windows.length}
    <section aria-label="Windows">
      <h3>One window</h3>
      <p>Only that window is sent, even when other windows cover it.</p>
      <div class="list windows">
        {#each windows as t (t.id)}
          <button class="target" onclick={() => onpick(t.id)} title={t.name}>
            <span class="name">{t.name}</span>
            <span class="meta">{t.app}</span>
          </button>
        {/each}
      </div>
    </section>
  {/if}

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
    width: min(520px, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--dusk-raised);
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    gap: 1rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
  }

  h3 {
    margin: 0 0 0.5rem;
    font-size: var(--step-0);
    color: var(--mist);
    font-weight: 600;
  }

  section {
    min-height: 0;
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr);
  }

  p {
    margin: -0.3rem 0 0.6rem;
    color: var(--mist);
    font-size: var(--step--1);
  }

  .list {
    display: grid;
    gap: 0.35rem;
  }

  .windows {
    overflow-y: auto;
    padding-right: 0.25rem;
  }

  .target {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: baseline;
    gap: 1rem;
    padding: 0.65rem 0.85rem;
    border: 1px solid var(--dusk-raised);
    border-radius: 10px;
    background: var(--ink);
    text-align: left;
  }

  .target:hover {
    border-color: var(--lamp);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }

  .meta {
    color: var(--mist);
    font-size: var(--step--1);
    white-space: nowrap;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
