<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api";
  import type { AudioStream, MemberView } from "./types";

  // The right-click menu on a participant: how loud we hear them, and for the
  // host, removing them. Everything here is local except removing.
  interface Props {
    member: MemberView;
    x: number;
    y: number;
    isHost: boolean;
    micOn: boolean;
    onclose: () => void;
    onaudiosettings: () => void;
  }

  let { member, x, y, isHost, micOn, onclose, onaudiosettings }: Props = $props();

  let menu: HTMLElement;
  // Where the menu actually sits, after keeping it inside the window.
  let left = $state(0);
  let top = $state(0);

  onMount(() => {
    const rect = menu.getBoundingClientRect();
    left = Math.max(8, Math.min(x, window.innerWidth - rect.width - 8));
    top = Math.max(8, Math.min(y, window.innerHeight - rect.height - 8));
    menu.querySelector<HTMLElement>("input, button")?.focus();
  });

  function setAudio(stream: AudioStream, volume: number, muted: boolean) {
    api.setPeerAudio(member.id, stream, volume, muted);
  }

  const percent = (gain: number) => Math.round(gain * 100);

  let confirmingRemove = $state(false);

  async function remove() {
    if (!confirmingRemove) {
      confirmingRemove = true;
      return;
    }
    await api.kick(member.id);
    onclose();
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }

  function pointerdown(event: PointerEvent) {
    if (!menu.contains(event.target as Node)) onclose();
  }
</script>

<svelte:window onkeydown={keydown} onpointerdown={pointerdown} onblur={onclose} />

<div
  class="menu"
  role="dialog"
  aria-label={`${member.name}`}
  bind:this={menu}
  style={`left: ${left}px; top: ${top}px`}
>
  <p class="who">{member.name}{#if member.is_self}<span> (you)</span>{/if}</p>

  {#if member.is_self}
    <button class="item" onclick={() => (api.setMic(!micOn), onclose())}>
      {micOn ? "Turn mic off" : "Turn mic on"}
    </button>
    <button class="item" onclick={() => (onaudiosettings(), onclose())}>Audio settings</button>
  {:else}
    <label class="row">
      <input
        type="checkbox"
        checked={member.voice_muted}
        onchange={(e) => setAudio("voice", member.voice_volume, e.currentTarget.checked)}
      />
      Mute for me
    </label>
    <label class="slider">
      <span>Voice volume <output>{percent(member.voice_volume)}%</output></span>
      <input
        type="range"
        min="0"
        max="200"
        step="5"
        value={percent(member.voice_volume)}
        disabled={member.voice_muted}
        oninput={(e) => setAudio("voice", Number(e.currentTarget.value) / 100, member.voice_muted)}
      />
    </label>

    {#if member.has_screen_audio}
      <hr />
      <label class="row">
        <input
          type="checkbox"
          checked={member.screen_muted}
          onchange={(e) => setAudio("screen", member.screen_volume, e.currentTarget.checked)}
        />
        Mute their screen sound
      </label>
      <label class="slider">
        <span>Screen sound <output>{percent(member.screen_volume)}%</output></span>
        <input
          type="range"
          min="0"
          max="200"
          step="5"
          value={percent(member.screen_volume)}
          disabled={member.screen_muted}
          oninput={(e) => setAudio("screen", Number(e.currentTarget.value) / 100, member.screen_muted)}
        />
      </label>
    {/if}

    {#if isHost}
      <hr />
      <button class="item danger" onclick={remove}>
        {confirmingRemove ? `Remove ${member.name}? Click again` : "Remove from room"}
      </button>
    {/if}
  {/if}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    width: 260px;
    padding: 0.5rem;
    border-radius: 12px;
    background: var(--dusk);
    border: 1px solid var(--dusk-raised);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
    display: grid;
    gap: 0.15rem;
  }

  .who {
    margin: 0;
    padding: 0.35rem 0.5rem 0.5rem;
    font-weight: 650;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .who span {
    color: var(--mist);
    font-weight: 400;
  }

  .item,
  .row {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    width: 100%;
    padding: 0.5rem;
    border: 0;
    border-radius: 8px;
    background: none;
    text-align: left;
  }

  .item:hover,
  .row:hover {
    background: var(--dusk-raised);
  }

  .danger {
    color: var(--signal);
  }

  .slider {
    display: grid;
    gap: 0.35rem;
    padding: 0.35rem 0.5rem 0.6rem;
  }

  .slider span {
    display: flex;
    justify-content: space-between;
    color: var(--mist);
    font-size: var(--step--1);
  }

  output {
    color: var(--moon);
    font-variant-numeric: tabular-nums;
  }

  input[type="range"] {
    width: 100%;
    accent-color: var(--lamp);
  }

  input[type="checkbox"] {
    accent-color: var(--lamp);
    width: 1rem;
    height: 1rem;
  }

  hr {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--dusk-raised);
    margin: 0.25rem 0;
  }
</style>
