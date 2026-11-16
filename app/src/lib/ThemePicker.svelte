<script lang="ts">
  import { appearance, setTheme, THEMES, type Theme } from "./theme.svelte";

  // The themes as a row of small pictures of the app, each drawn in its own
  // colours: data-theme on the preview scopes that theme's palette to it.
  // A radio group, so the arrow keys move the choice and Tab leaves.
  let { labelledby }: { labelledby?: string } = $props();

  let group: HTMLElement;

  function keydown(event: KeyboardEvent) {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
    if (!step) return;
    event.preventDefault();
    const at = THEMES.findIndex((t) => t.value === appearance.theme);
    const next = THEMES[(at + step + THEMES.length) % THEMES.length]!.value;
    pick(next);
    group.querySelector<HTMLElement>(`[data-value="${next}"]`)?.focus();
  }

  function pick(theme: Theme) {
    setTheme(theme);
  }
</script>

<div class="themes" role="radiogroup" aria-labelledby={labelledby} tabindex="-1" bind:this={group} onkeydown={keydown}>
  {#each THEMES as theme (theme.value)}
    {@const chosen = theme.value === appearance.theme}
    <button
      class="theme"
      class:chosen
      role="radio"
      aria-checked={chosen}
      tabindex={chosen ? 0 : -1}
      data-value={theme.value}
      onclick={() => pick(theme.value)}
    >
      <span class="preview" data-theme={theme.value} aria-hidden="true">
        <span class="rail">
          <span class="dot"></span>
          <span class="dot"></span>
        </span>
        <span class="panel">
          <span class="bar"></span>
          <span class="bar short"></span>
          <span class="card"></span>
        </span>
      </span>
      <span class="name">{theme.label}</span>
    </button>
  {/each}
</div>

<style>
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 12px;
    outline: none;
  }

  .theme {
    display: grid;
    gap: 8px;
    padding: 8px 8px 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-card);
    background: var(--dusk);
    color: var(--mist);
    text-align: left;
  }

  .theme:hover {
    border-color: var(--mist);
    color: var(--moon);
  }

  .theme.chosen {
    border-color: var(--lamp);
    color: var(--moon);
  }

  .name {
    padding: 0 4px;
    font-size: var(--step--1);
    font-weight: 500;
  }

  /* The app in miniature: the rail, and a panel holding a title, a line of
     text and a card. Radius nests inside the button's: 12 − 8 = 4. */
  .preview {
    display: grid;
    grid-template-columns: 16px 1fr;
    gap: 4px;
    height: 72px;
    padding: 4px 4px 4px 0;
    border-radius: 4px;
    background: var(--ink);
    overflow: hidden;
  }

  .rail {
    display: grid;
    align-content: start;
    justify-items: center;
    gap: 4px;
    padding-top: 4px;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--dusk-raised);
  }

  .dot:first-child {
    background: var(--lamp);
  }

  .panel {
    display: grid;
    align-content: start;
    gap: 4px;
    padding: 8px;
    border-radius: 4px;
    background: var(--panel);
  }

  .bar {
    width: 60%;
    height: 6px;
    border-radius: 3px;
    background: var(--moon);
  }

  .bar.short {
    width: 40%;
    background: var(--mist);
    opacity: 0.6;
  }

  .card {
    height: 16px;
    margin-top: 2px;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--surface);
  }
</style>
