<script lang="ts">
  import { api } from "./api";
  import type { DisplayInfo, MemberView, Quality, QualityMode, Snapshot } from "./types";
  import AudioSettings from "./AudioSettings.svelte";
  import Orb from "./Orb.svelte";
  import PeerMenu from "./PeerMenu.svelte";
  import ScreenMenu from "./ScreenMenu.svelte";
  import ScreenView from "./ScreenView.svelte";
  import SharePicker from "./SharePicker.svelte";
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
  // Every other share, as a thumbnail beside the stage.
  const others = $derived(sharers.filter((m) => m.id !== focused?.id));

  // Receiver-side quality: the share on stage is held at the best rung the
  // sharer sends, the others at the smallest since they are only thumbnails.
  // "Automatic" (following the link) is a choice rather than the default: its
  // bandwidth estimate rarely reads high enough over home internet to step up
  // to the top rung, so it left people watching 720p or 360p on links that
  // carried 1080p fine. A per-share choice on the stage overrides this.
  let overrides = $state<Record<string, QualityMode>>({});

  function wantedMode(m: MemberView): QualityMode {
    return (
      overrides[m.id] ??
      (focused?.id === m.id ? { mode: "pinned", quality: "high" } : { mode: "pinned", quality: "low" })
    );
  }

  $effect(() => {
    for (const m of members) {
      if (m.is_self || !m.has_video) continue;
      const wanted = wantedMode(m);
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
    const q = wantedMode(m);
    return q.mode === "auto" ? "auto" : q.quality;
  }

  function chooseQuality(id: string, value: string) {
    overrides = {
      ...overrides,
      [id]: value === "auto" ? { mode: "auto" } : { mode: "pinned", quality: value as Quality },
    };
  }

  // Right-click menus: on a participant, and on a shared screen.
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  const menuMember = $derived(menu ? members.find((m) => m.id === menu!.id) : undefined);
  let screenMenu = $state<{ id: string; x: number; y: number } | null>(null);
  const screenMenuMember = $derived(screenMenu ? sharers.find((m) => m.id === screenMenu!.id) : undefined);

  function openScreenMenu(id: string, event: MouseEvent) {
    event.preventDefault();
    menu = null;
    screenMenu = { id, x: event.clientX, y: event.clientY };
  }

  function thumbKey(event: KeyboardEvent, id: string) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      chosen = id;
    }
  }

  const shareLabel = (m: MemberView) => (m.is_self ? "Your screen" : `${m.name}'s screen`);

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

  // Screen sharing: a whole screen or one window, picked from a list.
  let picking = $state<DisplayInfo[] | null>(null);
  let error = $state<string | null>(null);

  async function toggleShare() {
    error = null;
    if (snapshot.sharing) {
      await api.shareScreen(null);
      return;
    }
    await pickShare();
  }

  // Also how a share in progress moves to another screen or window: the
  // backend swaps the source and viewers keep watching.
  async function pickShare() {
    error = null;
    try {
      const targets = await api.displays();
      const only = targets.length === 1 ? targets[0] : undefined;
      if (only) {
        await api.shareScreen(only.id);
      } else if (targets.length === 0) {
        error = "Nothing to share was found.";
      } else {
        picking = targets;
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
        <svg class="chevron" class:open={showInvite} viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
          <path fill="currentColor" d="M7.4 8.6 12 13.2l4.6-4.6L18 10l-6 6-6-6 1.4-1.4Z" />
        </svg>
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
        <p>Lets one person straight in. Works once, within 30 minutes.</p>
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

  <div class="main">
    {#if focused}
      <div class="screens" class:with-others={others.length > 0}>
        <Stage
          member={focused}
          quality={qualityChoice(focused)}
          onquality={(value) => chooseQuality(focused.id, value)}
          sendingSound={snapshot.screen_audio}
          windowSound={snapshot.sharing_window && snapshot.window_audio_supported}
          onmenu={(e) => openScreenMenu(focused.id, e)}
        />
        {#if others.length}
          <div class="others" aria-label="Other shared screens">
            {#each others as m (m.id)}
              <div
                class="thumb"
                role="button"
                tabindex="0"
                aria-label={`Show ${shareLabel(m)} on the big stage`}
                onclick={() => (chosen = m.id)}
                onkeydown={(e) => thumbKey(e, m.id)}
                oncontextmenu={(e) => openScreenMenu(m.id, e)}
              >
                <div class="thumb-picture"><ScreenView id={m.id} label={shareLabel(m)} /></div>
                <span>{shareLabel(m)}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    <section class="people" aria-label="People in the room">
      {#each members as member (member.id)}
        <Orb
          {member}
          micOn={snapshot.mic_on}
          small={!!focused}
          focused={focused?.id === member.id}
          onselect={sharers.some((s) => s.id === member.id) ? () => (chosen = member.id) : null}
          onmenu={(e) => {
            screenMenu = null;
            menu = { id: member.id, x: e.clientX, y: e.clientY };
          }}
        />
      {/each}
    </section>
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <footer>
    <button
      class="mic"
      class:off={!snapshot.mic_on}
      aria-pressed={!snapshot.mic_on}
      aria-label={snapshot.mic_on ? "Turn mic off" : "Turn mic on"}
      title={snapshot.mic_on ? "Turn mic off" : "Turn mic on"}
      onclick={() => api.setMic(!snapshot.mic_on)}
    >
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
        {#if snapshot.mic_on}
          <path
            fill="currentColor"
            d="M12 14a3 3 0 0 0 3-3V5a3 3 0 1 0-6 0v6a3 3 0 0 0 3 3Zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.93V21h2v-3.07A7 7 0 0 0 19 11h-2Z"
          />
        {:else}
          <path
            fill="currentColor"
            d="M12 14a3 3 0 0 0 3-3V5a3 3 0 1 0-6 0v6a3 3 0 0 0 3 3Zm5-3a5 5 0 0 1-8.58 3.5l-1.42 1.42A7 7 0 0 0 11 17.93V21h2v-3.07A7 7 0 0 0 19 11h-2ZM3.7 2.3 2.3 3.7l18 18 1.4-1.4-18-18Z"
          />
        {/if}
      </svg>
    </button>
    <button
      class="mic"
      class:off={snapshot.deafened}
      aria-pressed={snapshot.deafened}
      aria-label={snapshot.deafened ? "Undeafen" : "Deafen (hear nobody)"}
      title={snapshot.deafened ? "Undeafen" : "Deafen (hear nobody)"}
      onclick={() => api.setDeafened(!snapshot.deafened)}
    >
      <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
        {#if snapshot.deafened}
          <path
            fill="currentColor"
            d="M12 3a9 9 0 0 0-9 9v7a2 2 0 0 0 2 2h3v-8H5v-1a7 7 0 0 1 11.6-5.27l1.42-1.42A8.96 8.96 0 0 0 12 3Zm7 5.83-1.65 1.65c.42.9.65 1.79.65 2.52v1h-3v8h3a2 2 0 0 0 2-2v-7c0-1.47-.36-2.86-1-4.17ZM3.7 2.3 2.3 3.7l18 18 1.4-1.4-18-18Z"
          />
        {:else}
          <path
            fill="currentColor"
            d="M12 3a9 9 0 0 0-9 9v7a2 2 0 0 0 2 2h3v-8H5v-1a7 7 0 0 1 14 0v1h-3v8h3a2 2 0 0 0 2-2v-7a9 9 0 0 0-9-9Z"
          />
        {/if}
      </svg>
    </button>
    <button class="button" aria-pressed={snapshot.sharing} onclick={toggleShare}>
      {snapshot.sharing ? "Stop sharing" : "Share screen"}
    </button>
    {#if snapshot.sharing}
      <button class="button quiet" onclick={pickShare}>Share something else</button>
    {/if}
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
    <label>
      <span class="visually-hidden">Pictures per second you send</span>
      <select
        class="field compact"
        value={snapshot.framerate}
        onchange={(e) => api.setFramerate(Number(e.currentTarget.value))}
      >
        <option value={24}>24 fps</option>
        <option value={30}>30 fps</option>
        <option value={60}>60 fps</option>
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

{#if screenMenu && screenMenuMember}
  {#key screenMenu}
    <ScreenMenu
      member={screenMenuMember}
      x={screenMenu.x}
      y={screenMenu.y}
      onStage={focused?.id === screenMenuMember.id}
      quality={qualityChoice(screenMenuMember)}
      onquality={(value) => chooseQuality(screenMenuMember.id, value)}
      onstage={() => (chosen = screenMenuMember.id)}
      onstop={() => api.shareScreen(null)}
      onclose={() => (screenMenu = null)}
    />
  {/key}
{/if}

{#if picking}
  <SharePicker {snapshot} targets={picking} onpick={share} onclose={() => (picking = null)} />
{/if}

{#if showAudio}
  <AudioSettings {snapshot} onclose={() => (showAudio = false)} />
{/if}

<style>
  /* A column whose only flexible part is .main: banners, the invite panel and
     errors come and go above and below it without shifting anything, and
     nothing inside .main can grow past the window. */
  .room {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 1.25rem 1.5rem 1rem;
    gap: 0.9rem;
    overflow: hidden;
  }

  .room > :global(*) {
    flex-shrink: 0;
  }

  .room > .main {
    flex: 1 1 0;
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr);
    gap: 0.9rem;
  }

  .staged > .main {
    grid-template-rows: minmax(0, 1fr) auto;
  }

  .screens {
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 0.9rem;
  }

  .screens.with-others {
    grid-template-columns: minmax(0, 1fr) clamp(9rem, 18vw, 15rem);
  }

  .others {
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .thumb {
    display: grid;
    gap: 0.35rem;
    flex-shrink: 0;
    cursor: pointer;
    border-radius: 10px;
  }

  .thumb-picture {
    position: relative;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border-radius: 10px;
    background: #000;
    border: 2px solid transparent;
  }

  .thumb:hover .thumb-picture {
    border-color: var(--lamp);
  }

  .thumb span {
    font-size: var(--step--1);
    color: var(--mist);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .chevron {
    margin-right: -0.3rem;
    transition: transform 150ms ease;
  }

  .chevron.open {
    transform: rotate(180deg);
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

  .error {
    margin: 0;
    color: var(--signal);
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
    padding-top: 0.9rem;
    border-top: 1px solid var(--dusk-raised);
  }

  .mic {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border: 0;
    border-radius: 50%;
    background: var(--dusk-raised);
  }

  .mic:hover {
    background: var(--dusk-hover);
  }

  .mic.off {
    background: var(--signal);
    color: var(--ink);
  }

  .mic.off:hover {
    background: #ee7a81;
  }

  .spacer {
    flex: 1;
  }

  .confirm {
    color: var(--mist);
  }

  .compact {
    width: auto;
    padding: 0.5rem 2.4rem 0.5rem 0.9rem;
    border-radius: 999px;
  }
</style>
