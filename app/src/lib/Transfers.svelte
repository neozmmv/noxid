<script lang="ts">
  import Check from "@lucide/svelte/icons/check";
  import Download from "@lucide/svelte/icons/download";
  import Upload from "@lucide/svelte/icons/upload";
  import X from "@lucide/svelte/icons/x";
  import { api } from "./api";
  import { tooltip } from "./tooltip";
  import type { TransferView } from "./types";

  // File transfers, in and out, as banners like the one for someone asking
  // to join: an offer to accept or decline, a progress bar while it moves,
  // and how it ended until dismissed.
  let { transfers }: { transfers: TransferView[] } = $props();

  /** A size the way people say it: bytes up to a kilobyte, then KB, MB, GB. */
  function size(bytes: number): string {
    const units = ["KB", "MB", "GB", "TB"];
    if (bytes < 1024) return `${bytes} B`;
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toLocaleString("en-US", { maximumFractionDigits: value < 10 ? 1 : 0 })} ${units[unit]}`;
  }

  const percent = (t: TransferView) => (t.size ? Math.floor((t.done / t.size) * 100) : 100);

  // Moving speed per transfer, from how much arrived since the last update.
  let samples: Record<number, { at: number; done: number; speed: number }> = {};
  function speed(t: TransferView): string | null {
    const now = performance.now();
    const last = samples[t.id];
    if (!last) {
      samples[t.id] = { at: now, done: t.done, speed: 0 };
      return null;
    }
    if (now - last.at >= 500) {
      const instant = ((t.done - last.done) / (now - last.at)) * 1000;
      // Smoothed, so the number is readable rather than jumping every update.
      const smooth = last.speed ? last.speed * 0.6 + instant * 0.4 : instant;
      samples[t.id] = { at: now, done: t.done, speed: smooth };
    }
    const current = samples[t.id]!.speed;
    return current > 0 ? `${size(current)}/s` : null;
  }

  function fail(e: unknown) {
    console.warn("file transfer action failed", e);
  }
</script>

{#each transfers as t (t.id)}
  {@const sending = t.direction === "send"}
  <section
    class="transfer"
    class:ended={t.state !== "offered" && t.state !== "waiting" && t.state !== "active"}
    class:bad={t.state === "failed" || t.state === "canceled_by_peer" || t.state === "declined"}
    role={t.state === "offered" ? "alert" : "status"}
    aria-label={`${sending ? "Sending" : "Receiving"} ${t.file_name}`}
  >
    <span class="icon" aria-hidden="true">
      {#if t.state === "done"}
        <Check size={18} />
      {:else if sending}
        <Upload size={18} />
      {:else}
        <Download size={18} />
      {/if}
    </span>

    <div class="body">
      <p class="what">
        {#if t.state === "offered"}
          <strong>{t.peer_name}</strong> wants to send you <strong class="file">{t.file_name}</strong> - {size(t.size)}
        {:else if t.state === "waiting"}
          Waiting for <strong>{t.peer_name}</strong> to accept <strong class="file">{t.file_name}</strong> - {size(t.size)}
        {:else if t.state === "active"}
          {sending ? "Sending" : "Receiving"} <strong class="file">{t.file_name}</strong>
          {sending ? "to" : "from"} <strong>{t.peer_name}</strong>
        {:else if t.state === "done"}
          {#if sending}
            Sent <strong class="file">{t.file_name}</strong> to <strong>{t.peer_name}</strong>
          {:else}
            Saved <strong class="file">{t.file_name}</strong> from <strong>{t.peer_name}</strong> to Downloads
          {/if}
        {:else if t.state === "declined"}
          <strong>{t.peer_name}</strong> declined <strong class="file">{t.file_name}</strong>
        {:else if t.state === "canceled"}
          You stopped {sending ? "sending" : "receiving"} <strong class="file">{t.file_name}</strong>
        {:else if t.state === "canceled_by_peer"}
          <strong>{t.peer_name}</strong> stopped {sending ? "receiving" : "sending"}
          <strong class="file">{t.file_name}</strong>
        {:else if t.state === "failed"}
          <strong class="file">{t.file_name}</strong> did not arrive: {t.message}
        {/if}
      </p>

      {#if t.state === "active"}
        <div
          class="bar"
          role="progressbar"
          aria-label={`${t.file_name} progress`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent(t)}
        >
          <div class="fill" style={`width: ${percent(t)}%`}></div>
        </div>
        <p class="meta">
          {size(t.done)} of {size(t.size)} · {percent(t)}%{#if speed(t)}&nbsp;· {speed(t)}{/if}
        </p>
      {/if}
    </div>

    <div class="actions">
      {#if t.state === "offered"}
        <button class="button primary" onclick={() => api.answerTransfer(t.id, true).catch(fail)}>Accept</button>
        <button class="button quiet" onclick={() => api.answerTransfer(t.id, false).catch(fail)}>Decline</button>
      {:else if t.state === "waiting"}
        <button class="button quiet" onclick={() => api.cancelTransfer(t.id).catch(fail)}>Cancel</button>
      {:else if t.state === "active"}
        <button class="button quiet" onclick={() => api.cancelTransfer(t.id).catch(fail)}>Stop</button>
      {:else}
        {#if t.state === "done" && !sending}
          <button class="button quiet" onclick={() => api.showTransfer(t.id).catch(fail)}>Show in folder</button>
        {/if}
        <button
          class="dismiss"
          aria-label={`Dismiss ${t.file_name}`}
          use:tooltip={"Dismiss"}
          onclick={() => api.dismissTransfer(t.id).catch(fail)}
        >
          <X size={18} aria-hidden="true" />
        </button>
      {/if}
    </div>
  </section>
{/each}

<style>
  .transfer {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.7rem 1rem;
    border-radius: 14px;
    background: var(--dusk);
    border: 1px solid var(--line);
    border-left: 4px solid var(--lamp);
  }

  .transfer.ended {
    border-left-color: var(--line);
  }

  .transfer.bad {
    border-left-color: var(--signal);
  }

  .icon {
    display: grid;
    place-items: center;
    color: var(--mist);
  }

  .body {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 0.4rem;
  }

  p {
    margin: 0;
  }

  .what {
    overflow-wrap: anywhere;
  }

  .file {
    font-weight: 650;
  }

  .meta {
    color: var(--mist);
    font-size: var(--step--1);
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 8px;
    border-radius: 999px;
    background: var(--ink);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--lamp);
    transition: width 150ms linear;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .dismiss {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--mist);
  }

  .dismiss:hover {
    background: var(--dusk-raised);
    color: var(--moon);
  }
</style>
