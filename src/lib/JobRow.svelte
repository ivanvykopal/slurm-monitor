<script>
  import { invoke } from "@tauri-apps/api/core";
  let { job, mode = "expanded" } = $props();
  let busy = $state(false);
  let logText = $state(null);
  let logBusy = $state(false);
  let logStream = $state("stdout");
  let refreshInterval = null;
  let requestGeneration = 0;

  let live = $derived(logText !== null && job.state === "RUNNING");

  function startRefresh() {
    stopRefresh();
    if (job.state === "RUNNING") {
      refreshInterval = setInterval(() => peek(logStream), 5000);
    }
  }

  function stopRefresh() {
    if (refreshInterval !== null) {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
  }

  $effect(() => {
    if (logText !== null && job.state === "RUNNING") {
      startRefresh();
    } else {
      stopRefresh();
    }
  });

  function formatEstStart(raw) {
    const d = new Date(raw);
    return isNaN(d)
      ? raw
      : d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  async function cancel() {
    if (!confirm(`Cancel job ${job.name} (${job.id}) on ${job.cluster}?`)) return;
    busy = true;
    try {
      await invoke("cancel_job", { jobId: job.id, cluster: job.cluster });
    } catch (e) {
      alert(`Cancel failed: ${e}`);
    } finally {
      busy = false;
    }
  }

  function closeLog() {
    stopRefresh();
    requestGeneration++;
    logText = null;
  }

  async function peek(stream = "stdout") {
    const generation = ++requestGeneration;
    logStream = stream;
    logBusy = true;
    try {
      const text = await invoke("tail_log", { jobId: job.id, cluster: job.cluster, lines: 40, stream });
      if (generation === requestGeneration) {
        logText = text;
      }
    } catch (e) {
      if (generation === requestGeneration) {
        logText = `Error: ${e}`;
      }
    } finally {
      logBusy = false;
    }
  }
</script>
<li class="job {mode}">
  <span class="job-name">{job.name} ({job.id})</span>
  {#if mode === "expanded"}
    <span class="job-state state-{job.state}">{job.state} · {job.time}{#if job.time_limit} / {job.time_limit}{/if}</span>
    <span class="job-meta">
      {job.partition} · {job.nodes} node(s)
      {#if job.state === "PENDING"}
        {#if job.est_start && job.est_start !== "N/A"}
          · est {formatEstStart(job.est_start)}
        {/if}
        · {job.reason}
      {/if}
    </span>
  {:else}
    <span class="job-state state-{job.state}">{job.state}</span>
  {/if}
  <span class="job-actions">
    <button
      class="job-action"
      disabled={logBusy}
      onclick={() => peek("stdout")}
      title={logBusy ? "Fetching output…" : `View last 40 lines of output for ${job.name}`}
    >▤</button>
    <button
      class="job-action"
      disabled={logBusy}
      onclick={() => peek("stderr")}
      title={logBusy ? "Fetching error output…" : `View last 40 lines of error output for ${job.name}`}
    >⚠</button>
    {#if job.state === "RUNNING" || job.state === "PENDING"}
      <button
        class="job-action danger"
        disabled={busy}
        onclick={cancel}
        title={busy ? "Cancelling…" : `Cancel job ${job.name} (${job.id})`}
      >✕</button>
    {/if}
  </span>
</li>
{#if logText !== null}
  <div
    class="log-modal"
    role="button"
    tabindex="0"
    onclick={closeLog}
    onkeydown={(e) => (e.key === "Escape" || e.key === "Enter") && closeLog()}
  >
    <div class="log-panel" role="button" tabindex="0" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
      <div class="log-tabs">
        <button class:active={logStream === "stdout"} disabled={logBusy} onclick={() => peek("stdout")}>Output</button>
        <button class:active={logStream === "stderr"} disabled={logBusy} onclick={() => peek("stderr")}>Errors</button>
        {#if live}<span class="live-indicator" title="Auto-refreshing while job runs">● live</span>{/if}
        <button class="log-close" onclick={closeLog} title="Close">✕</button>
      </div>
      <pre>{logText}</pre>
    </div>
  </div>
{/if}

<style>
  .job-action {
    background: var(--color-input-bg);
    border: 1px solid var(--color-panel-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--font-md);
    line-height: 1;
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
  }
  .job-action:hover:not(:disabled) {
    color: var(--color-text);
    border-color: var(--color-accent);
  }
  .job-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .job-action.danger:hover:not(:disabled) {
    color: var(--color-error-fg);
    border-color: var(--color-danger);
  }
  .log-modal {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .log-panel {
    display: flex;
    flex-direction: column;
    max-width: 80vw;
    max-height: 80vh;
  }
  .log-tabs {
    display: flex;
    gap: var(--space-1);
    margin-bottom: var(--space-1);
  }
  .log-tabs button {
    background: var(--color-input-bg);
    border: 1px solid var(--color-panel-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--font-md);
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
  }
  .log-tabs button.active {
    color: var(--color-text);
    border-color: var(--color-accent);
  }
  .log-tabs button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .log-close {
    margin-left: auto;
  }
  .log-close:hover {
    color: var(--color-error-fg);
    border-color: var(--color-danger);
  }
  .live-indicator {
    color: var(--color-success-fg);
    font-size: var(--font-xs);
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-right: auto;
    padding: 0 var(--space-1);
  }
  .log-panel pre {
    background: var(--color-bg);
    color: var(--color-text);
    border: 1px solid var(--color-panel-border);
    font-size: var(--font-sm);
    padding: var(--space-3);
    margin: 0;
    overflow: auto;
    border-radius: var(--radius-sm);
  }
</style>
