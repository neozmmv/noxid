<script lang="ts">
  import Avatar from "./Avatar.svelte";
  import IdChip from "./IdChip.svelte";
  import Menu from "./Menu.svelte";
  import NameField from "./NameField.svelte";
  import ProfilePicture from "./ProfilePicture.svelte";
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
  aria-label="Your profile"
  aria-expanded={!!open}
  use:tooltip={{ label: snapshot.name, side: "right" }}
  onpointerdown={(e) => {
    // Otherwise the card's click-outside closes it and this click reopens it.
    if (open) e.stopPropagation();
  }}
  onclick={toggle}
>
  <Avatar name={snapshot.name} hash={snapshot.avatar} size={36} />
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
      <div class="id">
        <span>Your Noxid ID</span>
        <IdChip id={snapshot.id} masked label="Noxid ID" />
        <small>Share it so people can add you as a friend. It's also your room code when you host.</small>
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

  .id span,
  .id small {
    color: var(--mist);
    font-size: var(--step--1);
  }
</style>
