<script lang="ts">
  import MicOff from "@lucide/svelte/icons/mic-off";
  import { REMOTE_THRESHOLD, fromDb, prefs, voice } from "./levels.svelte";
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
    /** Whether files can be dropped on them to send. */
    dropTarget?: boolean;
    /** Whether files are being dragged over them right now. */
    dropping?: boolean;
  }

  let {
    member,
    micOn = true,
    small = false,
    focused = false,
    onselect = null,
    onmenu = null,
    dropTarget = false,
    dropping = false,
  }: Props = $props();

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
  // Peak above which someone counts as speaking: our own sensitivity setting
  // for us, a fixed level for everyone else.
  const threshold = $derived(member.is_self ? fromDb(prefs.sensitivityDb) : REMOTE_THRESHOLD);
  let speaking = $state(false);
  let release: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    if (!micOff && !connecting && level > threshold) {
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
            : member.has_camera
              ? "camera on"
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
  class:dropping
  data-drop-peer={dropTarget ? member.id : undefined}
  onclick={onselect}
  oncontextmenu={contextmenu}
  aria-label={`${member.name}${member.is_self ? " (you)" : ""}${status ? `, ${status}` : ""}${speaking ? ", speaking" : ""}`}
>
  <span class="face">
    {initials}
    {#if micOff || mutedByMe}
      <span class="badge" class:mine={mutedByMe && !micOff} aria-hidden="true">
        <MicOff size={small ? 11 : 14} strokeWidth={2.5} />
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
    gap: 0.75rem;
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

  .orb.small {
    gap: 0.85rem;
  }

  .small.focused .face {
    outline-offset: 6px;
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

  /* Files held over them: the face shows it will take them. */
  .dropping .face {
    background: var(--dusk-raised);
    outline: 2px dashed var(--lamp);
    outline-offset: 6px;
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

  /* A long name wraps onto more lines rather than being cut short. */
  .name {
    font-weight: 600;
    max-width: 14ch;
    text-align: center;
    overflow-wrap: anywhere;
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
