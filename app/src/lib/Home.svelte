<script lang="ts">
  import { untrack } from "svelte";
  import Phone from "@lucide/svelte/icons/phone";
  import LogIn from "@lucide/svelte/icons/log-in";
  import { api } from "./api";
  import type { Snapshot } from "./types";

  // Outside a call: start a room or join one. The name now lives in the
  // profile card; until there is a real one, a field for it sits here too,
  // because nobody should call in as a string of hex.
  let { snapshot }: { snapshot: Snapshot } = $props();

  let roomName = $state("");
  let code = $state("");
  let busy = $state<"start" | "join" | null>(null);
  let error = $state<string | null>(null);

  const fallbackName = $derived(snapshot.id.slice(0, 10));
  const named = $derived(!!snapshot.name.trim() && snapshot.name.trim() !== fallbackName);
  // The first-run field, before the snapshot carries a real name.
  let name = $state(untrack(() => snapshot.name));
  const typedName = $derived(!!name.trim() && name.trim() !== fallbackName);
  const hasRealName = $derived(named || typedName);

  async function saveName() {
    if (!named && typedName && name.trim() !== snapshot.name) {
      await api.setName(name.trim());
    }
  }

  async function start(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    busy = "start";
    try {
      await saveName();
      await api.createRoom(roomName.trim() || null);
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function join(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    busy = "join";
    try {
      await saveName();
      await api.joinRoom(code.trim());
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }
</script>

<div class="home">
  <header>
    <h1>{named ? `Welcome back, ${snapshot.name}` : "Welcome to Noxid"}</h1>
    <p>Voice calls and screen sharing for small groups, straight between your computers. No server in the middle.</p>
  </header>

  {#if !named}
    <section class="card first-name">
      <label>
        <span>Pick a name first, so the others know who's calling</span>
        <input class="field" bind:value={name} onblur={saveName} placeholder="How others see you" maxlength="32" />
      </label>
    </section>
  {/if}

  <div class="ways">
    <form class="card" onsubmit={start}>
      <div class="title">
        <span class="icon" aria-hidden="true"><Phone size={18} strokeWidth={1.75} /></span>
        <h2>Start a room</h2>
      </div>
      <p>You'll host it, and decide who comes in.</p>
      <input class="field" bind:value={roomName} placeholder="Room name (optional)" maxlength="48" />
      <button class="button primary" disabled={busy !== null || !hasRealName}>
        {busy === "start" ? "Starting…" : "Start room"}
      </button>
    </form>

    <form class="card" onsubmit={join}>
      <div class="title">
        <span class="icon" aria-hidden="true"><LogIn size={18} strokeWidth={1.75} /></span>
        <h2>Join a room</h2>
      </div>
      <p>Paste an invite to come straight in, or a room code to ask the host.</p>
      <input class="field" bind:value={code} placeholder="Invite or room code" spellcheck="false" />
      <button class="button primary" disabled={busy !== null || !code.trim() || !hasRealName}>
        {busy === "join" ? "Waiting for the host…" : "Join"}
      </button>
    </form>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .home {
    max-width: 880px;
    margin: 0 auto;
    padding: 48px 24px;
    display: grid;
    gap: 24px;
    container-type: inline-size;
  }

  h1 {
    margin: 0;
    font-size: var(--step-3);
    font-weight: 700;
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
  }

  header p {
    margin: 8px 0 0;
    color: var(--mist);
    font-size: var(--step-1);
    max-width: 44ch;
  }

  .first-name label {
    display: grid;
    gap: 8px;
    max-width: 360px;
  }

  .first-name span {
    color: var(--mist);
  }

  .ways {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  @container (max-width: 640px) {
    .ways {
      grid-template-columns: 1fr;
    }
  }

  form {
    display: grid;
    grid-template-rows: auto 1fr auto auto;
    gap: 12px;
  }

  .title {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: var(--dusk-raised);
    color: var(--lamp);
  }

  h2 {
    margin: 0;
    font-size: var(--step-1);
    font-weight: 650;
  }

  form p {
    margin: 0;
    color: var(--mist);
  }

  form .button {
    justify-self: start;
  }

  .error {
    margin: 0;
    color: var(--signal);
  }
</style>
