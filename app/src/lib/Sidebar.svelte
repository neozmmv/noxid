<script lang="ts">
  import { onMount } from "svelte";
  import House from "@lucide/svelte/icons/house";
  import Users from "@lucide/svelte/icons/users";
  import Server from "@lucide/svelte/icons/server";
  import Plus from "@lucide/svelte/icons/plus";
  import Settings from "@lucide/svelte/icons/settings";
  import { go, nav } from "./nav.svelte";
  import ProfileButton from "./ProfileButton.svelte";
  import RailButton from "./RailButton.svelte";
  import type { Snapshot } from "./types";

  // The navigation rail: always there, beside whatever view is showing,
  // including a call. Up and Down move between its buttons; Tab leaves it.
  interface Props {
    snapshot: Snapshot;
    /** Friend requests waiting for an answer, for the badge. */
    requests?: number;
  }

  let { snapshot, requests = 0 }: Props = $props();

  const ICON = { size: 20, strokeWidth: 1.75, "aria-hidden": true } as const;

  let rail: HTMLElement;

  function keydown(event: KeyboardEvent) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const buttons = [...rail.querySelectorAll<HTMLElement>("button")];
    const at = buttons.indexOf(document.activeElement as HTMLElement);
    if (at < 0) return;
    event.preventDefault();
    const step = event.key === "ArrowDown" ? 1 : -1;
    buttons[(at + step + buttons.length) % buttons.length]?.focus();
  }

  // Added by hand: a nav landmark takes no key handlers in markup, but arrow
  // keys between its buttons are what a vertical rail is expected to do.
  onMount(() => {
    rail.addEventListener("keydown", keydown);
    return () => rail.removeEventListener("keydown", keydown);
  });
</script>

<nav class="rail" aria-label="Main" bind:this={rail}>
  <RailButton
    label={snapshot.room ? "Home: in a call" : "Home"}
    active={nav.view === "home"}
    dot={!!snapshot.room}
    onclick={() => go("home")}
  >
    <House {...ICON} />
  </RailButton>
  <RailButton label="Friends" active={nav.view === "friends"} badge={requests} onclick={() => go("friends")}>
    <Users {...ICON} />
  </RailButton>

  <hr />

  <RailButton label="Servers" active={nav.view === "servers"} onclick={() => go("servers")}>
    <Server {...ICON} />
  </RailButton>
  <RailButton label="Add a server (coming soon)" disabled>
    <Plus {...ICON} />
  </RailButton>

  <div class="spacer"></div>

  <RailButton label="Settings" active={nav.view === "settings"} onclick={() => go("settings")}>
    <Settings {...ICON} />
  </RailButton>
  <ProfileButton {snapshot} />
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    width: var(--rail-w);
    height: 100%;
    padding: 12px 0;
    border-right: 1px solid var(--line);
    background: var(--ink);
    overflow-y: auto;
    overflow-x: hidden;
  }

  hr {
    width: 28px;
    margin: 4px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }

  .spacer {
    flex: 1;
    min-height: 8px;
  }
</style>
