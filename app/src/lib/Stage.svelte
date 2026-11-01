<script lang="ts">
  import { api } from "./api";
  import { setFullscreen as goFullscreen, view } from "./fullscreen.svelte";
  import ScreenView from "./ScreenView.svelte";
  import type { MemberView, Quality } from "./types";

  // The shared screen on the big stage, with its own controls: fullscreen, the
  // sound that comes with it, and the picture quality.
  interface Props {
    member: MemberView;
    quality: "auto" | Quality;
    onquality: (value: string) => void;
    /** For our own share: whether the PC's sound goes out with it. */
    sendingSound: boolean;
    /** Right-click on the picture. */
    onmenu: (event: MouseEvent) => void;
  }

  let { member, quality, onquality, sendingSound, onmenu }: Props = $props();

  const fullscreen = $derived(view.fullscreen);
  // In fullscreen the controls get out of the way until the pointer moves.
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  function setFullscreen(on: boolean) {
    goFullscreen(on);
  }

  // Coming out of hiding whenever fullscreen changes, whoever changed it.
  $effect(() => {
    void view.fullscreen;
    wake();
  });

  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    if (fullscreen) idleTimer = setTimeout(() => (idle = true), 2500);
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape" && fullscreen) setFullscreen(false);
    if (event.key === "f" && !(event.target instanceof HTMLInputElement)) setFullscreen(!fullscreen);
  }

  // Leaving fullscreen when the share goes away (or the stage unmounts), so
  // the app is never stuck in it.
  $effect(() => () => {
    clearTimeout(idleTimer);
    if (view.fullscreen) goFullscreen(false);
  });

  const percent = $derived(Math.round(member.screen_volume * 100));

  function setSound(volume: number, muted: boolean) {
    api.setPeerAudio(member.id, "screen", volume, muted);
  }
</script>

<svelte:window onkeydown={keydown} />

<section
  class="stage"
  class:fullscreen
  class:idle
  onpointermove={wake}
  ondblclick={() => setFullscreen(!fullscreen)}
  oncontextmenu={(e) => {
    e.preventDefault();
    onmenu(e);
  }}
  aria-label={member.is_self ? "Your screen" : `${member.name}'s screen`}
>
  <div class="picture">
    {#key member.id}
      <ScreenView id={member.id} label={member.is_self ? "Your screen" : `${member.name}'s screen`} />
    {/key}
  </div>

  <div class="bar">
    <span class="title">{member.is_self ? "Your screen" : `${member.name}'s screen`}</span>

    {#if member.is_self}
      <span class="meta">{sendingSound ? "With your PC's sound" : "Without sound"}</span>
    {:else}
      <span class="meta">
        {#if member.width}{member.width}×{member.height}, {member.fps} fps{/if}
      </span>

      {#if member.has_screen_audio}
        <button
          class="icon"
          aria-pressed={member.screen_muted}
          aria-label={member.screen_muted ? "Unmute the screen's sound" : "Mute the screen's sound"}
          onclick={() => setSound(member.screen_volume, !member.screen_muted)}
          ondblclick={(e) => e.stopPropagation()}
        >
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
            {#if member.screen_muted}
              <path fill="currentColor" d="M3 9v6h4l5 5V4L7 9H3Zm13.6 3 2.7-2.7-1.4-1.4-2.7 2.7-2.7-2.7-1.4 1.4 2.7 2.7-2.7 2.7 1.4 1.4 2.7-2.7 2.7 2.7 1.4-1.4-2.7-2.7Z" />
            {:else}
              <path fill="currentColor" d="M3 9v6h4l5 5V4L7 9H3Zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4ZM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6Z" />
            {/if}
          </svg>
        </button>
        <label class="volume" ondblclick={(e) => e.stopPropagation()}>
          <span class="visually-hidden">Screen sound volume</span>
          <input
            type="range"
            min="0"
            max="100"
            step="5"
            value={percent}
            disabled={member.screen_muted}
            oninput={(e) => setSound(Number(e.currentTarget.value) / 100, false)}
          />
          <output>{percent}%</output>
        </label>
      {:else}
        <span class="meta">No sound</span>
      {/if}

      <label ondblclick={(e) => e.stopPropagation()}>
        <span class="visually-hidden">Picture quality</span>
        <select class="field compact" value={quality} onchange={(e) => onquality(e.currentTarget.value)}>
          <option value="auto">Quality: automatic</option>
          <option value="high">Quality: high</option>
          <option value="mid">Quality: medium</option>
          <option value="low">Quality: low</option>
        </select>
      </label>
    {/if}

    <button
      class="icon"
      aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
      title={fullscreen ? "Exit fullscreen (Esc)" : "Fullscreen (F)"}
      onclick={() => setFullscreen(!fullscreen)}
      ondblclick={(e) => e.stopPropagation()}
    >
      <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
        {#if fullscreen}
          <path fill="currentColor" d="M8 3v3a2 2 0 0 1-2 2H3v2h3a4 4 0 0 0 4-4V3H8Zm8 0h-2v3a4 4 0 0 0 4 4h3V8h-3a2 2 0 0 1-2-2V3ZM3 16h3a2 2 0 0 1 2 2v3h2v-3a4 4 0 0 0-4-4H3v2Zm15-2a4 4 0 0 0-4 4v3h2v-3a2 2 0 0 1 2-2h3v-2h-3Z" />
        {:else}
          <path fill="currentColor" d="M3 3h6v2H5v4H3V3Zm12 0h6v6h-2V5h-4V3ZM3 15h2v4h4v2H3v-6Zm16 0h2v6h-6v-2h4v-4Z" />
        {/if}
      </svg>
    </button>
  </div>
</section>

<style>
  .stage {
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: 0.5rem;
  }

  .picture {
    position: relative;
    min-height: 0;
    overflow: hidden;
    border-radius: 10px;
    background: #000;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 0.8rem;
  }

  .title {
    font-weight: 600;
  }

  .meta {
    color: var(--mist);
    flex: 1;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border: 0;
    border-radius: 50%;
    background: var(--dusk-raised);
  }

  .icon:hover {
    background: #3a416f;
  }

  .icon[aria-pressed="true"] {
    background: var(--signal);
    color: var(--ink);
  }

  .volume {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .volume input {
    width: 110px;
    accent-color: var(--lamp);
  }

  output {
    min-width: 3.2em;
    color: var(--mist);
    font-variant-numeric: tabular-nums;
  }

  .compact {
    width: auto;
    padding: 0.5rem 2.4rem 0.5rem 0.9rem;
    border-radius: 999px;
  }

  /* Fullscreen: the picture takes everything, the bar floats over its foot. */
  .fullscreen {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: #000;
    grid-template-rows: 1fr;
    gap: 0;
  }

  .fullscreen .picture {
    grid-row: 1;
    grid-column: 1;
    border-radius: 0;
  }

  .fullscreen .bar {
    position: relative;
    z-index: 1;
    grid-row: 1;
    grid-column: 1;
    align-self: end;
    margin: 1rem;
    padding: 0.5rem 0.75rem;
    border-radius: 14px;
    background: rgb(27 31 59 / 0.85);
    transition: opacity 200ms ease;
  }

  .fullscreen.idle {
    cursor: none;
  }

  .fullscreen.idle .bar {
    opacity: 0;
  }
</style>
