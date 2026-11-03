<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api } from "./api";
  import { METER_FLOOR_DB, prefs, setSensitivity, toDb, voice } from "./levels.svelte";
  import Select from "./Select.svelte";
  import type { AudioDevice, Snapshot } from "./types";

  // Which microphone we talk into, where everyone else is heard, and whether
  // a screen share carries the PC's sound. Choices are remembered.
  let { snapshot, onclose }: { snapshot: Snapshot; onclose: () => void } = $props();

  let inputs = $state<AudioDevice[]>([]);
  let outputs = $state<AudioDevice[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // `""` in a select stands for the system default.
  const DEFAULT = "";

  async function load() {
    loading = true;
    try {
      const devices = await api.audioDevices();
      inputs = devices.inputs;
      outputs = devices.outputs;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(load);

  // Keep the mic meter running while this is open, even outside a call, so
  // the sensitivity can be set against a live level.
  onMount(() => {
    api.setMicTest(true);
    return () => {
      api.setMicTest(false);
    };
  });

  // The meter falls back slowly, so a peak stays readable for a moment.
  let shownDb = $state(METER_FLOOR_DB);
  $effect(() => {
    const db = toDb(voice.levels[snapshot.id] ?? 0);
    const previous = untrack(() => shownDb);
    shownDb = db > previous ? db : Math.max(db, previous - 6);
  });
  const toPercent = (db: number) => ((db - METER_FLOOR_DB) / -METER_FLOOR_DB) * 100;
  const speaking = $derived(shownDb > prefs.sensitivityDb);

  async function pick(kind: "input" | "output", value: string) {
    error = null;
    const device = value === DEFAULT ? null : value;
    try {
      if (kind === "input") await api.setMicDevice(device);
      else await api.setOutputDevice(device);
    } catch (e) {
      error = String(e);
    }
  }

  function defaultName(list: AudioDevice[]) {
    const d = list.find((d) => d.default);
    return d ? `System default (${d.name})` : "System default";
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }

  let dialog: HTMLElement;
  // The dialog itself takes focus, so keyboard users land inside it without
  // a ring appearing on the first control before they have pressed a key.
  onMount(() => dialog.focus());
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="settings" role="dialog" aria-modal="true" aria-label="Audio settings" tabindex="-1" bind:this={dialog}>
  <h2>Audio</h2>

  <label>
    <span>Microphone</span>
    <Select
      disabled={loading}
      value={snapshot.mic_device ?? DEFAULT}
      onchange={(e) => pick("input", e.currentTarget.value)}
    >
      <option value={DEFAULT}>{defaultName(inputs)}</option>
      {#each inputs as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </Select>
  </label>

  <label>
    <span>Speakers or headphones</span>
    <Select
      disabled={loading}
      value={snapshot.output_device ?? DEFAULT}
      onchange={(e) => pick("output", e.currentTarget.value)}
    >
      <option value={DEFAULT}>{defaultName(outputs)}</option>
      {#each outputs as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </Select>
  </label>

  <div class="sensitivity">
    <label for="sensitivity">
      <span>Input sensitivity</span>
      <output>{prefs.sensitivityDb} dB</output>
    </label>
    <div class="meter" class:speaking aria-hidden="true">
      <div class="fill" style={`width: ${toPercent(shownDb)}%`}></div>
      <div class="mark" style={`left: ${toPercent(prefs.sensitivityDb)}%`}></div>
    </div>
    <input
      id="sensitivity"
      type="range"
      min={METER_FLOOR_DB}
      max="-5"
      step="1"
      value={prefs.sensitivityDb}
      oninput={(e) => setSensitivity(Number(e.currentTarget.value))}
    />
    <small>
      {#if !snapshot.mic_on}
        Your mic is off, so the meter is still.
      {:else}
        Talk normally: the bar should pass the line while you speak and stay below it when you are
        quiet. Move the slider left if your ring does not light up.
      {/if}
    </small>
  </div>

  {#if snapshot.screen_audio_supported}
    <label class="check">
      <input
        type="checkbox"
        checked={snapshot.screen_audio}
        onchange={(e) => api.setScreenAudio(e.currentTarget.checked)}
      />
      <span>
        Share the PC's sound with my screen
        <small>
          <!-- Windows can capture one app alone and leave Noxid out; a Linux
               sound server's monitor cannot, so the call's voices go out too. -->
          {#if snapshot.window_audio_supported}
            A whole screen sends everything your computer plays, except Noxid. One window sends only
            that app's sound.
          {:else}
            Everything your computer plays goes out with the share, including this call's voices, so
            others may hear themselves. Use headphones while sharing sound.
          {/if}
        </small>
      </span>
    </label>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <button class="button quiet" onclick={load} disabled={loading}>Refresh devices</button>
    <button class="button primary" onclick={onclose}>Done</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .settings {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(440px, calc(100vw - 2rem));
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    outline: none;
    display: grid;
    gap: 1rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-2);
  }

  label {
    display: grid;
    gap: 0.4rem;
  }

  label > span {
    color: var(--mist);
  }

  .settings :global(select.field) {
    width: 100%;
    background-color: var(--dusk-raised);
  }

  .sensitivity {
    display: grid;
    gap: 0.45rem;
  }

  .sensitivity label {
    display: flex;
    justify-content: space-between;
  }

  .sensitivity output {
    color: var(--moon);
    font-variant-numeric: tabular-nums;
  }

  .meter {
    position: relative;
    height: 10px;
    border-radius: 999px;
    background: var(--ink);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--mist);
    transition: width 90ms linear;
  }

  .meter.speaking .fill {
    background: var(--lamp);
  }

  .mark {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--moon);
  }

  .sensitivity input[type="range"] {
    width: 100%;
    accent-color: var(--lamp);
  }

  .check {
    grid-template-columns: auto 1fr;
    align-items: start;
    gap: 0.6rem;
  }

  .check input {
    margin-top: 0.25rem;
    accent-color: var(--lamp);
    width: 1rem;
    height: 1rem;
  }

  .check span {
    color: var(--moon);
    display: grid;
    gap: 0.2rem;
  }

  small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .error {
    margin: 0;
    color: var(--signal);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
  }
</style>
