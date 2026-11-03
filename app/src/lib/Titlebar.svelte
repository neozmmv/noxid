<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import CircleArrowUp from "@lucide/svelte/icons/circle-arrow-up";
  import { api } from "./api";
  import { tooltip } from "./tooltip";
  import { newerRelease } from "./update";
  import Copy from "@lucide/svelte/icons/copy";
  import Minus from "@lucide/svelte/icons/minus";
  import Square from "@lucide/svelte/icons/square";
  import X from "@lucide/svelte/icons/x";
  import { onMount } from "svelte";

  // The window's own bar, drawn by the app so it sits in the night palette
  // rather than the system's. The empty stretch drags the window, and a
  // double-click on it maximizes, as a native bar would.
  const win = getCurrentWindow();
  let maximized = $state(false);
  // A newer release's tag, once one is found.
  let update = $state<string | null>(null);
  // Checked at start and then now and again, for a window left open for days.
  const RECHECK = 6 * 60 * 60 * 1000;

  onMount(() => {
    const sync = () => win.isMaximized().then((m) => (maximized = m));
    sync();
    const unlisten = win.onResized(sync);
    const check = () => newerRelease().then((tag) => (update = tag));
    check();
    const timer = setInterval(check, RECHECK);
    return () => {
      clearInterval(timer);
      void unlisten.then((f) => f());
    };
  });
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="mark" data-tauri-drag-region>
    <span class="ring" aria-hidden="true" data-tauri-drag-region></span>
    Noxid
  </span>

  <div class="controls">
    {#if update}
      <button
        class="update"
        use:tooltip={`Noxid ${update} is out`}
        onclick={() => update && api.openRelease(update)}
      >
        <CircleArrowUp size={14} aria-hidden="true" />
        Update
      </button>
    {/if}
    <button aria-label="Minimize" onclick={() => win.minimize()}>
      <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
    </button>
    <button aria-label={maximized ? "Restore" : "Maximize"} onclick={() => win.toggleMaximize()}>
      {#if maximized}
        <Copy size={13} strokeWidth={1.5} aria-hidden="true" />
      {:else}
        <Square size={13} strokeWidth={1.5} aria-hidden="true" />
      {/if}
    </button>
    <button class="close" aria-label="Close" onclick={() => win.close()}>
      <X size={17} strokeWidth={1.5} aria-hidden="true" />
    </button>
  </div>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    height: 34px;
    background: var(--ink);
    user-select: none;
    -webkit-user-select: none;
  }

  .mark {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0 0.9rem;
    color: var(--mist);
    font-size: var(--step--1);
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  /* The app icon's ring: the one warm thing up here. */
  .ring {
    width: 12px;
    height: 12px;
    border: 2.5px solid var(--lamp);
    border-radius: 50%;
  }

  .controls {
    display: flex;
  }

  .controls button {
    display: grid;
    place-items: center;
    width: 46px;
    border: 0;
    background: transparent;
    color: var(--mist);
  }

  .controls button:hover {
    background: var(--dusk-raised);
    color: var(--moon);
  }

  .controls .close:hover {
    background: var(--signal);
    color: var(--ink);
  }

  /* Lit like the ring in the corner: the one thing up here asking for you. */
  .controls .update {
    display: flex;
    gap: 0.35rem;
    align-self: center;
    width: auto;
    height: 24px;
    margin-right: 0.4rem;
    padding: 0 0.65rem;
    border-radius: 7px;
    background: rgb(246 185 74 / 0.12);
    color: var(--lamp);
    font-size: var(--step--1);
    font-weight: 600;
  }

  .controls .update:hover {
    background: var(--lamp);
    color: var(--ink);
  }

  .controls button:focus-visible {
    outline-offset: -2px;
  }
</style>
