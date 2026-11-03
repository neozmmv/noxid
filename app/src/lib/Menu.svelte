<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  // A right-click menu: placed at the pointer but kept inside the window, and
  // closed by Esc, a click elsewhere, or the window losing focus. What goes in
  // it is the caller's; the item styles below are shared by every menu.
  interface Props {
    label: string;
    x: number;
    y: number;
    onclose: () => void;
    children: Snippet;
  }

  let { label, x, y, onclose, children }: Props = $props();

  let menu: HTMLElement;
  let left = $state(0);
  let top = $state(0);

  onMount(() => {
    const rect = menu.getBoundingClientRect();
    left = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    top = Math.max(8, Math.min(y, window.innerHeight - rect.height - 8));
    menu.querySelector<HTMLElement>("input, button")?.focus();
  });

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      // Esc closes the menu, not the fullscreen picture behind it.
      event.stopImmediatePropagation();
      onclose();
    }
  }

  function pointerdown(event: PointerEvent) {
    if (!menu.contains(event.target as Node)) onclose();
  }
</script>

<svelte:window onkeydowncapture={keydown} onpointerdown={pointerdown} onblur={onclose} />

<div class="menu" role="dialog" aria-label={label} bind:this={menu} style={`left: ${left}px; top: ${top}px`}>
  {@render children()}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    width: 260px;
    padding: 0.5rem;
    border-radius: 12px;
    background: var(--dusk);
    border: 1px solid var(--line);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
    display: grid;
    gap: 0.15rem;
  }

  .menu :global(.who) {
    margin: 0;
    padding: 0.35rem 0.5rem 0.5rem;
    font-weight: 650;
    overflow-wrap: anywhere;
  }

  .menu :global(.who span) {
    color: var(--mist);
    font-weight: 400;
  }

  .menu :global(.item),
  .menu :global(.row) {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.5rem;
    border: 0;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  .menu :global(.item:hover:not(:disabled)),
  .menu :global(.row:hover) {
    background: var(--dusk-raised);
  }

  .menu :global(.item:disabled) {
    opacity: 0.5;
    cursor: default;
  }

  .menu :global(.danger) {
    color: var(--signal);
  }

  .menu :global(.slider) {
    display: grid;
    gap: 0.35rem;
    padding: 0.35rem 0.5rem 0.6rem;
  }

  .menu :global(.slider span) {
    display: flex;
    justify-content: space-between;
    color: var(--mist);
    font-size: var(--step--1);
  }

  .menu :global(output) {
    color: var(--moon);
    font-variant-numeric: tabular-nums;
  }

  .menu :global(input[type="range"]) {
    width: 100%;
    accent-color: var(--lamp);
  }

  .menu :global(input[type="checkbox"]) {
    accent-color: var(--lamp);
    width: 1rem;
    height: 1rem;
  }

  .menu :global(hr) {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--line);
    margin: 0.25rem 0;
  }
</style>
