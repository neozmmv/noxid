<script lang="ts">
  import Phone from "@lucide/svelte/icons/phone";
  import PhoneOff from "@lucide/svelte/icons/phone-off";
  import { onDestroy } from "svelte";
  import { api } from "./api";
  import Avatar from "./Avatar.svelte";
  import { answerRing } from "./calls";
  import { friends } from "./friendlist.svelte";
  import { startRinging, stopRinging } from "./ringtone";
  import type { Snapshot } from "./types";

  // Calls in both directions, over whatever view is showing: a card for a
  // friend ringing us, and a bar while we ring one.
  let { snapshot }: { snapshot: Snapshot } = $props();

  const rings = $derived(friends.view?.rings ?? []);
  const calling = $derived(friends.view?.calling ?? null);

  // Ring for as long as anyone is ringing.
  $effect(() => {
    if (rings.length > 0) startRinging();
    else stopRinging();
  });
  onDestroy(stopRinging);

  // A countdown, so it is clear the ring gives up on its own.
  let now = $state(Date.now());
  $effect(() => {
    if (rings.length === 0) return;
    const timer = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(timer);
  });
</script>

{#if rings.length > 0}
  <div class="rings" role="region" aria-label="Incoming calls">
    {#each rings as ring (ring.id)}
      <div class="card ring" role="alertdialog" aria-label={`${ring.name} is calling`}>
        <Avatar name={ring.name} size={44} />
        <div class="text">
          <strong>{ring.name} is calling</strong>
          <span>
            {ring.room_name ? `Into "${ring.room_name}"` : "Into their room"}
            · {Math.max(0, Math.ceil((ring.expires_at - now) / 1000))}s
          </span>
          {#if snapshot.room}
            <small>Accepting leaves your current call.</small>
          {/if}
        </div>
        <button class="round decline" aria-label="Decline" onclick={() => answerRing(ring.id, false)}>
          <PhoneOff size={18} aria-hidden="true" />
        </button>
        <button class="round accept" aria-label="Accept" onclick={() => answerRing(ring.id, true)}>
          <Phone size={18} aria-hidden="true" />
        </button>
      </div>
    {/each}
  </div>
{/if}

{#if calling}
  <div class="calling" role="status">
    <span class="pulse" aria-hidden="true"></span>
    Calling {calling.name}…
    <button class="button danger small" onclick={() => api.hangUp()}>Hang up</button>
  </div>
{/if}

<style>
  .rings {
    position: fixed;
    z-index: 35;
    top: 46px;
    right: 16px;
    display: grid;
    gap: 8px;
    width: min(380px, calc(100vw - 32px));
  }

  .ring {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px;
    border-color: var(--lamp);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
  }

  .text {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }

  .text strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .text span,
  .text small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .round {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    flex: none;
    border: 0;
    border-radius: 50%;
  }

  .accept {
    background: var(--lamp);
    color: var(--ink);
  }

  .accept:hover {
    background: #ffc964;
  }

  .decline {
    background: var(--signal);
    color: #fff;
  }

  .decline:hover {
    background: #c4434c;
  }

  .calling {
    position: fixed;
    z-index: 35;
    top: 46px;
    left: 50%;
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 16px;
    border-radius: 999px;
    background: var(--dusk-raised);
    border: 1px solid var(--line);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.4);
    white-space: nowrap;
  }

  .small {
    padding: 0.35rem 0.8rem;
    border-radius: 999px;
  }

  .pulse {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--lamp);
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
