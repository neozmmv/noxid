<script lang="ts">
  import { ago } from "./friendlist.svelte";
  import type { FriendView } from "./types";

  // A dot and a word for whether a friend can be reached. Only ever what the
  // backend reported: "unknown" is said as such, never guessed.
  let { friend }: { friend: FriendView } = $props();

  // Online friends show the status they chose instead.
  const shown = $derived(
    friend.finishing ? "unknown" : friend.presence === "online" ? friend.status : friend.presence,
  );

  const label = $derived.by(() => {
    if (friend.finishing) return "Finishing…";
    switch (friend.presence) {
      case "online":
        return friend.status === "away" ? "Away" : friend.status === "busy" ? "Do not disturb" : "Online";
      case "offline":
        return friend.last_seen ? `Offline · seen ${ago(friend.last_seen)}` : "Offline";
      default:
        return "Status unknown";
    }
  });
</script>

<span class={["presence", shown]}>
  <span class="dot" aria-hidden="true"></span>
  {label}
</span>

<style>
  .presence {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--mist);
    font-size: var(--step--1);
    white-space: nowrap;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--mist);
  }

  .online .dot {
    border-color: var(--status-online);
    background: var(--status-online);
  }

  .away .dot {
    border-color: var(--status-away);
    background: var(--status-away);
  }

  .busy .dot {
    border-color: var(--signal);
    background: var(--signal);
  }

  .offline .dot {
    border-color: var(--line);
    background: var(--line);
  }

  .unknown .dot {
    border-style: dashed;
  }
</style>
