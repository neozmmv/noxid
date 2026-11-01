<script lang="ts">
  import { api } from "./api";
  import Menu from "./Menu.svelte";
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
</script>

<Menu label={member.name} {x} {y} {onclose}>
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
          max="100"
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
</Menu>
