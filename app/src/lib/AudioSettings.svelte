<script lang="ts">
  import { onMount } from "svelte";
  import AudioSettingsPanel from "./AudioSettingsPanel.svelte";
  import type { Snapshot } from "./types";

  // The audio settings as a dialog, for the call, where leaving the room view
  // to reach Settings would be a step too far.
  let { snapshot, onclose }: { snapshot: Snapshot; onclose: () => void } = $props();

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }

  let dialog: HTMLElement;
  // The dialog itself takes focus, so keyboard users land inside it without
  // a ring appearing on the first control before they have pressed a key.
  onMount(() => dialog.focus());
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="settings" role="dialog" aria-modal="true" aria-label="Audio settings" tabindex="-1" bind:this={dialog}>
  <h2>Audio</h2>
  <AudioSettingsPanel {snapshot} />
  <div class="actions">
    <button class="button primary" onclick={onclose}>Done</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .settings {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 2rem));
    max-height: calc(100vh - 2rem);
    overflow: auto;
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    outline: none;
    display: grid;
    gap: 1rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }
</style>
