<script lang="ts">
  import Check from "@lucide/svelte/icons/check";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import { STATUSES, setStatus, statusLabel } from "./status";
  import StatusDot from "./StatusDot.svelte";
  import type { Status } from "./types";

  // Choosing the status friends see: a field showing the current one, which
  // opens the list of the others. The list opens upward when there is no
  // room below, as at the foot of the rail.
  interface Props {
    status: Status;
    labelledby?: string;
  }

  let { status, labelledby }: Props = $props();
  const uid = $props.id();

  let root: HTMLElement;
  let trigger: HTMLButtonElement;
  let list = $state<HTMLElement>();
  let open = $state(false);
  let upward = $state(false);

  /** Roughly the list's height, to decide which way it opens. */
  const LIST_HEIGHT = 240;

  function toggle() {
    if (open) {
      open = false;
      return;
    }
    const rect = trigger.getBoundingClientRect();
    upward = window.innerHeight - rect.bottom < LIST_HEIGHT && rect.top > LIST_HEIGHT;
    open = true;
  }

  // Focus the chosen option once the list is there, for the arrow keys.
  $effect(() => {
    if (open && list) list.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  });

  function pick(value: Status) {
    open = false;
    trigger.focus();
    if (value !== status) void setStatus(value);
  }

  function listKey(event: KeyboardEvent) {
    const options = [...(list?.querySelectorAll<HTMLElement>('[role="option"]') ?? [])];
    const at = options.indexOf(document.activeElement as HTMLElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const step = event.key === "ArrowDown" ? 1 : -1;
      options[(at + step + options.length) % options.length]?.focus();
    } else if (event.key === "Home") {
      options[0]?.focus();
    } else if (event.key === "End") {
      options.at(-1)?.focus();
    } else if (event.key === "Tab") {
      open = false;
      return;
    } else {
      return;
    }
    event.preventDefault();
  }

  function triggerKey(event: KeyboardEvent) {
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !open) {
      event.preventDefault();
      toggle();
    }
  }

  function outside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="picker" bind:this={root}>
  <button
    class="field trigger"
    bind:this={trigger}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-labelledby={labelledby ? `${labelledby} ${uid}-current` : undefined}
    onclick={toggle}
    onkeydown={triggerKey}
  >
    <StatusDot {status} />
    <span id={`${uid}-current`} class="current">{statusLabel(status)}</span>
    <ChevronDown class={["arrow", open && "turned"]} size={16} aria-hidden="true" />
  </button>

  {#if open}
    <div
      class="list"
      class:upward
      role="listbox"
      tabindex="-1"
      aria-labelledby={labelledby}
      bind:this={list}
      onkeydown={listKey}
    >
      {#each STATUSES as option (option.value)}
        <button
          class="option"
          role="option"
          aria-selected={option.value === status}
          onclick={() => pick(option.value)}
        >
          <StatusDot status={option.value} />
          <span class="text">
            <strong>{option.label}</strong>
            <small>{option.detail}</small>
          </span>
          {#if option.value === status}
            <Check class="check" size={16} aria-hidden="true" />
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }

  .trigger {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--moon);
    text-align: left;
    cursor: pointer;
  }

  .trigger:hover {
    border-color: var(--mist);
  }

  .current {
    flex: 1;
  }

  .trigger :global(.arrow) {
    color: var(--mist);
    transition: transform 150ms ease;
  }

  .trigger :global(.arrow.turned) {
    transform: rotate(180deg);
  }

  .list {
    position: absolute;
    z-index: 5;
    left: 0;
    right: 0;
    top: calc(100% + 4px);
    min-width: 240px;
    padding: 4px;
    display: grid;
    gap: 2px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--dusk);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
  }

  .list.upward {
    top: auto;
    bottom: calc(100% + 4px);
  }

  .option {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--moon);
    text-align: left;
    cursor: pointer;
  }

  .option:hover,
  .option:focus-visible {
    background: var(--dusk-raised);
    outline: none;
  }

  .text {
    flex: 1;
    display: grid;
    gap: 1px;
  }

  .text strong {
    font-weight: 600;
  }

  .text small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .option :global(.check) {
    color: var(--lamp);
  }
</style>
