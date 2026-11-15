<script lang="ts">
  import type { Status } from "./types";

  // A status as a dot: filled in its colour, or a hollow ring for invisible.
  let { status, size = 10 }: { status: Status; size?: number } = $props();
</script>

<span class={["dot", status]} style={`--size: ${size}px`} aria-hidden="true"></span>

<style>
  .dot {
    display: block;
    flex: none;
    width: var(--size);
    height: var(--size);
    border-radius: 50%;
    border: 2px solid var(--mist);
    box-sizing: border-box;
  }

  .online {
    border-color: var(--status-online);
    background: var(--status-online);
  }

  .away {
    border-color: var(--status-away);
    background: var(--status-away);
  }

  .busy {
    border-color: var(--signal);
    background: var(--signal);
  }
</style>
