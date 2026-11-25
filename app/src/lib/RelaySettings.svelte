<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "./api";
  import type { OnlyRelayStatus, RelayHealth, Snapshot } from "./types";

  // One relay of the user's own, used beside the default ones. Saving it
  // checks it at once; Test checks the saved one again.
  let { snapshot }: { snapshot: Snapshot } = $props();

  // Seeded once from the snapshot; after that the field is the user's.
  let url = $state(untrack(() => snapshot.custom_relay ?? ""));
  let error = $state<string | null>(null);
  let busy = $state(false);
  let health = $state<RelayHealth | "checking" | null>(null);
  // Bumped on every check and removal, so a slow answer about a relay that
  // is no longer saved is dropped.
  let generation = 0;

  const saved = $derived(snapshot.custom_relay);
  const changed = $derived(url.trim() !== "" && url.trim() !== (saved ?? ""));

  const messages: Record<RelayHealth | "checking", string> = {
    checking: "Checking…",
    good: "All good.",
    no_address_discovery:
      "Relay is reachable, but QUIC address discovery failed. Connections may go through the relay instead of P2P.",
    unreachable: "Relay unreachable or not an iroh relay.",
  };

  // While it is the only relay, how it is doing, from the node itself.
  const onlyMessages: Record<OnlyRelayStatus, string> = {
    connected: "Connected. Your codes name this relay.",
    connecting: "Connecting to this relay…",
    unreachable:
      "Only this relay is allowed and it isn't reachable. Turn off “Use only this relay” to use the default relays.",
  };
  const onlyDot: Record<OnlyRelayStatus, string> = {
    connected: "good",
    connecting: "",
    unreachable: "unreachable",
  };

  async function setOnly(event: Event & { currentTarget: HTMLInputElement }) {
    const box = event.currentTarget;
    const on = box.checked;
    error = null;
    busy = true;
    try {
      await api.setCustomRelayOnly(on);
    } catch (e) {
      box.checked = !on;
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function check(target: string) {
    const mine = ++generation;
    health = "checking";
    try {
      const result = await api.checkRelay(target);
      if (mine === generation) health = result;
    } catch (e) {
      if (mine === generation) {
        health = null;
        error = String(e);
      }
    }
  }

  async function save() {
    if (!changed || busy) return;
    const next = url.trim();
    error = null;
    busy = true;
    try {
      await api.setCustomRelay(next);
    } catch (e) {
      error = String(e);
      return;
    } finally {
      busy = false;
    }
    void check(next);
  }

  async function remove() {
    error = null;
    busy = true;
    try {
      await api.setCustomRelay(null);
      generation++;
      health = null;
      url = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void save();
    }
  }
</script>

<div class="relay">
  <small>Used beside the default relays, so you stay reachable where those are blocked. Friends reach you through it without setting it up.</small>

  <label class="url">
    <span>Relay URL</span>
    <input
      class="field"
      bind:value={url}
      oninput={() => (error = null)}
      onkeydown={keydown}
      placeholder="https://relay.example.com"
      spellcheck="false"
      autocomplete="off"
      aria-invalid={error ? "true" : undefined}
    />
    {#if error}
      <small class="error" role="alert">{error}</small>
    {/if}
  </label>

  <div class="actions">
    <button class="button primary" onclick={save} disabled={!changed || busy}>Save</button>
    {#if saved}
      <button class="button quiet" onclick={() => check(saved)} disabled={busy || health === "checking"}>Test</button>
      <button class="button quiet" onclick={remove} disabled={busy}>Remove</button>
    {/if}
  </div>

  {#if health}
    <p class="health" role="status">
      <span class={["dot", health]} aria-hidden="true"></span>
      {messages[health]}
    </p>
  {/if}

  <label class="check">
    <input
      type="checkbox"
      checked={snapshot.custom_relay_only}
      disabled={!saved || busy}
      onchange={setOnly}
    />
    <span class:off={!saved}>
      Use only this relay
      <small>
        For talking to people on networks that block the default relays; all of you should turn it on.
        Calls that can't go direct use this relay's bandwidth.
      </small>
    </span>
  </label>

  {#if snapshot.only_relay_status}
    <p class="health" role="status">
      <span class={["dot", onlyDot[snapshot.only_relay_status]]} aria-hidden="true"></span>
      {onlyMessages[snapshot.only_relay_status]}
    </p>
  {/if}
</div>

<style>
  .relay {
    display: grid;
    gap: 16px;
  }

  .relay > small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .url {
    display: grid;
    gap: 6px;
  }

  .url span {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .error {
    color: var(--signal);
  }

  .check {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: start;
    gap: 0.6rem;
  }

  .check input {
    margin-top: 0.25rem;
    accent-color: var(--lamp);
    width: 1rem;
    height: 1rem;
  }

  .check span {
    color: var(--moon);
    display: grid;
    gap: 0.2rem;
  }

  .check small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .check .off {
    opacity: 0.5;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .health {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0;
  }

  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--mist);
    /* Sits on the first line of a message that may wrap. */
    translate: 0 1px;
  }

  .good {
    background: var(--status-online);
  }

  .no_address_discovery {
    background: var(--status-away);
  }

  .unreachable {
    background: var(--signal);
  }
</style>
