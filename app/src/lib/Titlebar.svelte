<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
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

  onMount(() => {
    const sync = () => win.isMaximized().then((m) => (maximized = m));
    sync();
    const unlisten = win.onResized(sync);
    return () => void unlisten.then((f) => f());
  });
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="mark" data-tauri-drag-region>
    <span class="ring" aria-hidden="true" data-tauri-drag-region></span>
    Noxid
  </span>

  <div class="controls">
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

  .controls button:focus-visible {
    outline-offset: -2px;
  }
</style>
