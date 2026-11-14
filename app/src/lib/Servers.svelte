<script lang="ts">
  import Server from "@lucide/svelte/icons/server";
</script>

<!-- Not built yet, and saying so: nothing here pretends to work. -->
<div class="placeholder">
  <span class="icon" aria-hidden="true"><Server size={28} strokeWidth={1.5} /></span>
  <h1>Servers are on the way</h1>
  <p>Persistent spaces for a group, with rooms you can drop into. Not in this version yet.</p>
</div>

<style>
  .placeholder {
    height: 100%;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    padding: 24px;
    text-align: center;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: var(--radius-card);
    background: var(--dusk-raised);
    color: var(--mist);
  }

  h1 {
    margin: 0;
    font-size: var(--step-2);
  }

  p {
    margin: 0;
    max-width: 40ch;
    color: var(--mist);
  }
</style>
