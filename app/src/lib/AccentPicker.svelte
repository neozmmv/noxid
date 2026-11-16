<script lang="ts">
  import Check from "@lucide/svelte/icons/check";
  import { ACCENTS, appearance, setAccent, type Accent } from "./theme.svelte";
  import { tooltip } from "./tooltip";

  // The accents as a row of round swatches, each filled with its colour and
  // named in a tooltip. A radio group like the themes above it: the arrow
  // keys move the choice and Tab leaves.
  let { labelledby }: { labelledby?: string } = $props();

  let group: HTMLElement;

  function keydown(event: KeyboardEvent) {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
    if (!step) return;
    event.preventDefault();
    const at = ACCENTS.findIndex((a) => a.value === appearance.accent);
    const next = ACCENTS[(at + step + ACCENTS.length) % ACCENTS.length]!.value;
    setAccent(next);
    group.querySelector<HTMLElement>(`[data-value="${next}"]`)?.focus();
  }

  function pick(accent: Accent) {
    setAccent(accent);
  }
</script>

<div class="accents" role="radiogroup" aria-labelledby={labelledby} tabindex="-1" bind:this={group} onkeydown={keydown}>
  {#each ACCENTS as accent (accent.value)}
    {@const chosen = accent.value === appearance.accent}
    <!-- data-accent scopes that accent's colour to the swatch, as data-theme
         does for the theme previews. -->
    <button
      class="swatch"
      class:chosen
      role="radio"
      aria-checked={chosen}
      aria-label={accent.label}
      tabindex={chosen ? 0 : -1}
      data-value={accent.value}
      data-accent={accent.value}
      use:tooltip={accent.label}
      onclick={() => pick(accent.value)}
    >
      {#if chosen}<Check size={16} strokeWidth={2.5} aria-hidden="true" />{/if}
    </button>
  {/each}
</div>

<style>
  .accents {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    outline: none;
  }

  .swatch {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--lamp);
    color: var(--ink);
    /* A ring set off by a gap of the card's colour, shown when chosen. */
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 4px transparent;
    transition:
      box-shadow var(--dur-quick) var(--ease),
      scale var(--dur-quick) var(--ease);
  }

  .swatch:hover {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 4px var(--line);
  }

  .swatch.chosen {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 4px var(--lamp);
  }

  .swatch:active {
    scale: 0.94;
  }
</style>
