<script lang="ts">
  import { api } from "./api";
  import Dialog from "./Dialog.svelte";
  import { showToast } from "./toast.svelte";

  // Adding someone by the Noxid ID they shared. Sending is not befriending:
  // the copy says so, and so does the toast.
  let { onclose }: { onclose: () => void } = $props();

  let text = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  // A quick look before asking the backend, which has the final word: the
  // 64-character hex form, base32, or a room code in base58.
  const looksLikeId = $derived.by(() => {
    const t = text.trim();
    return (
      /^[0-9a-fA-F]{64}$/.test(t) ||
      /^[a-zA-Z2-7]{52}$/.test(t) ||
      /^[1-9A-HJ-NP-Za-km-z]{40,46}$/.test(t)
    );
  });
  const hint = $derived(
    text.trim() && !looksLikeId ? "A Noxid ID is 64 letters and digits, or a room code." : null,
  );

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    busy = true;
    try {
      const id = await api.checkFriendId(text);
      await api.sendFriendRequest(id);
      showToast("Friend request sent. You'll be friends once they accept.");
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Add a friend" {onclose}>
  <form id="add-friend" onsubmit={submit}>
    <label>
      <span>Their Noxid ID</span>
      <input
        class="field"
        bind:value={text}
        placeholder="Paste the ID they shared"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={!!error || !!hint}
        aria-describedby="add-friend-note"
      />
    </label>
    {#if error}
      <p class="error" role="alert">{error}</p>
    {:else if hint}
      <p class="hint">{hint}</p>
    {/if}
    <p class="note" id="add-friend-note">
      They'll get your request the next time you're both online, and you'll be friends once they accept.
    </p>
  </form>
  {#snippet actions()}
    <button class="button quiet" type="button" onclick={onclose}>Cancel</button>
    <button class="button primary" form="add-friend" disabled={busy || !looksLikeId}>
      {busy ? "Sending…" : "Send request"}
    </button>
  {/snippet}
</Dialog>

<style>
  form {
    display: grid;
    gap: 10px;
  }

  label {
    display: grid;
    gap: 6px;
  }

  label span {
    color: var(--mist);
    font-size: var(--step--1);
  }

  p {
    margin: 0;
  }

  .note,
  .hint {
    color: var(--mist);
    font-size: var(--step--1);
  }

  .error {
    color: var(--signal);
  }
</style>
