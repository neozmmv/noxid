<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./api";
  import type { AudioDevice, Snapshot } from "./types";

  // Which microphone we talk into, where everyone else is heard, and whether
  // a screen share carries the PC's sound. Choices are remembered.
  let { snapshot, onclose }: { snapshot: Snapshot; onclose: () => void } = $props();

  let inputs = $state<AudioDevice[]>([]);
  let outputs = $state<AudioDevice[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Windows can leave Noxid's own output out of the capture; a Linux sound
  // server monitor cannot, so the call's voices go out with the share there.
  const exactExclusion = navigator.userAgent.includes("Windows");

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
  onMount(() => dialog.querySelector<HTMLElement>("select")?.focus());
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={onclose}></div>
<div class="settings" role="dialog" aria-modal="true" aria-label="Audio settings" tabindex="-1" bind:this={dialog}>
  <h2>Audio</h2>

  <label>
    <span>Microphone</span>
    <select
      class="field"
      disabled={loading}
      value={snapshot.mic_device ?? DEFAULT}
      onchange={(e) => pick("input", e.currentTarget.value)}
    >
      <option value={DEFAULT}>{defaultName(inputs)}</option>
      {#each inputs as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </select>
  </label>

  <label>
    <span>Speakers or headphones</span>
    <select
      class="field"
      disabled={loading}
      value={snapshot.output_device ?? DEFAULT}
      onchange={(e) => pick("output", e.currentTarget.value)}
    >
      <option value={DEFAULT}>{defaultName(outputs)}</option>
      {#each outputs as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </select>
  </label>

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
          {#if exactExclusion}
            Everything your computer plays goes out with the share, except Noxid itself.
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
    background: rgb(10 12 28 / 0.55);
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
    border: 1px solid var(--dusk-raised);
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

  select.field {
    background: var(--ink);
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
