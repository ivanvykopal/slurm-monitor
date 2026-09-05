<script>
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { jobs, connections, viewMode, uiState, history } from "./lib/stores.js";
  import ClusterSection from "./lib/ClusterSection.svelte";
  import SummaryBar from "./lib/SummaryBar.svelte";
  import HistoryPanel from "./lib/HistoryPanel.svelte";
  import { onMount } from "svelte";

  let view = $state("monitor");
  let showHistory = $state(false);

  async function loadUiState() {
    try {
      const saved = await invoke("get_ui_state");
      uiState.set(saved);
      viewMode.set(saved.view_mode || "expanded");
    } catch (e) {
      console.warn("failed to load ui state", e);
    }
  }

  function saveUiState(state) {
    invoke("save_ui_state", { state }).catch(() => {});
  }

  function minimize() {
    invoke("hide_window");
  }

  function toggleView() {
    viewMode.update(v => v === "expanded" ? "compact" : "expanded");
  }

  let clusterNames = $derived(
    [...new Set([...Object.keys($jobs), ...Object.keys($connections)])].sort()
  );

  viewMode.subscribe(v => {
    uiState.update(s => ({ ...s, view_mode: v }));
  });

  uiState.subscribe(s => {
    saveUiState(s);
  });

  onMount(async () => {
    loadUiState();
    try {
      history.set(await invoke("get_history", { count: 20 }));
    } catch (e) {
      console.warn("failed to load history", e);
    }
    const unlisten = [];
    listen("jobs-updated", (e) => {
      jobs.update((m) => ({ ...m, [e.payload.cluster]: e.payload.jobs }));
    }).then((f) => unlisten.push(f));
    listen("connection-status", (e) => {
      connections.update((m) => ({ ...m,
        [e.payload.cluster]: {
          status: e.payload.status,
          detail: e.payload.detail,
          next_retry_secs: e.payload.next_retry_secs,
        } }));
    }).then((f) => unlisten.push(f));
    listen("history-updated", (e) => {
      history.update((h) => [e.payload, ...h].slice(0, 20));
    }).then((f) => unlisten.push(f));
    return () => unlisten.forEach((f) => f());
  });
</script>

<nav data-tauri-drag-region>
  <h1 data-tauri-drag-region>Slurm Monitor</h1>
  <button onclick={() => showHistory = !showHistory} title="Recent state changes" class:active={showHistory}>🕐</button>
  <button onclick={toggleView} title={$viewMode === "expanded" ? "Switch to compact view" : "Switch to expanded view"}>
    {$viewMode === "expanded" ? "⛶" : "☰"}
  </button>
  <button onclick={minimize} title="Minimize to tray (restore via the tray icon)">🗕</button>
  <button onclick={() => view = view === "monitor" ? "settings" : "monitor"}>
    {view === "monitor" ? "⚙" : "←"}
  </button>
</nav>

<main data-tauri-drag-region>
  {#if view === "settings"}
    {#await import("./lib/Settings.svelte") then Settings}<Settings.default />{/await}
  {:else}
    {#if showHistory}
      <HistoryPanel />
    {:else}
      <SummaryBar />
      {#each clusterNames as cluster (cluster)}
        <ClusterSection {cluster} jobs={$jobs[cluster] ?? []} conn={$connections[cluster]} mode={$viewMode} />
      {/each}
      {#if clusterNames.length === 0}
        <p class="empty">No clusters configured.</p>
      {/if}
    {/if}
  {/if}
</main>
