<script>
  import { onMount } from "svelte";
  import { api, onSnapshot, onNotice, connectFrames } from "./lib/api.js";
  import Lobby from "./lib/Lobby.svelte";
  import Room from "./lib/Room.svelte";

  let snapshot = $state(null);
  let notice = $state(null);
  let startError = $state(null);

  const NOTICES = {
    kicked: "The host removed you from the room.",
    room_closed: "The host ended the room.",
  };

  function showNotice(n) {
    notice = NOTICES[n.kind] ?? n.message ?? "Something went wrong.";
    setTimeout(() => (notice = null), 6000);
  }

  onMount(() => {
    const unlisten = [onSnapshot((s) => (snapshot = s)), onNotice(showNotice)];
    api
      .snapshot()
      .then((s) => (snapshot = s))
      .catch((e) => (startError = String(e)));
    connectFrames();
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });
</script>

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
  {:else if snapshot.room}
    <Room {snapshot} />
  {:else}
    <Lobby {snapshot} />
  {/if}

  {#if notice}
    <div class="notice" role="status">{notice}</div>
  {/if}
</main>

<style>
  main {
    height: 100%;
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
