<script lang="ts">
  import type { Snippet } from "svelte";
  import { tooltip } from "./tooltip";

  // One icon in the navigation rail. `active` marks the view on screen;
  // `disabled` is for what is not built yet, which must never look like it
  // does something.
  interface Props {
    label: string;
    active?: boolean;
    disabled?: boolean;
    /** A small gold dot, for "something is going on here". */
    dot?: boolean;
    badge?: number;
    onclick?: () => void;
    children: Snippet;
  }

  let { label, active = false, disabled = false, dot = false, badge = 0, onclick, children }: Props = $props();

  const spoken = $derived(badge > 0 ? `${label}, ${badge} new` : label);
</script>

<button
  class="rail-button"
  class:active
  aria-label={spoken}
  aria-current={active ? "page" : undefined}
  aria-disabled={disabled || undefined}
  use:tooltip={{ label, side: "right" }}
  onclick={() => !disabled && onclick?.()}
>
  {@render children()}
  {#if badge > 0}
    <span class="badge count" aria-hidden="true">{badge > 99 ? "99+" : badge}</span>
  {:else if dot}
    <span class="dot" aria-hidden="true"></span>
  {/if}
</button>

<style>
  .rail-button {
    position: relative;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    padding: 0;
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--mist);
  }

  .rail-button:hover:not([aria-disabled]) {
    background: color-mix(in srgb, var(--dusk-raised) 60%, transparent);
    color: var(--moon);
  }

  .rail-button.active {
    background: var(--dusk-raised);
    color: var(--lamp);
  }

  /* The pill on the rail's edge beside the view on screen. */
  .rail-button.active::before {
    content: "";
    position: absolute;
    left: -12px;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--lamp);
  }

  .rail-button[aria-disabled] {
    opacity: 0.45;
    cursor: default;
  }

  .count {
    position: absolute;
    top: 2px;
    right: 0;
  }

  .dot {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--lamp);
    box-shadow: 0 0 0 2px var(--ink);
  }
</style>
