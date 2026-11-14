<script lang="ts">
  import { api } from "./api";
  import { setMic } from "./controls";
  import { friends } from "./friendlist.svelte";
  import { showToast } from "./toast.svelte";
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
    deafened: boolean;
    onclose: () => void;
    onaudiosettings: () => void;
    /** Opens the file picker to send them a file. */
    onsendfile: () => void;
  }

  let { member, x, y, isHost, micOn, deafened, onclose, onaudiosettings, onsendfile }: Props = $props();

  function setAudio(stream: AudioStream, volume: number, muted: boolean) {
    api.setPeerAudio(member.id, stream, volume, muted);
  }

  const percent = (gain: number) => Math.round(gain * 100);

  // Where we stand with them as friends, from the list the backend sent.
  // Their member id is their endpoint id, which is what a Noxid ID is.
  const friendship = $derived.by(() => {
    const view = friends.view;
    if (!view || view.problem) return "unavailable";
    const friend = view.friends.find((f) => f.id === member.id);
    if (friend) return friend.finishing ? "finishing" : "friends";
    if (view.incoming.some((r) => r.id === member.id)) return "incoming";
    if (view.outgoing.some((r) => r.id === member.id)) return "outgoing";
    return "none";
  });

  async function befriend() {
    onclose();
    try {
      if (friendship === "incoming") {
        await api.answerFriendRequest(member.id, true);
        showToast(`Accepted ${member.name}'s friend request.`);
      } else {
        await api.sendFriendRequest(member.id);
        showToast(`Friend request sent to ${member.name}. You'll be friends once they accept.`);
      }
    } catch (e) {
      showToast(String(e));
    }
  }

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
    <button class="item" onclick={() => (setMic(!micOn, deafened), onclose())}>
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

    <hr />
    <button class="item" disabled={!member.connected} onclick={() => (onsendfile(), onclose())}>
      Send a file…
    </button>
    {#if friendship === "none"}
      <button class="item" onclick={befriend}>Add friend</button>
    {:else if friendship === "incoming"}
      <button class="item" onclick={befriend}>Accept friend request</button>
    {:else if friendship === "outgoing"}
      <button class="item" disabled>Friend request sent</button>
    {:else if friendship === "finishing"}
      <button class="item" disabled>Becoming friends…</button>
    {:else if friendship === "friends"}
      <button class="item" disabled>Friends</button>
    {/if}

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
