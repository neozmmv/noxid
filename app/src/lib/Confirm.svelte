<script lang="ts">
  import { onMount } from "svelte";

  // A yes-or-no question in a modal. The safe answer gets the focus, so Enter
  // or Esc never does the drastic thing by accident.
  interface Props {
    question: string;
    detail?: string;
    confirm: string;
    cancel: string;
    onconfirm: () => void;
    oncancel: () => void;
  }

  let { question, detail, confirm, cancel, onconfirm, oncancel }: Props = $props();

  let safe: HTMLButtonElement;
  onMount(() => safe.focus());

  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") oncancel();
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onclick={oncancel}></div>
<div class="confirm" role="alertdialog" aria-modal="true" aria-labelledby="confirm-question" aria-describedby={detail ? "confirm-detail" : undefined}>
  <h2 id="confirm-question">{question}</h2>
  {#if detail}
    <p id="confirm-detail">{detail}</p>
  {/if}
  <div class="actions">
    <button class="button quiet" bind:this={safe} onclick={oncancel}>{cancel}</button>
    <button class="button danger" onclick={onconfirm}>{confirm}</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: var(--scrim);
  }

  .confirm {
    position: fixed;
    z-index: 31;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(400px, calc(100vw - 2rem));
    padding: 1.4rem;
    border-radius: 16px;
    background: var(--dusk);
    border: 1px solid var(--line);
    display: grid;
    gap: 0.6rem;
  }

  h2 {
    margin: 0;
    font-size: var(--step-1);
  }

  p {
    margin: 0;
    color: var(--mist);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.6rem;
    margin-top: 0.6rem;
  }
</style>
