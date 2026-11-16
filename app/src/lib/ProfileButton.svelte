<script lang="ts">
  import Avatar from "./Avatar.svelte";
  import IdChip from "./IdChip.svelte";
  import Menu from "./Menu.svelte";
  import NameField from "./NameField.svelte";
  import ProfilePicture from "./ProfilePicture.svelte";
  import { statusLabel } from "./status";
  import StatusDot from "./StatusDot.svelte";
  import StatusPicker from "./StatusPicker.svelte";
  import { go } from "./nav.svelte";
  import { tooltip } from "./tooltip";
  import type { Snapshot } from "./types";

  // Who we are, at the foot of the rail: the picture opens a card with the
  // name, the picture and our Noxid ID. The secret key never reaches the
  // webview, so there is nothing secret here to show by mistake.
  let { snapshot }: { snapshot: Snapshot } = $props();

  let button: HTMLButtonElement;
  let open = $state<{ x: number; y: number } | null>(null);
  let showPicture = $state(false);

  function toggle() {
    if (open) {
      open = null;
      return;
    }
    const rect = button.getBoundingClientRect();
    // Beside the rail, bottom-aligned with the button; Menu keeps it inside
    // the window.
    open = { x: rect.right + 12, y: rect.bottom };
  }
</script>

<button
  class="profile"
  bind:this={button}
  aria-label={`Your profile, ${statusLabel(snapshot.status)}`}
  aria-expanded={!!open}
  use:tooltip={{ label: snapshot.name, side: "right" }}
  onpointerdown={(e) => {
    // Otherwise the card's click-outside closes it and this click reopens it.
    if (open) e.stopPropagation();
  }}
  onclick={toggle}
>
  <Avatar name={snapshot.name} hash={snapshot.avatar} size={36} />
  <span class="badge"><StatusDot status={snapshot.status} size={12} /></span>
</button>

{#if open}
  <Menu label="Your profile" x={open.x} y={open.y} onclose={() => (open = null)}>
    <div class="card-body">
      <div class="who-row">
        <Avatar name={snapshot.name} hash={snapshot.avatar} size={44} />
        <div class="who-text">
          <strong>{snapshot.name}</strong>
          <button
            class="link"
            onclick={() => {
              open = null;
              showPicture = true;
            }}>Change picture</button
          >
        </div>
      </div>
      <NameField {snapshot} />
      <div class="status">
        <span id="profile-status">Status</span>
        <StatusPicker status={snapshot.status} labelledby="profile-status" />
      </div>
      <div class="id">
        <span>Your Noxid ID</span>
        <IdChip id={snapshot.id} masked label="Noxid ID" />
      </div>
      <button
        class="button quiet"
        onclick={() => {
          open = null;
          go("settings");
        }}>Settings</button
      >
    </div>
  </Menu>
{/if}

{#if showPicture}
  <ProfilePicture {snapshot} onclose={() => (showPicture = false)} />
{/if}

<style>
  .profile {
    position: relative;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
  }

  .profile:hover :global(.avatar) {
    box-shadow: 0 0 0 2px var(--line);
  }

  /* The status on the picture's lower right, ringed in the rail's colour so
     it reads as cut out of the picture. */
  .badge {
    position: absolute;
    right: 3px;
    bottom: 3px;
    padding: 2px;
    border-radius: 50%;
    background: var(--ink);
    pointer-events: none;
  }

  .status {
    display: grid;
    gap: 4px;
  }

  .status > span {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .card-body {
    display: grid;
    gap: 14px;
    padding: 6px;
  }

  .who-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .who-text {
    display: grid;
    min-width: 0;
  }

  .who-text strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .link {
    justify-self: start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--lamp);
    font-size: var(--step--1);
  }

  .link:hover {
    text-decoration: underline;
  }

  .id {
    display: grid;
    gap: 6px;
  }

  .id span {
    color: var(--mist);
    font-size: var(--step--1);
  }
</style>
