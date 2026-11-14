<script lang="ts">
  import { view as screen } from "./fullscreen.svelte";
  import Friends from "./Friends.svelte";
  import { friends } from "./friendlist.svelte";
  import Home from "./Home.svelte";
  import { nav } from "./nav.svelte";
  import Rings from "./Rings.svelte";
  import Room from "./Room.svelte";
  import Servers from "./Servers.svelte";
  import Settings from "./Settings.svelte";
  import Sidebar from "./Sidebar.svelte";
  import type { Snapshot } from "./types";

  // The app once the node is up: the rail on the left, one view beside it. A
  // call lives under Home, so moving to another view never touches it; a
  // share filling the screen takes the rail's space too.
  let { snapshot }: { snapshot: Snapshot } = $props();
</script>

<div class="shell" class:bare={screen.fullscreen}>
  {#if !screen.fullscreen}
    <Sidebar {snapshot} requests={friends.view?.incoming.length ?? 0} />
  {/if}
  <section class="content">
    {#if nav.view === "home"}
      {#if snapshot.room}
        <Room {snapshot} />
      {:else}
        <Home {snapshot} />
      {/if}
    {:else if nav.view === "friends"}
      <Friends {snapshot} />
    {:else if nav.view === "servers"}
      <Servers />
    {:else}
      <Settings {snapshot} />
    {/if}
  </section>
</div>

<Rings {snapshot} />

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--rail-w) minmax(0, 1fr);
    height: 100%;
  }

  .shell.bare {
    grid-template-columns: minmax(0, 1fr);
  }

  .content {
    min-width: 0;
    height: 100%;
    overflow: auto;
  }
</style>
