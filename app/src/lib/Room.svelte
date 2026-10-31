<script lang="ts">
  import { api } from "./api";
  import type { DisplayInfo, MemberView, Quality, QualityMode, Snapshot } from "./types";
  import AudioSettings from "./AudioSettings.svelte";
  import Orb from "./Orb.svelte";
  import PeerMenu from "./PeerMenu.svelte";
  import Stage from "./Stage.svelte";

  let { snapshot }: { snapshot: Snapshot } = $props();

  // Room is only mounted while there is a room.
  const room = $derived(snapshot.room!);
  const members = $derived(room.members);

  // Who is sharing right now, ourselves included.
  const sharers = $derived(members.filter((m) => (m.is_self ? snapshot.sharing : m.has_video)));

  // The share on the big stage. The first remote sharer by default, since
  // that is what people are there to look at; clicking an orb changes it.
  let chosen = $state<string | null>(null);
  const focused = $derived(
    sharers.find((m) => m.id === chosen) ?? sharers.find((m) => !m.is_self) ?? sharers[0] ?? null,
  );

  // Receiver-side quality: the share on stage follows the link (adaptive,
  // up to the top rung), the others are pinned to the smallest rung since they
  // are not on screen. A per-share choice on the stage overrides this.
  let overrides = $state<Record<string, QualityMode>>({});
  $effect(() => {
    for (const m of members) {
      if (m.is_self || !m.has_video) continue;
      const wanted: QualityMode =
        overrides[m.id] ?? (focused?.id === m.id ? { mode: "auto" } : { mode: "pinned", quality: "low" });
      if (!sameMode(m.quality, wanted)) {
        api.setPeerQuality(m.id, wanted);
      }
    }
  });

  function sameMode(a: QualityMode, b: QualityMode): boolean {
    if (a.mode === "auto" || b.mode === "auto") return a.mode === b.mode;
    return a.quality === b.quality;
  }

  function qualityChoice(m: MemberView): "auto" | Quality {
    const q = overrides[m.id];
    return !q || q.mode === "auto" ? "auto" : q.quality;
  }

  function chooseQuality(id: string, value: string) {
    overrides = {
      ...overrides,
      [id]: value === "auto" ? { mode: "auto" } : { mode: "pinned", quality: value as Quality },
    };
  }

  // Right-click menu on a participant.
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  const menuMember = $derived(menu ? members.find((m) => m.id === menu!.id) : undefined);

  let showAudio = $state(false);

  // Invites and the room code, for the host.
  let showInvite = $state(false);
  let invite = $state<string | null>(null);
  let copied = $state<"room" | "invite" | null>(null);

  async function newInvite() {
    invite = await api.invite();
  }

  async function copy(text: string, what: "room" | "invite") {
    await navigator.clipboard.writeText(text);
    copied = what;
    setTimeout(() => (copied = null), 1500);
  }

  // Screen sharing.
  let picking = $state<DisplayInfo[] | null>(null);
  let error = $state<string | null>(null);

  async function toggleShare() {
    error = null;
    if (snapshot.sharing) {
      await api.shareScreen(null);
      return;
    }
    try {
      const displays = await api.displays();
      const only = displays.length === 1 ? displays[0] : undefined;
      if (only) {
        await api.shareScreen(only.id);
      } else if (displays.length === 0) {
        error = "No screen to share was found.";
      } else {
        picking = displays;
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function share(id: string) {
    picking = null;
    try {
      await api.shareScreen(id);
    } catch (e) {
      error = String(e);
    }
  }

  let confirmingLeave = $state(false);

  async function leave() {
    if (room.is_coordinator && members.length > 1 && !confirmingLeave) {
      confirmingLeave = true;
      return;
    }
    await api.leaveRoom();
  }
</script>

<div class="room" class:staged={focused}>
  <header>
    <h1>{room.name ?? "Room"}</h1>
    <span class="count">{members.length} {members.length === 1 ? "person" : "people"}</span>
    {#if room.is_coordinator}
      <button class="button quiet" onclick={() => (showInvite = !showInvite)} aria-expanded={showInvite}>
        Invite people
      </button>
    {/if}
  </header>

  {#if showInvite && room.is_coordinator}
    <section class="invite" aria-label="Invite people">
      <div>
        <h2>Room code</h2>
        <p>Anyone with it can ask to join; you let them in.</p>
        <div class="code">
          <code>{room.room_code}</code>
          <button class="button quiet" onclick={() => room.room_code && copy(room.room_code, "room")}>
            {copied === "room" ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <div>
        <h2>Invite</h2>
        <p>Comes straight in, once, within 30 minutes.</p>
        {#if invite}
          <div class="code">
            <code>{invite}</code>
            <button class="button quiet" onclick={() => invite && copy(invite, "invite")}>
              {copied === "invite" ? "Copied" : "Copy"}
            </button>
          </div>
        {/if}
        <button class="button" onclick={newInvite}>{invite ? "Make another" : "Make an invite"}</button>
      </div>
    </section>
  {/if}

  {#each snapshot.knocks as knock (knock.id)}
    <section class="knock" role="alert">
      <span><strong>{knock.name}</strong> is asking to join.</span>
      <button class="button primary" onclick={() => api.answerKnock(knock.id, true)}>Let in</button>
      <button class="button quiet" onclick={() => api.answerKnock(knock.id, false)}>Turn away</button>
    </section>
  {/each}

  {#if focused}
    <Stage
      member={focused}
      quality={qualityChoice(focused)}
      onquality={(value) => chooseQuality(focused.id, value)}
      sendingSound={snapshot.screen_audio}
    />
  {/if}

  <section class="people" aria-label="People in the room">
    {#each members as member (member.id)}
      <Orb
        {member}
        micOn={snapshot.mic_on}
        small={!!focused}
        focused={focused?.id === member.id}
        onselect={sharers.some((s) => s.id === member.id) ? () => (chosen = member.id) : null}
        onmenu={(e) => (menu = { id: member.id, x: e.clientX, y: e.clientY })}
      />
    {/each}
  </section>

  {#if picking}
    <section class="picker" aria-label="Choose a screen">
      <span>Share which screen?</span>
      {#each picking as d (d.id)}
        <button class="button" onclick={() => share(d.id)}>{d.name} ({d.width}×{d.height})</button>
      {/each}
      <button class="button quiet" onclick={() => (picking = null)}>Cancel</button>
    </section>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <footer>
    <button
      class="button"
      class:off={!snapshot.mic_on}
      aria-pressed={!snapshot.mic_on}
      onclick={() => api.setMic(!snapshot.mic_on)}
    >
      {snapshot.mic_on ? "Mute" : "Unmute"}
    </button>
    <button class="button" aria-pressed={snapshot.sharing} onclick={toggleShare}>
      {snapshot.sharing ? "Stop sharing" : "Share screen"}
    </button>
    <label>
      <span class="visually-hidden">Highest quality you send</span>
      <select
        class="field compact"
        value={snapshot.max_quality}
        onchange={(e) => api.setMaxQuality(e.currentTarget.value as Quality)}
      >
        <option value="high">Send up to 1080p</option>
        <option value="mid">Send up to 720p</option>
        <option value="low">Send up to 360p</option>
      </select>
    </label>
    <button class="button quiet" onclick={() => (showAudio = true)}>Audio</button>
    <span class="spacer"></span>
    {#if confirmingLeave}
      <span class="confirm">Leaving ends the room for everyone.</span>
      <button class="button quiet" onclick={() => (confirmingLeave = false)}>Stay</button>
    {/if}
    <button class="button danger" onclick={leave}>
      {room.is_coordinator ? "End room" : "Leave"}
    </button>
  </footer>
</div>

{#if menu && menuMember}
  {#key menu}
    <PeerMenu
      member={menuMember}
      x={menu.x}
      y={menu.y}
      isHost={room.is_coordinator}
      micOn={snapshot.mic_on}
      onclose={() => (menu = null)}
      onaudiosettings={() => (showAudio = true)}
    />
  {/key}
{/if}

{#if showAudio}
  <AudioSettings {snapshot} onclose={() => (showAudio = false)} />
{/if}

<style>
  .room {
    height: 100%;
    display: grid;
    grid-template-rows: auto auto auto 1fr auto auto auto;
    padding: 1.25rem 1.5rem 1rem;
    gap: 0.9rem;
  }

  .room.staged {
    grid-template-rows: auto auto auto minmax(0, 1fr) auto auto auto;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.9rem;
  }

  h1 {
    margin: 0;
    font-size: var(--step-3);
    font-weight: 700;
    letter-spacing: -0.015em;
  }

  .count {
    color: var(--mist);
    flex: 1;
  }

  .invite {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2rem;
    padding: 1rem 0;
    border-top: 1px solid var(--dusk-raised);
    border-bottom: 1px solid var(--dusk-raised);
  }

  .invite > div {
    display: grid;
    gap: 0.5rem;
    align-content: start;
    justify-items: start;
  }

  .invite h2 {
    margin: 0;
    font-size: var(--step-1);
  }

  .invite p {
    margin: 0;
    color: var(--mist);
  }

  .code {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    max-width: 100%;
  }

  code {
    padding: 0.4rem 0.6rem;
    border-radius: 8px;
    background: var(--dusk);
    font-family: inherit;
    font-size: var(--step--1);
    overflow-wrap: anywhere;
  }

  .knock {
    display: flex;
    align-items: center;
    gap: 0.7rem;
    padding: 0.7rem 1rem;
    border-radius: 14px;
    background: var(--dusk);
    border-left: 4px solid var(--lamp);
  }

  .knock span {
    flex: 1;
  }

  .people {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    align-content: center;
    gap: 1.5rem 2.5rem;
  }

  .staged .people {
    align-content: start;
    gap: 0.5rem 1.25rem;
  }

  .picker {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
  }

  .error {
    margin: 0;
    color: var(--signal);
  }

  footer {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding-top: 0.9rem;
    border-top: 1px solid var(--dusk-raised);
  }

  footer .off {
    background: var(--signal);
    color: var(--ink);
  }

  .spacer {
    flex: 1;
  }

  .confirm {
    color: var(--mist);
  }

  .compact {
    width: auto;
    padding: 0.5rem 0.7rem;
    border-radius: 999px;
  }
</style>
