<script lang="ts">
  import { api } from "./api";
  import { setFullscreen, view } from "./fullscreen.svelte";
  import Menu from "./Menu.svelte";
  import type { MemberView, Quality } from "./types";

  // The right-click menu on a shared screen: the picture (stage, fullscreen,
  // quality) and the sound that comes with it.
  interface Props {
    member: MemberView;
    x: number;
    y: number;
    /** Whether this share is the one on the big stage. */
    onStage: boolean;
    /** What they send: a screen, a camera, or both. */
    hasScreen: boolean;
    hasCamera: boolean;
    quality: "auto" | Quality;
    onquality: (value: string) => void;
    onstage: () => void;
    onstop: () => void;
    /** Our own camera only: turns it off. */
    oncameraoff: () => void;
    onclose: () => void;
  }

  let {
    member,
    x,
    y,
    onStage,
    hasScreen,
    hasCamera,
    quality,
    onquality,
    onstage,
    onstop,
    oncameraoff,
    onclose,
  }: Props = $props();

  const title = $derived(`${member.is_self ? "Your" : `${member.name}'s`} ${hasScreen ? "screen" : "camera"}`);
  const percent = $derived(Math.round(member.screen_volume * 100));

  function fullscreen() {
    if (!onStage) onstage();
    setFullscreen(!(onStage && view.fullscreen));
    onclose();
  }

  function setSound(volume: number, muted: boolean) {
    api.setPeerAudio(member.id, "screen", volume, muted);
  }

  const qualities: { value: "auto" | Quality; label: string }[] = [
    { value: "auto", label: "Auto" },
    { value: "high", label: "High" },
    { value: "mid", label: "Medium" },
    { value: "low", label: "Low" },
  ];
</script>

<Menu label={title} {x} {y} {onclose}>
  <p class="who">{title}</p>

  {#if !onStage}
    <button class="item" onclick={() => (onstage(), onclose())}>Show on the big stage</button>
  {/if}
  <button class="item" onclick={fullscreen}>
    {onStage && view.fullscreen ? "Exit fullscreen" : "Fullscreen"}
  </button>

  {#if member.is_self}
    <hr />
    {#if hasScreen}
      <p class="note">The sound you share is not played back here, so it has no volume of its own.</p>
      <button class="item danger" onclick={() => (onstop(), onclose())}>Stop sharing</button>
    {/if}
    {#if hasCamera}
      <button class="item danger" onclick={() => (oncameraoff(), onclose())}>Turn camera off</button>
    {/if}
  {:else}
    {#if member.has_screen_audio}
      <hr />
      <label class="row">
        <input
          type="checkbox"
          checked={member.screen_muted}
          onchange={(e) => setSound(member.screen_volume, e.currentTarget.checked)}
        />
        Mute the screen's sound
      </label>
      <label class="slider">
        <span>Sound volume <output>{percent}%</output></span>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={percent}
          disabled={member.screen_muted}
          oninput={(e) => setSound(Number(e.currentTarget.value) / 100, false)}
        />
      </label>
    {/if}

    {#if hasScreen}
      <hr />
      <div class="quality" role="radiogroup" aria-label="Picture quality">
        <span>Picture quality</span>
        <div class="choices">
          {#each qualities as q (q.value)}
            <button
              role="radio"
              aria-checked={quality === q.value}
              class:on={quality === q.value}
              onclick={() => onquality(q.value)}
            >
              {q.label}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</Menu>

<style>
  .note {
    margin: 0;
    padding: 0.35rem 0.5rem;
    color: var(--mist);
    font-size: var(--step--1);
  }

  .quality {
    display: grid;
    gap: 0.4rem;
    padding: 0.35rem 0.5rem 0.4rem;
  }

  .quality > span {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .choices {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 2px;
    padding: 2px;
    border-radius: 9px;
    background: var(--ink);
  }

  .choices button {
    padding: 0.35rem 0;
    border: 0;
    border-radius: 7px;
    background: none;
    font-size: var(--step--1);
  }

  .choices button:hover {
    background: var(--dusk-raised);
  }

  .choices button.on {
    background: var(--lamp);
    color: var(--ink);
    font-weight: 650;
  }
</style>
