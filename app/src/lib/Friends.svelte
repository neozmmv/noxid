<script lang="ts">
  import UserPlus from "@lucide/svelte/icons/user-plus";
  import Search from "@lucide/svelte/icons/search";
  import Users from "@lucide/svelte/icons/users";
  import PhoneMissed from "@lucide/svelte/icons/phone-missed";
  import { api } from "./api";
  import AddFriendDialog from "./AddFriendDialog.svelte";
  import { callFriend } from "./calls";
  import FriendRow from "./FriendRow.svelte";
  import { ago, friends } from "./friendlist.svelte";
  import IdChip from "./IdChip.svelte";
  import RequestRow from "./RequestRow.svelte";
  import type { FriendView, Snapshot } from "./types";

  let { snapshot }: { snapshot: Snapshot } = $props();

  let adding = $state(false);
  let query = $state("");

  const view = $derived(friends.view);
  const q = $derived(query.trim().toLowerCase());
  const matches = (item: { id: string; name: string; alias?: string | null }) =>
    !q || item.name.toLowerCase().includes(q) || item.id.startsWith(q) || !!item.alias?.toLowerCase().includes(q);

  const shown = $derived(view?.friends.filter(matches) ?? []);
  const groups = $derived(
    (
      [
        ["Online", shown.filter((f) => !f.finishing && f.presence === "online")],
        ["Offline", shown.filter((f) => !f.finishing && f.presence === "offline")],
        ["Status unknown", shown.filter((f) => f.finishing || f.presence === "unknown")],
      ] as [string, FriendView[]][]
    ).filter(([, list]) => list.length > 0),
  );
  const incoming = $derived(view?.incoming.filter(matches) ?? []);
  const outgoing = $derived(view?.outgoing.filter(matches) ?? []);
  const empty = $derived(!!view && view.friends.length + view.incoming.length + view.outgoing.length === 0);

  const callBlocked = $derived.by(() => {
    if (view?.calling) return "You're already calling someone";
    if (snapshot.room && !snapshot.room.is_coordinator) return "Only the host can call people into this room";
    return null;
  });
</script>

<div class="friends">
  <header>
    <h1>Friends</h1>
    <button class="button primary" onclick={() => (adding = true)} disabled={!!view?.problem}>
      <UserPlus size={18} strokeWidth={1.75} aria-hidden="true" />
      Add friend
    </button>
  </header>

  {#if view?.problem}
    <p class="problem" role="alert">{view.problem}</p>
  {/if}

  {#if !view}
    <p class="quiet">Loading…</p>
  {:else if empty}
    <section class="empty card">
      <span class="icon" aria-hidden="true"><Users size={28} strokeWidth={1.5} /></span>
      <h2>No friends yet</h2>
      <p>Add someone with their Noxid ID, or share yours so they can add you.</p>
      <IdChip id={snapshot.id} masked label="Noxid ID" />
      <button class="button primary" onclick={() => (adding = true)} disabled={!!view.problem}>Add friend</button>
    </section>
  {:else}
    <label class="search">
      <Search size={16} aria-hidden="true" />
      <span class="visually-hidden">Search friends</span>
      <input class="field" type="search" bind:value={query} placeholder="Search by name or ID" />
    </label>

    {#if view.missed.length > 0}
      <section aria-labelledby="missed-h">
        <div class="section-head">
          <h2 id="missed-h">Missed calls</h2>
          <button class="link" onclick={() => api.dismissMissedCalls()}>Clear</button>
        </div>
        <ul>
          {#each view.missed as call (call.id)}
            <li class="missed">
              <PhoneMissed size={16} aria-hidden="true" />
              <span>{call.name}</span>
              <small>{ago(call.at)}</small>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if incoming.length > 0}
      <section aria-labelledby="incoming-h">
        <h2 id="incoming-h">Requests <span class="badge">{incoming.length}</span></h2>
        <ul>
          {#each incoming as request (request.id)}
            <RequestRow kind="incoming" {request} />
          {/each}
        </ul>
      </section>
    {/if}

    {#if outgoing.length > 0}
      <section aria-labelledby="outgoing-h">
        <h2 id="outgoing-h">Sent</h2>
        <ul>
          {#each outgoing as request (request.id)}
            <RequestRow kind="outgoing" {request} />
          {/each}
        </ul>
      </section>
    {/if}

    {#each groups as [title, list] (title)}
      <section aria-label={title}>
        <h2>{title} · {list.length}</h2>
        <ul>
          {#each list as friend (friend.id)}
            <FriendRow {friend} {callBlocked} oncall={() => callFriend(friend.id, friend.name)} />
          {/each}
        </ul>
      </section>
    {/each}

    {#if q && groups.length === 0 && incoming.length === 0 && outgoing.length === 0}
      <p class="quiet">Nobody matches "{query.trim()}".</p>
    {/if}
  {/if}
</div>

{#if adding}
  <AddFriendDialog onclose={() => (adding = false)} />
{/if}

<style>
  .friends {
    max-width: 760px;
    margin: 0 auto;
    padding: 32px 24px 48px;
    display: grid;
    gap: 20px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  h1 {
    margin: 0;
    font-size: var(--step-3);
    letter-spacing: -0.02em;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 4px 12px;
    color: var(--mist);
    font-size: var(--step--1);
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 2px;
  }

  .search {
    position: relative;
    display: block;
    color: var(--mist);
  }

  .search :global(svg) {
    position: absolute;
    left: 12px;
    top: 50%;
    translate: 0 -50%;
    pointer-events: none;
  }

  .search input {
    padding-left: 36px;
  }

  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .link {
    padding: 0 12px;
    border: 0;
    background: none;
    color: var(--lamp);
    font-size: var(--step--1);
  }

  .missed {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    color: var(--moon);
  }

  .missed :global(svg) {
    color: var(--signal);
  }

  .missed small {
    color: var(--mist);
  }

  .problem {
    margin: 0;
    padding: 12px 16px;
    border-radius: var(--radius-card);
    border: 1px solid var(--signal);
    color: var(--moon);
  }

  .empty {
    display: grid;
    justify-items: center;
    gap: 12px;
    padding: 40px 24px;
    text-align: center;
  }

  .empty h2 {
    margin: 0;
    color: var(--moon);
    font-size: var(--step-1);
    text-transform: none;
    letter-spacing: 0;
  }

  .empty p {
    margin: 0;
    max-width: 40ch;
    color: var(--mist);
  }

  .empty .icon {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: var(--radius-card);
    background: var(--dusk-raised);
    color: var(--mist);
  }

  .quiet {
    color: var(--mist);
  }
</style>
