<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "./api";
  import AudioSettings from "./AudioSettings.svelte";
  import type { Snapshot } from "./types";

  let { snapshot }: { snapshot: Snapshot } = $props();

  // Seeded once from the snapshot; after that the field is the user's.
  let name = $state(untrack(() => snapshot.name));
  let roomName = $state("");
  let code = $state("");
  let showAudio = $state(false);
  let busy = $state<"start" | "join" | null>(null);
  let error = $state<string | null>(null);

  const fallbackName = $derived(snapshot.id.slice(0, 10));
  const hasRealName = $derived(name.trim() && name.trim() !== fallbackName);

  async function saveName() {
    if (name.trim() && name.trim() !== snapshot.name) {
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

<div class="lobby">
  <header>
    <h1><span class="orb" aria-hidden="true"></span>Noxid</h1>
    <p>Voice calls and screen sharing for small groups, straight between your computers.</p>
  </header>

  <div class="you">
    <label class="name">
      <span>Your name</span>
      <input class="field" bind:value={name} onblur={saveName} placeholder="How others see you" maxlength="32" />
    </label>
    <button class="button quiet" onclick={() => (showAudio = true)}>Audio settings</button>
  </div>

  <div class="ways">
    <form onsubmit={start}>
      <h2>Start a room</h2>
      <p>You'll host it, and decide who comes in.</p>
      <input class="field" bind:value={roomName} placeholder="Room name (optional)" maxlength="48" />
      <button class="button primary" disabled={busy !== null || !hasRealName}>
        {busy === "start" ? "Starting…" : "Start room"}
      </button>
    </form>

    <form onsubmit={join}>
      <h2>Join a room</h2>
      <p>Paste an invite to come straight in, or a room code to ask the host.</p>
      <input class="field" bind:value={code} placeholder="Invite or room code" spellcheck="false" />
      <button class="button primary" disabled={busy !== null || !code.trim() || !hasRealName}>
        {busy === "join" ? "Waiting for the host…" : "Join"}
      </button>
    </form>
  </div>

  {#if !hasRealName}
    <p class="hint">Pick a name first, so the others know who's calling.</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

{#if showAudio}
  <AudioSettings {snapshot} onclose={() => (showAudio = false)} />
{/if}

<style>
  .lobby {
    max-width: 760px;
    margin: 0 auto;
    padding: 4rem 2rem;
    display: grid;
    gap: 2.25rem;
  }

  h1 {
    display: flex;
    align-items: center;
    gap: 0.9rem;
    margin: 0;
    font-size: var(--step-4);
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  .orb {
    width: 0.9em;
    height: 0.9em;
    border-radius: 50%;
    border: 0.14em solid var(--lamp);
  }

  header p {
    margin: 0.5rem 0 0;
    color: var(--mist);
    font-size: var(--step-1);
    max-width: 34ch;
  }

  .you {
    display: flex;
    align-items: end;
    gap: 0.8rem;
  }

  .name {
    display: grid;
    gap: 0.4rem;
    width: 100%;
    max-width: 320px;
  }

  .name span {
    color: var(--mist);
  }

  .ways {
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: repeat(4, auto);
    row-gap: 0.8rem;
  }

  form {
    display: grid;
    grid-row: span 4;
    grid-template-rows: subgrid;
    align-content: start;
    padding-right: 2rem;
  }

  form + form {
    padding: 0 0 0 2rem;
    border-left: 1px solid var(--line);
  }

  form .button {
    justify-self: start;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
    font-weight: 650;
  }

  form p {
    margin: 0;
    color: var(--mist);
    max-width: 32ch;
  }

  .hint {
    margin: 0;
    color: var(--mist);
  }

  .error {
    margin: 0;
    color: var(--signal);
  }

  @media (max-width: 680px) {
    .ways {
      grid-template-columns: 1fr;
      grid-template-rows: none;
      gap: 2rem;
    }
    form {
      grid-row: auto;
      grid-template-rows: none;
      gap: 0.8rem;
    }
    form + form {
      padding: 2rem 0 0;
      border-left: 0;
      border-top: 1px solid var(--line);
    }
  }
</style>
