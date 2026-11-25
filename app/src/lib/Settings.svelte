<script lang="ts">
  import AccentPicker from "./AccentPicker.svelte";
  import AudioSettingsPanel from "./AudioSettingsPanel.svelte";
  import Avatar from "./Avatar.svelte";
  import IdChip from "./IdChip.svelte";
  import NameField from "./NameField.svelte";
  import ProfilePicture from "./ProfilePicture.svelte";
  import RelaySettings from "./RelaySettings.svelte";
  import StatusPicker from "./StatusPicker.svelte";
  import ThemePicker from "./ThemePicker.svelte";
  import type { Snapshot } from "./types";

  let { snapshot }: { snapshot: Snapshot } = $props();

  let showPicture = $state(false);
</script>

<div class="settings">
  <h1>Settings</h1>

  <section class="card">
    <h2>Profile</h2>
    <div class="profile">
      <Avatar name={snapshot.name} hash={snapshot.avatar} size={56} />
      <button class="button quiet" onclick={() => (showPicture = true)}>Change picture</button>
    </div>
    <NameField {snapshot} />
    <div class="id">
      <span>Your Noxid ID</span>
      <IdChip id={snapshot.friend_code} masked label="Noxid ID" />
      <small>Share it so people can add you as a friend. It's also your room code when you host.</small>
    </div>
  </section>

  <section class="card">
    <h2 id="status-heading">Status</h2>
    <StatusPicker status={snapshot.status} labelledby="status-heading" />
  </section>

  <section class="card appearance">
    <h2>Appearance</h2>
    <div class="group">
      <span id="theme-heading">Theme</span>
      <ThemePicker labelledby="theme-heading" />
    </div>
    <div class="group">
      <span id="accent-heading">Accent color</span>
      <AccentPicker labelledby="accent-heading" />
    </div>
  </section>

  <section class="card">
    <h2>Audio</h2>
    <AudioSettingsPanel {snapshot} />
  </section>

  <section class="card">
    <h2>Custom relay</h2>
    <RelaySettings {snapshot} />
  </section>
</div>

{#if showPicture}
  <ProfilePicture {snapshot} onclose={() => (showPicture = false)} />
{/if}

<style>
  .settings {
    max-width: 640px;
    margin: 0 auto;
    padding: 32px 24px 48px;
    display: grid;
    gap: 16px;
  }

  h1 {
    margin: 0 0 8px;
    font-size: var(--step-3);
    letter-spacing: -0.02em;
  }

  section {
    display: grid;
    gap: 16px;
  }

  h2 {
    margin: 0;
    font-size: var(--step-1);
  }

  .profile {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .id {
    display: grid;
    gap: 6px;
    justify-items: start;
  }

  .id span,
  .id small {
    color: var(--mist);
    font-size: var(--step--1);
  }

  /* Two choices of equal weight under one heading: each gets the same small
     label, set closer to its own options (8) than to the group above (24). */
  .appearance {
    gap: 24px;
  }

  .appearance h2 {
    margin-bottom: -8px;
  }

  .group {
    display: grid;
    gap: 8px;
  }

  .group > span {
    color: var(--mist);
    font-size: var(--step--1);
  }

</style>
