<script>
  import { api } from "./api.js";
  import Orb from "./Orb.svelte";
  import ScreenView from "./ScreenView.svelte";

  let { snapshot } = $props();

  const room = $derived(snapshot.room);
  const members = $derived(room.members);
  const self = $derived(members.find((m) => m.is_self));

  // Who is sharing right now, ourselves included.
  const sharers = $derived(members.filter((m) => (m.is_self ? snapshot.sharing : m.has_video)));

  // The share on the big stage. The first remote sharer by default, since
  // that is what people are there to look at; clicking an orb changes it.
  let chosen = $state(null);
  const focused = $derived(
    sharers.find((m) => m.id === chosen) ?? sharers.find((m) => !m.is_self) ?? sharers[0] ?? null,
  );

  // Receiver-side quality: the share on stage follows the link (adaptive,
  // up to the top rung), the others are pinned to the smallest rung since they
  // are not on screen. A per-share choice on the stage overrides this.
  let overrides = $state({});
  $effect(() => {
    for (const m of members) {
      if (m.is_self || !m.has_video) continue;
      const wanted =
        overrides[m.id] ?? (focused?.id === m.id ? { mode: "auto" } : { mode: "pinned", quality: "low" });
      const current = m.quality;
      if (current.mode !== wanted.mode || current.quality !== wanted.quality) {
        api.setPeerQuality(m.id, wanted);
      }
    }
  });

  function qualityChoice(m) {
    const q = overrides[m.id];
    return q ? (q.mode === "auto" ? "auto" : q.quality) : "auto";
  }

  function chooseQuality(id, value) {
    overrides = {
      ...overrides,
      [id]: value === "auto" ? { mode: "auto" } : { mode: "pinned", quality: value },
    };
  }

  // Invites and the room code, for the host.
  let showInvite = $state(false);
  let invite = $state(null);
  let copied = $state(null);

  async function newInvite() {
    invite = await api.invite();
  }

  async function copy(text, what) {
    await navigator.clipboard.writeText(text);
    copied = what;
    setTimeout(() => (copied = null), 1500);
  }

  // Screen sharing.
  let picking = $state(null);
  let error = $state(null);

  async function toggleShare() {
    error = null;
    if (snapshot.sharing) {
      await api.shareScreen(null);
      return;
    }
    try {
      const displays = await api.displays();
      if (displays.length === 1) {
        await api.shareScreen(displays[0].id);
      } else if (displays.length === 0) {
        error = "No screen to share was found.";
      } else {
        picking = displays;
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function share(id) {
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
          <button class="button quiet" onclick={() => copy(room.room_code, "room")}>
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
            <button class="button quiet" onclick={() => copy(invite, "invite")}>
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
    <section class="stage">
      {#key focused.id}
        <ScreenView id={focused.id} label={`${focused.name}'s screen`} />
      {/key}
      <div class="stage-bar">
        <span>{focused.is_self ? "Your screen" : `${focused.name}'s screen`}</span>
        {#if !focused.is_self}
          <span class="meta">
            {#if focused.width}{focused.width}×{focused.height}, {focused.fps} fps{/if}
          </span>
          <label>
            <span class="visually-hidden">Picture quality</span>
            <select
              class="field compact"
              value={qualityChoice(focused)}
              onchange={(e) => chooseQuality(focused.id, e.currentTarget.value)}
            >
              <option value="auto">Quality: automatic</option>
              <option value="high">Quality: high</option>
              <option value="mid">Quality: medium</option>
              <option value="low">Quality: low</option>
            </select>
          </label>
        {/if}
      </div>
    </section>
  {/if}

  <section class="people" aria-label="People in the room">
    {#each members as member (member.id)}
      <div class="person">
        <Orb
          {member}
          micOn={snapshot.mic_on}
          small={!!focused}
          focused={focused?.id === member.id}
          onselect={sharers.some((s) => s.id === member.id) ? () => (chosen = member.id) : null}
        />
        {#if room.is_coordinator && !member.is_self}
          <button class="kick" onclick={() => api.kick(member.id)} aria-label={`Remove ${member.name}`}>
            Remove
          </button>
        {/if}
      </div>
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
        onchange={(e) => api.setMaxQuality(e.currentTarget.value)}
      >
        <option value="high">Send up to 1080p</option>
        <option value="mid">Send up to 720p</option>
        <option value="low">Send up to 360p</option>
      </select>
    </label>
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

  .stage {
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: 0.5rem;
  }

  .stage-bar {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .stage-bar > span:first-child {
    font-weight: 600;
  }

  .meta {
    color: var(--mist);
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

  .person {
    display: grid;
    justify-items: center;
  }

  .kick {
    border: 0;
    background: none;
    color: var(--mist);
    font-size: var(--step--1);
    text-decoration: underline;
    opacity: 0;
  }

  .person:hover .kick,
  .kick:focus-visible {
    opacity: 1;
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
