<script lang="ts">
  import { api } from "./api";
  import Avatar from "./Avatar.svelte";
  import { ago } from "./friendlist.svelte";
  import IdChip from "./IdChip.svelte";
  import type { IncomingView, OutgoingView } from "./types";

  // A pending friend request: one someone sent us (accept or decline), or
  // one we sent (where it stands, and cancel).
  type Props =
    | { kind: "incoming"; request: IncomingView }
    | { kind: "outgoing"; request: OutgoingView };

  let props: Props = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);

  async function run(action: () => Promise<void>) {
    busy = true;
    error = null;
    try {
      await action();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const status = $derived.by(() => {
    if (props.kind !== "outgoing") return "";
    const r = props.request;
    if (r.delivered_at) return "Delivered · waiting for them to accept";
    if (r.stalled) return "Not delivered · no longer retrying";
    if (r.last_error) return "Not delivered yet · will retry when they're online";
    return "Sending…";
  });
</script>

<li class="row">
  <Avatar name={props.request.name} size={36} />
  <div class="who">
    {#if props.kind === "incoming"}
      <span class="name">{props.request.name} <small>sent with the request</small></span>
    {:else}
      <span class="name">{props.request.name}</span>
    {/if}
    <IdChip id={props.request.id} />
    {#if props.kind === "incoming"}
      <span class="meta">Received {ago(props.request.received_at)}</span>
    {:else}
      <span class="meta" class:waiting={!props.request.delivered_at}>{status}</span>
    {/if}
    {#if error}<span class="error" role="alert">{error}</span>{/if}
  </div>
  <div class="actions">
    {#if props.kind === "incoming"}
      {@const id = props.request.id}
      <button class="button quiet" disabled={busy} onclick={() => run(() => api.answerFriendRequest(id, false))}>
        Decline
      </button>
      <button class="button primary" disabled={busy} onclick={() => run(() => api.answerFriendRequest(id, true))}>
        Accept
      </button>
    {:else}
      {@const id = props.request.id}
      {#if !props.request.delivered_at && (props.request.stalled || props.request.last_error)}
        <button class="button quiet" disabled={busy} onclick={() => run(() => api.retryFriendRequest(id))}>
          Retry now
        </button>
      {/if}
      <button class="button quiet" disabled={busy} onclick={() => run(() => api.cancelFriendRequest(id))}>
        Cancel request
      </button>
    {/if}
  </div>
</li>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border-radius: var(--radius-card);
  }

  .row:hover {
    background: var(--dusk);
  }

  .who {
    flex: 1;
    min-width: 0;
    display: grid;
    justify-items: start;
    gap: 4px;
  }

  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .name small,
  .meta {
    color: var(--mist);
    font-size: var(--step--1);
    font-weight: 400;
  }

  .meta.waiting {
    color: var(--mist);
  }

  .error {
    color: var(--signal);
    font-size: var(--step--1);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
