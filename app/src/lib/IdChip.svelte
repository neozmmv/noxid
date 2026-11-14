<script lang="ts">
  import Copy from "@lucide/svelte/icons/copy";
  import Check from "@lucide/svelte/icons/check";
  import Eye from "@lucide/svelte/icons/eye";
  import EyeOff from "@lucide/svelte/icons/eye-off";
  import { tooltip } from "./tooltip";

  // An endpoint id, shortened to its ends, with a copy button. `masked`
  // hides it until revealed: our own id is also our room code, so it is kept
  // off the screen by default, as the room code is.
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
  <code title={hidden ? undefined : id}>
    {#if hidden}
      <span aria-hidden="true">••••••••…••••••</span><span class="visually-hidden">{label} hidden</span>
    {:else}
      {short}
    {/if}
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

  code {
    font-size: var(--step--1);
    color: var(--mist);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-right: 4px;
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
