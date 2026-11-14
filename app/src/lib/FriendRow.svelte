<script lang="ts">
  import Phone from "@lucide/svelte/icons/phone";
  import Ellipsis from "@lucide/svelte/icons/ellipsis";
  import { api } from "./api";
  import Avatar from "./Avatar.svelte";
  import Confirm from "./Confirm.svelte";
  import IdChip from "./IdChip.svelte";
  import Menu from "./Menu.svelte";
  import Presence from "./Presence.svelte";
  import { tooltip } from "./tooltip";
  import { showToast } from "./toast.svelte";
  import type { FriendView } from "./types";

  interface Props {
    friend: FriendView;
    /** Why calling is not possible right now, or null when it is. */
    callBlocked: string | null;
    oncall: () => void;
  }

  let { friend, callBlocked, oncall }: Props = $props();

  let menu = $state<{ x: number; y: number } | null>(null);
  let confirming = $state(false);

  function openMenu(event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: rect.left - 200, y: rect.bottom + 4 };
  }

  async function remove() {
    confirming = false;
    try {
      await api.removeFriend(friend.id);
    } catch (e) {
      showToast(String(e));
    }
  }
</script>

<li class="row" oncontextmenu={(e) => { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }}>
  <Avatar name={friend.name} size={36} />
  <div class="who">
    <span class="name">{friend.name}</span>
    <div class="meta">
      <Presence {friend} />
      <IdChip id={friend.id} />
    </div>
    {#if friend.unaware}
      <span class="warn">They may not have you as a friend any more.</span>
    {/if}
  </div>
  <div class="actions">
    {#if !friend.finishing}
      <button
        class="icon call"
        aria-label={`Call ${friend.name}`}
        use:tooltip={callBlocked ?? "Call"}
        aria-disabled={callBlocked ? "true" : undefined}
        onclick={() => !callBlocked && oncall()}
      >
        <Phone size={18} strokeWidth={1.75} aria-hidden="true" />
      </button>
    {/if}
    <button class="icon" aria-label={`More for ${friend.name}`} use:tooltip={"More"} onclick={openMenu}>
      <Ellipsis size={18} strokeWidth={1.75} aria-hidden="true" />
    </button>
  </div>
</li>

{#if menu}
  <Menu label={`Actions for ${friend.name}`} x={menu.x} y={menu.y} onclose={() => (menu = null)}>
    <p class="who">{friend.name}</p>
    <button
      class="item"
      onclick={() => {
        menu = null;
        void navigator.clipboard.writeText(friend.id).then(() => showToast("Noxid ID copied.", 2000));
      }}>Copy Noxid ID</button
    >
    <button
      class="item danger"
      onclick={() => {
        menu = null;
        confirming = true;
      }}>Remove friend</button
    >
  </Menu>
{/if}

{#if confirming}
  <Confirm
    question={`Remove ${friend.name}?`}
    detail="You'll stop being friends on both sides as soon as they're told. To be friends again, one of you sends a new request."
    confirm="Remove"
    cancel="Keep"
    onconfirm={remove}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border-radius: var(--radius-card);
  }

  .row:hover {
    background: var(--surface);
  }

  .who {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 4px;
  }

  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 12px;
  }

  .warn {
    color: var(--mist);
    font-size: var(--step--1);
    font-style: italic;
  }

  .actions {
    display: flex;
    gap: 4px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--mist);
  }

  .icon:hover:not([aria-disabled]) {
    background: var(--dusk-raised);
    color: var(--moon);
  }

  .icon.call:hover:not([aria-disabled]) {
    color: var(--lamp);
  }

  .icon[aria-disabled] {
    opacity: 0.4;
    cursor: default;
  }
</style>
