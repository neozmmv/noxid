<script lang="ts">
  import Copy from "@lucide/svelte/icons/copy";
  import Check from "@lucide/svelte/icons/check";
  import Eye from "@lucide/svelte/icons/eye";
  import EyeOff from "@lucide/svelte/icons/eye-off";
  import { tooltip } from "./tooltip";

  // An endpoint id or friend code, shortened to its ends, with a copy button.
  // `masked` hides it until revealed: our friend code is also our room code,
  // so it is kept off the screen by default, as the room code is.
  interface Props {
    id: string;
    masked?: boolean;
    label?: string;
  }

  let { id, masked = false, label = "ID" }: Props = $props();

  let revealed = $state(false);
  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const short = $derived(id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-6)}` : id);
  const hidden = $derived(masked && !revealed);

  async function copy() {
    await navigator.clipboard.writeText(id);
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), 1500);
  }
</script>

<span class="chip">
  <!-- Both laid out, one over the other, so the chip keeps its size and the
       buttons their place whichever is showing. -->
  <code title={hidden ? undefined : id}>
    <span class="plain" class:gone={hidden} aria-hidden={hidden}>{short}</span>
    <span class="mask" class:gone={!hidden} aria-hidden="true">••••••••••••••</span>
    {#if hidden}<span class="visually-hidden">{label} hidden</span>{/if}
  </code>
  {#if masked}
    <button
      class="icon"
      aria-pressed={revealed}
      aria-label={revealed ? `Hide ${label}` : `Show ${label}`}
      use:tooltip={revealed ? "Hide" : "Show"}
      onclick={() => (revealed = !revealed)}
    >
      {#if revealed}<EyeOff size={14} aria-hidden="true" />{:else}<Eye size={14} aria-hidden="true" />{/if}
    </button>
  {/if}
  <button class="icon" aria-label={`Copy ${label}`} use:tooltip={copied ? "Copied" : "Copy"} onclick={copy}>
    {#if copied}<Check size={14} aria-hidden="true" />{:else}<Copy size={14} aria-hidden="true" />{/if}
  </button>
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    max-width: 100%;
    padding: 2px 2px 2px 8px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--ink);
  }

  /* Takes the chip's free width, so where the chip is stretched across a
     card the buttons still sit at its right edge. */
  code {
    flex: 1;
    min-width: 0;
    display: grid;
    margin-right: 4px;
    font-size: var(--step--1);
    color: var(--mist);
    white-space: nowrap;
  }

  .plain,
  .mask {
    grid-area: 1 / 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .gone {
    visibility: hidden;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--mist);
  }

  .icon:hover {
    background: var(--dusk-raised);
    color: var(--moon);
  }
</style>
