<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  // A modal: the night thickened behind it, Esc or a click outside to close,
  // and focus placed inside so keyboard users land in it.
  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
  }

  let { title, onclose, children, actions }: Props = $props();

  let dialog: HTMLElement;
  const titleId = `dialog-${Math.random().toString(36).slice(2)}`;

  onMount(() => {
    const first = dialog.querySelector<HTMLElement>("input, textarea, button");
    (first ?? dialog).focus();
  });

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.stopImmediatePropagation();
      onclose();
    }
  }
</script>

<svelte:window onkeydowncapture={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabindex="-1" bind:this={dialog}>
  <h2 id={titleId}>{title}</h2>
  {@render children()}
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .dialog {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 24px;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    outline: none;
    display: grid;
    gap: 16px;
  }

  h2 {
    margin: 0;
    font-size: var(--step-1);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
