<script lang="ts">
  import { avatarUrl } from "./avatars.svelte";

  // A picture, or initials when there is none: for the profile button and the
  // friends list, where a call's speaking ring (Orb) would mean nothing.
  interface Props {
    name: string;
    hash?: string | null;
    size?: number;
  }

  let { name, hash = null, size = 36 }: Props = $props();

  const picture = $derived(avatarUrl(hash));
  const initials = $derived(
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w.charAt(0).toUpperCase())
      .join("") || "?",
  );
</script>

<span class="avatar" style={`--size: ${size}px`} aria-hidden="true">
  {#if picture}
    <img src={picture} alt="" />
  {:else}
    {initials}
  {/if}
</span>

<style>
  .avatar {
    display: grid;
    place-items: center;
    flex: none;
    width: var(--size);
    height: var(--size);
    border-radius: 50%;
    overflow: hidden;
    background: var(--dusk-raised);
    color: var(--moon);
    font-size: calc(var(--size) * 0.38);
    font-weight: 650;
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
