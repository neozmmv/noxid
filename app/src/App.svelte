<script lang="ts">
  import { onMount } from "svelte";
  import { api, onSnapshot, onNotice, onLevels, onFriends, connectFrames } from "./lib/api";
  import { friends } from "./lib/friendlist.svelte";
  import { showToast, toast } from "./lib/toast.svelte";
  import { voice } from "./lib/levels.svelte";
  import type { Notice, Snapshot } from "./lib/types";
  import Shell from "./lib/Shell.svelte";
  import Titlebar from "./lib/Titlebar.svelte";

  let snapshot = $state<Snapshot | null>(null);
  let startError = $state<string | null>(null);

  function describe(n: Notice): string | null {
    switch (n.kind) {
      case "kicked":
        return "The host removed you from the room.";
      case "room_closed":
        return "The host ended the room.";
      case "error":
        return n.message;
      case "friend":
        switch (n.message.kind) {
          case "request_received":
            return `${n.message.name} sent you a friend request.`;
          case "friend_added":
            return `You and ${n.message.name} are now friends.`;
          case "request_declined":
            return `${n.message.name} declined your friend request.`;
          case "missed_call":
            return `Missed call from ${n.message.name}.`;
          case "ringing":
            // The ringing card says it.
            return null;
        }
    }
  }

  function showNotice(n: Notice) {
    const message = describe(n);
    if (message) showToast(message);
  }

  onMount(() => {
    const unlisten = [
      onSnapshot((s) => (snapshot = s)),
      onNotice(showNotice),
      onLevels((l) => (voice.levels = l)),
      onFriends((f) => (friends.view = f)),
    ];
    api
      .snapshot()
      .then((s) => (snapshot = s))
      .catch((e) => (startError = String(e)));
    api
      .friends()
      .then((f) => (friends.view ??= f))
      .catch(() => {});
    connectFrames();
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });
</script>

<Titlebar />
<main>
  {#if startError}
    <section class="status">
      <h1>Noxid could not start</h1>
      <p>{startError}</p>
    </section>
  {:else if !snapshot}
    <section class="status">
      <div class="pulse" aria-hidden="true"></div>
      <p>Finding the network…</p>
    </section>
  {:else}
    <Shell {snapshot} />
  {/if}

  {#if toast.message}
    <div class="notice" role="status">{toast.message}</div>
  {/if}
</main>

<style>
  /* Below the titlebar, which stays put while the page scrolls. */
  main {
    height: calc(100% - 34px);
    overflow: hidden;
  }

  .status {
    height: 100%;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 1rem;
    color: var(--mist);
  }

  .status h1 {
    font-size: var(--step-2);
    color: var(--moon);
    margin: 0;
  }

  .pulse {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    border: 3px solid var(--lamp);
    animation: breathe 1.6s ease-in-out infinite;
  }

  @keyframes breathe {
    50% {
      transform: scale(0.8);
      opacity: 0.5;
    }
  }

  .notice {
    position: fixed;
    left: 50%;
    bottom: 6.5rem;
    transform: translateX(-50%);
    padding: 0.7rem 1.2rem;
    border-radius: 12px;
    background: var(--moon);
    color: var(--ink);
    font-weight: 600;
  }
</style>
