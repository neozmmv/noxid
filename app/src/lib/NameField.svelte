<script lang="ts">
  import { untrack } from "svelte";
  import { api } from "./api";
  import type { Snapshot } from "./types";

  // The display name, saved when the field is left or Enter is pressed. A
  // label for humans and nothing more: identity is the endpoint id.
  interface Props {
    snapshot: Snapshot;
    label?: string;
    autofocus?: boolean;
  }

  let { snapshot, label = "Your name", autofocus = false }: Props = $props();

  // Seeded once from the snapshot; after that the field is the user's.
  let name = $state(untrack(() => snapshot.name));
  let error = $state<string | null>(null);

  async function save() {
    const next = name.trim();
    if (!next || next === snapshot.name) return;
    error = null;
    try {
      await api.setName(next);
    } catch (e) {
      error = String(e);
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      void save();
    }
  }
</script>

<label class="name">
  <span>{label}</span>
  <!-- svelte-ignore a11y_autofocus -->
  <input
    class="field"
    bind:value={name}
    onblur={save}
    onkeydown={keydown}
    placeholder="How others see you"
    maxlength="32"
    {autofocus}
  />
  {#if error}
    <small class="error" role="alert">{error}</small>
  {/if}
</label>

<style>
  .name {
    display: grid;
    gap: 6px;
  }

  .name span {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .error {
    color: var(--signal);
  }
</style>
