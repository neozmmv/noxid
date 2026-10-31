<script>
  // One participant as a voice orb. The ring grows with how loud they are,
  // which is the whole point of a voice-first call: you see who is talking.
  let { member, micOn = true, small = false, focused = false, onselect = null } = $props();

  const initials = $derived(
    member.name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join("") || "?",
  );

  const muted = $derived(member.is_self ? !micOn : member.connected && !member.has_audio);
  const connecting = $derived(!member.is_self && !member.connected);
  const speaking = $derived(!muted && !connecting && member.level > 0.08);
  // Ring width in px, from audio peak. Self has no meter (we don't loop our
  // own mic back), so it only shows muted or not.
  const ring = $derived(speaking ? 3 + Math.min(member.level, 1) * 14 : 0);

  const status = $derived(
    connecting ? "connecting…" : muted ? "muted" : member.has_video ? "sharing" : "",
  );
</script>

<svelte:element
  this={onselect ? "button" : "div"}
  role={onselect ? undefined : "group"}
  class="orb"
  class:small
  class:focused
  class:connecting
  onclick={onselect}
  aria-label={`${member.name}${member.is_self ? " (you)" : ""}${status ? `, ${status}` : ""}`}
>
  <span class="face" style={`--ring: ${ring}px`}>{initials}</span>
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
    width: var(--size);
    height: var(--size);
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--dusk);
    font-size: calc(var(--size) * 0.32);
    font-weight: 650;
    letter-spacing: 0.02em;
    box-shadow: 0 0 0 var(--ring) var(--lamp);
    transition: box-shadow 120ms linear;
  }

  .small .face {
    --size: 56px;
  }

  .focused .face {
    outline: 2px solid var(--moon);
    outline-offset: 4px;
  }

  .connecting .face {
    opacity: 0.45;
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
