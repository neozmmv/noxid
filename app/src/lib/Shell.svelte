<script lang="ts">
  import { view as screen } from "./fullscreen.svelte";
  import Friends from "./Friends.svelte";
  import { friends } from "./friendlist.svelte";
  import Home from "./Home.svelte";
  import { page } from "./motion";
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
    <!-- A new view, or a call starting or ending, fades in a few pixels up;
         the old one goes at once, so the two never share the panel. -->
    {#key nav.view === "home" ? `home:${!!snapshot.room}` : nav.view}
      <div class="view" in:page>
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
      </div>
    {/key}
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

  /* The open view, on a panel of its own rather than behind a rule. */
  .content {
    min-width: 0;
    min-height: 0;
    margin: 0 8px 8px 0;
    border-radius: var(--radius-card);
    background: var(--panel);
    overflow: hidden;
  }

  /* The view scrolls itself rather than the panel: while it rises into
     place it sits a few pixels low, and in a scrolling panel that overhang
     would flash a scrollbar. The panel clips it instead. The room fills the
     panel to its edges, so the view does too. */
  .view {
    height: 100%;
    overflow: auto;
  }

  /* A share filling the screen keeps every pixel. */
  .bare .content {
    margin: 0;
    border-radius: 0;
  }
</style>
