<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { uiState } from "./stores.js";
  import JobRow from "./JobRow.svelte";
  let { cluster, jobs = [], conn = { status: "…", detail: "", next_retry_secs: 0 }, mode = "expanded" } = $props();

  let open = $derived(!$uiState.collapsed_clusters.includes(cluster));
  let showHealth = $derived($uiState.open_health.includes(cluster));
  let showProjects = $derived($uiState.open_projects.includes(cluster));
  let health = $state(null);
  let healthLoading = $state(false);
  let projects = $state(null);
  let projectsLoading = $state(false);

  function setOpen(value) {
    uiState.update(s => ({
      ...s,
      collapsed_clusters: value
        ? s.collapsed_clusters.filter(c => c !== cluster)
        : [...s.collapsed_clusters, cluster]
    }));
  }

  async function loadHealth() {
    if (healthLoading || health) return;
    healthLoading = true;
    try {
      health = await invoke("cluster_health", { cluster });
    } catch (e) {
      health = { error: String(e) };
    } finally {
      healthLoading = false;
    }
  }

  async function loadProjects() {
    if (projectsLoading || projects) return;
    projectsLoading = true;
    try {
      const result = await invoke("cluster_projects", { cluster });
      projects = result.projects ?? [];
    } catch (e) {
      let msg = String(e);
      if (msg.includes("6819") || msg.includes("talking to the database")) {
        msg = "Cluster accounting database (slurmdbd) is unreachable on the cluster — projects data is unavailable until it is back.";
      } else if (msg.includes("10060") || msg.includes("10053") || msg.includes("connecting to")) {
        msg = "Could not reach the cluster login node over SSH — check your connection or VPN.";
      }
      projects = { error: msg };
    } finally {
      projectsLoading = false;
    }
  }

  function retryProjects() {
    projects = null;
    loadProjects();
  }

  function toggleHealthPanel() {
    uiState.update(s => ({
      ...s,
      open_health: showHealth
        ? s.open_health.filter(c => c !== cluster)
        : [...s.open_health, cluster]
    }));
  }

  function toggleProjectsPanel() {
    const willShow = !showProjects;
    uiState.update(s => ({
      ...s,
      open_projects: willShow
        ? [...s.open_projects, cluster]
        : s.open_projects.filter(c => c !== cluster)
    }));
    if (willShow) {
      loadProjects();
    } else {
      projects = null;
    }
  }

  $effect(() => {
    if (showHealth) {
      loadHealth();
    } else {
      health = null;
    }
  });

  // Load once if the projects panel was persisted open across restarts.
  // Ongoing opens/closes are driven by toggleProjectsPanel, not a reactive
  // effect, so the account data is fetched exactly once per open instead of
  // being re-fetched in a loop.
  onMount(() => {
    if (showProjects) loadProjects();
  });

  function parseNum(value) {
    const n = parseFloat(value);
    return isNaN(n) || n < 0 ? null : n;
  }

  // "<spent> <unit>-h (<pct>% of <allocated>)", or just spent when no cap.
  function hoursText(spent, allocated, unit) {
    if (allocated === null || allocated <= 0) {
      return `${spent} ${unit}-h`;
    }
    const pct = Math.round((spent / allocated) * 100);
    return `${spent} ${unit}-h (${pct}% of ${allocated})`;
  }

  // Usage ratio clamped for the bar fill; over-quota keeps the bar full and
  // the exact percentage is shown in the text.
  function fillPct(spent, allocated) {
    if (allocated === null || allocated <= 0) return 0;
    return Math.min(100, (spent / allocated) * 100);
  }

  function tier(spent, allocated) {
    if (allocated === null || allocated <= 0) return "none";
    const ratio = spent / allocated;
    if (ratio >= 1) return "over";
    if (ratio >= 0.9) return "high";
    if (ratio >= 0.7) return "mid";
    return "low";
  }
</script>

<section class="cluster">
  <header onclick={() => setOpen(!open)} onkeydown={(e) => e.key === 'Enter' && setOpen(!open)} role="button" tabindex="0">
    <span class="chevron">{open ? "▾" : "▸"}</span>
    <span class="cluster-name">{cluster}</span>
    <span class="badge {conn.status}" title={conn.detail}>
      {conn.status}{#if conn.status === "disconnected" && conn.next_retry_secs}
        &nbsp;· retry {conn.next_retry_secs}s{/if}
    </span>
    <button class:active={showHealth} onclick={(e) => { e.stopPropagation(); toggleHealthPanel(); }} title={showHealth ? "hide cluster health" : "show cluster health"}>☰</button>
    <button class:active={showProjects} onclick={(e) => { e.stopPropagation(); toggleProjectsPanel(); }} title={showProjects ? "hide projects" : "show projects (accounts with usage and GPU)"}>◫</button>
  </header>
  {#if showProjects}
    <div class="health projects">
      {#if projectsLoading}
        <div>Loading projects… (may take up to ~30s if the cluster is unreachable)</div>
      {:else if projects?.error}
        <div class="err">{projects.error}</div>
        <button class="retry-btn" onclick={retryProjects} disabled={projectsLoading} title="Fetch projects again">↻ Retry</button>
      {:else if projects?.length}
        {#each projects as p (p.account)}
          {@const gpuSpent = parseNum(p.gpu_hours) ?? 0}
          {@const gpuAlloc = parseNum(p.gpu_hours_allocated)}
          {@const cpuSpent = parseNum(p.cpu_hours) ?? 0}
          {@const cpuAlloc = parseNum(p.cpu_hours_allocated)}
          {@const billSpent = parseNum(p.billing_hours) ?? 0}
          {@const billAlloc = parseNum(p.billing_hours_allocated)}
          {@const showBilling = (billAlloc ?? 0) > 0 || billSpent > 0}
          <div class="project-row">
            <span class="project-name">{p.account}</span>
            <div class="project-bars">
              <div class="usage-line" class:skip={gpuAlloc === null || gpuAlloc <= 0}
                   title={`GPU: ${hoursText(gpuSpent, gpuAlloc, "GPU")}`}>
                <span class="usage-label">GPU</span>
                <div class="usage-bar {tier(gpuSpent, gpuAlloc)}">
                  <div class="usage-fill" style={`width:${fillPct(gpuSpent, gpuAlloc)}%`}></div>
                </div>
                <span class="usage-text">{hoursText(gpuSpent, gpuAlloc, "GPU")}</span>
              </div>
              <div class="usage-line" class:skip={cpuAlloc === null || cpuAlloc <= 0}
                   title={`CPU: ${hoursText(cpuSpent, cpuAlloc, "CPU")}`}>
                <span class="usage-label">CPU</span>
                <div class="usage-bar {tier(cpuSpent, cpuAlloc)}">
                  <div class="usage-fill" style={`width:${fillPct(cpuSpent, cpuAlloc)}%`}></div>
                </div>
                <span class="usage-text">{hoursText(cpuSpent, cpuAlloc, "CPU")}</span>
              </div>
              {#if showBilling}
                <div class="usage-line" class:skip={billAlloc === null || billAlloc <= 0}
                     title={`Billing: ${hoursText(billSpent, billAlloc, "billing")} — cluster budget unit (GPU counts 16x CPU)`}>
                  <span class="usage-label">BILL</span>
                  <div class="usage-bar {tier(billSpent, billAlloc)}">
                    <div class="usage-fill" style={`width:${fillPct(billSpent, billAlloc)}%`}></div>
                  </div>
                  <span class="usage-text">{hoursText(billSpent, billAlloc, "billing")}</span>
                </div>
              {/if}
            </div>
          </div>
        {/each}
      {:else}
        <div>No projects found.</div>
      {/if}
    </div>
  {/if}
  {#if showHealth && health}
    <div class="health">
      {#if health.fair_share}<div>Fair-share priority: {health.fair_share}</div>{/if}
      {#each health.partitions ?? [] as p}
        {@const [alloc, idle, other, total] = p.nodes.split("/")}
        <div>
          <strong>{p.partition}</strong> ({p.avail}) &mdash;
          {alloc} allocated · {idle} idle · {other} other · {total} total nodes
        </div>
      {/each}
      {#if health.error}<div class="err">{health.error}</div>{/if}
    </div>
  {/if}
  {#if open}
    <ul>
      {#each jobs as job (job.id)}<JobRow {job} {mode} />{/each}
      {#if jobs.length === 0}<li class="empty">No active jobs</li>{/if}
    </ul>
  {/if}
</section>

<style>
  .projects .project-row {
    margin-bottom: var(--space-2);
  }
  .projects .project-name {
    font-weight: 600;
    overflow-wrap: anywhere;
    margin-bottom: var(--space-1);
  }
  .projects .usage-line {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: 2px;
  }
  .projects .usage-line.skip .usage-bar {
    visibility: hidden;
  }
  .projects .usage-label {
    font-size: var(--font-xs);
    font-weight: 600;
    color: var(--color-text-muted);
    width: 2.25em;
    flex-shrink: 0;
  }
  .projects .usage-bar {
    flex: 1 1 90px;
    height: 5px;
    border-radius: var(--radius-sm);
    background: var(--color-input-bg);
    border: 1px solid var(--color-panel-border);
    overflow: hidden;
    flex-shrink: 1;
  }
  .projects .usage-fill {
    height: 100%;
    border-radius: inherit;
    transition: width 0.3s ease;
  }
  .projects .usage-bar.low .usage-fill {
    background: var(--color-ok, #3fb950);
  }
  .projects .usage-bar.mid .usage-fill {
    background: var(--color-warn, #d29922);
  }
  .projects .usage-bar.high .usage-fill {
    background: var(--color-danger, #f85149);
  }
  .projects .usage-bar.over .usage-fill {
    background: repeating-linear-gradient(
      45deg,
      var(--color-danger, #f85149) 0 4px,
      #a02020 4px 8px
    );
  }
  .projects .usage-text {
    font-size: var(--font-xs);
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .projects .project-raw {
    flex-basis: 100%;
    font-size: var(--font-xs);
    color: var(--color-text-muted);
    opacity: 0.7;
  }
  .projects .project-users {
    flex-basis: 100%;
    list-style: none;
    margin: 0 0 0 var(--space-2);
    padding: 0;
    font-size: var(--font-xs);
    color: var(--color-text-muted);
  }
  .projects .project-users li {
    margin-bottom: var(--space-1);
  }
  .projects .retry-btn {
    background: var(--color-input-bg);
    border: 1px solid var(--color-panel-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--font-xs);
    padding: var(--space-1) var(--space-2);
    margin-top: var(--space-1);
    cursor: pointer;
  }
  .projects .retry-btn:hover:not(:disabled) {
    color: var(--color-text);
    border-color: var(--color-accent);
  }
  .projects .retry-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
