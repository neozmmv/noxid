<script lang="ts">
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import type { HTMLSelectAttributes } from "svelte/elements";

  // A select with the app's own arrow. The native one ignores the rounded
  // corners and ends up pressed against the edge, so it is hidden and an icon
  // sits over the right end instead (clicks pass through it to the select).
  // `compact` is the pill-shaped version used in toolbars.
  let {
    compact = false,
    class: className = "",
    children,
    ...rest
  }: HTMLSelectAttributes & { compact?: boolean } = $props();
</script>

<span class="select" class:compact>
  <select class={["field", className]} {...rest}>
    {@render children?.()}
  </select>
  <ChevronDown class="arrow" size={16} aria-hidden="true" />
</span>

<style>
  .select {
    position: relative;
    display: inline-grid;
    align-items: center;
  }

  select {
    appearance: none;
    padding-right: 2.4rem;
  }

  .compact select {
    width: auto;
    padding: 0.5rem 2.2rem 0.5rem 0.9rem;
    border-radius: 999px;
  }

  .select :global(.arrow) {
    position: absolute;
    right: 0.8rem;
    color: var(--mist);
    pointer-events: none;
  }

  select:disabled + :global(.arrow) {
    opacity: 0.5;
  }
</style>
