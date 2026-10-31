<script lang="ts">
  import { voice } from "./levels.svelte";
  import type { MemberView } from "./types";

  // One participant as a voice orb. A ring lights up around it while they
  // speak, which is the whole point of a voice-first call: you see who it is.
  interface Props {
    member: MemberView;
    micOn?: boolean;
    small?: boolean;
    focused?: boolean;
    onselect?: (() => void) | null;
    onmenu?: ((event: MouseEvent) => void) | null;
  }

  let { member, micOn = true, small = false, focused = false, onselect = null, onmenu = null }: Props =
    $props();

  /** Peak level above which someone counts as speaking. */
  const THRESHOLD = 0.06;
  /** How long the ring stays lit after the level drops, so it does not
   *  flicker between syllables. */
  const HOLD_MS = 300;

  const initials = $derived(
    member.name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w.charAt(0).toUpperCase())
      .join("") || "?",
  );

  // Muted at the source (they turned their mic off) versus muted by us.
  const micOff = $derived(member.is_self ? !micOn : member.connected && !member.has_audio);
  const mutedByMe = $derived(!member.is_self && member.voice_muted);
  const connecting = $derived(!member.is_self && !member.connected);

  const level = $derived(voice.levels[member.id] ?? 0);
  let speaking = $state(false);
  let release: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    if (!micOff && !connecting && level > THRESHOLD) {
      clearTimeout(release);
      release = undefined;
      speaking = true;
    } else if (speaking && release === undefined) {
      release = setTimeout(() => {
        speaking = false;
        release = undefined;
      }, HOLD_MS);
    }
  });

  $effect(() => () => clearTimeout(release));

  const status = $derived(
    connecting
      ? "connecting…"
      : micOff
        ? "mic off"
        : mutedByMe
          ? "muted for you"
          : member.has_video
            ? "sharing"
            : "",
  );

  function contextmenu(event: MouseEvent) {
    if (!onmenu) return;
    event.preventDefault();
    onmenu(event);
  }
</script>

<svelte:element
  this={onselect ? "button" : "div"}
  role={onselect ? undefined : "group"}
  class="orb"
  class:small
  class:focused
  class:connecting
  class:speaking
  onclick={onselect}
  oncontextmenu={contextmenu}
  aria-label={`${member.name}${member.is_self ? " (you)" : ""}${status ? `, ${status}` : ""}${speaking ? ", speaking" : ""}`}
>
  <span class="face">
    {initials}
    {#if micOff || mutedByMe}
      <span class="badge" class:mine={mutedByMe && !micOff} aria-hidden="true">
        <svg viewBox="0 0 24 24" width="14" height="14">
          <path
            fill="currentColor"
            d="M12 14a3 3 0 0 0 3-3V5a3 3 0 1 0-6 0v6a3 3 0 0 0 3 3Zm5-3a5 5 0 0 1-8.58 3.5l-1.42 1.42A7 7 0 0 0 11 17.93V21h2v-3.07A7 7 0 0 0 19 11h-2ZM3.7 2.3 2.3 3.7l18 18 1.4-1.4-18-18Z"
          />
        </svg>
      </span>
    {/if}
  </span>
  <span class="name">
    {member.name}{#if member.is_self}<span class="you"> (you)</span>{/if}
  </span>
  <span class="status">
    {#if member.is_coordinator}host{/if}{#if member.is_coordinator && status}, {/if}{status}
  </span>
</svelte:element>

<style>
  .orb {
    display: grid;
    justify-items: center;
    gap: 0.35rem;
    padding: 0.5rem;
    border: 0;
    background: none;
    border-radius: 16px;
    min-width: 0;
  }

  button.orb:hover .face {
    background: var(--dusk-raised);
  }

  .face {
    --size: 104px;
    position: relative;
    width: var(--size);
    height: var(--size);
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--dusk);
    font-size: calc(var(--size) * 0.32);
    font-weight: 650;
    letter-spacing: 0.02em;
    /* A gap of background between face and ring, so the ring reads as a
       separate indicator rather than a thicker edge. */
    box-shadow:
      0 0 0 3px var(--ink),
      0 0 0 6px transparent;
    transition: box-shadow 90ms linear;
  }

  .speaking .face {
    box-shadow:
      0 0 0 3px var(--ink),
      0 0 0 6px var(--lamp);
  }

  .small .face {
    --size: 56px;
  }

  .small.speaking .face {
    box-shadow:
      0 0 0 2px var(--ink),
      0 0 0 4px var(--lamp);
  }

  .focused .face {
    outline: 2px solid var(--moon);
    outline-offset: 9px;
  }

  .connecting .face {
    opacity: 0.45;
  }

  .badge {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--signal);
    color: var(--ink);
    border: 3px solid var(--ink);
  }

  /* Muted by us rather than by them: quieter, since it is our choice. */
  .badge.mine {
    background: var(--mist);
  }

  .small .badge {
    width: 22px;
    height: 22px;
    right: -3px;
    bottom: -3px;
    border-width: 2px;
  }

  .small .badge svg {
    width: 11px;
    height: 11px;
  }

  .name {
    font-weight: 600;
    max-width: 14ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .you {
    color: var(--mist);
    font-weight: 400;
  }

  .status {
    min-height: 1.2em;
    font-size: var(--step--1);
    color: var(--mist);
  }

  .small .status {
    display: none;
  }
</style>
