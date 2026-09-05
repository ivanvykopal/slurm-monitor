<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { uiState, projectsByCluster } from "./stores.js";
  import JobRow from "./JobRow.svelte";
  let { cluster, jobs = [], conn = { status: "…", detail: "", next_retry_secs: 0 }, mode = "expanded" } = $props();

  let open = $derived(!$uiState.collapsed_clusters.includes(cluster));
  let showHealth = $derived($uiState.open_health.includes(cluster));
  let showProjects = $derived($uiState.open_projects.includes(cluster));
  let health = $state(null);
  let healthLoading = $state(false);
  let projects = $state(null);
  let projectsLoading = $state(false);
  let efficiency = $state(null);
  let efficiencyLoading = $state(false);
  let disks = $state(null);
  let disksLoading = $state(false);

  let showEfficiency = $derived($uiState.open_efficiency?.includes(cluster) ?? false);
  let showDisks = $derived($uiState.open_disks?.includes(cluster) ?? false);

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
      projectsByCluster.update(m => ({ ...m, [cluster]: projects }));
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

  async function loadEfficiency() {
    if (efficiencyLoading || efficiency) return;
    efficiencyLoading = true;
    try {
      const result = await invoke("cluster_efficiency", { cluster });
      efficiency = result.jobs ?? [];
    } catch (e) {
      efficiency = { error: String(e) };
    } finally {
      efficiencyLoading = false;
    }
  }

  function retryEfficiency() {
    efficiency = null;
    loadEfficiency();
  }

  async function loadDisks() {
    if (disksLoading || disks) return;
    disksLoading = true;
    try {
      const result = await invoke("cluster_disks", { cluster });
      disks = result.disks ?? [];
    } catch (e) {
      disks = { error: String(e) };
    } finally {
      disksLoading = false;
    }
  }

  function retryDisks() {
    disks = null;
    loadDisks();
  }

  function toggleDisksPanel() {
    const willShow = !showDisks;
    uiState.update(s => ({
      ...s,
      open_disks: willShow
        ? [...(s.open_disks ?? []), cluster]
        : (s.open_disks ?? []).filter(c => c !== cluster)
    }));
    if (willShow) {
      loadDisks();
    } else {
      disks = null;
    }
  }

  function toggleEfficiencyPanel() {
    const willShow = !showEfficiency;
    uiState.update(s => ({
      ...s,
      open_efficiency: willShow
        ? [...(s.open_efficiency ?? []), cluster]
        : (s.open_efficiency ?? []).filter(c => c !== cluster)
    }));
    if (willShow) {
      loadEfficiency();
    } else {
      efficiency = null;
    }
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

  // Load once if the projects / efficiency panels were persisted open
  // across restarts. Ongoing opens/closes are driven by the toggle
  // functions, not a reactive effect, so the data is fetched exactly once
  // per open instead of being re-fetched in a loop.
  onMount(() => {
    if (showProjects) loadProjects();
    if (showEfficiency) loadEfficiency();
    if (showDisks) loadDisks();
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

  // "312 GPU-h left · ~9.3/day → ~34 days", or null when no cap or no
  // recent usage. Days-left uses the 30-day daily burn; under 7 days is
  // treated as urgent by the caller.
  function burnText(spent, allocated, spent30d) {
    if (allocated === null || allocated <= 0) return null;
    const remaining = Math.max(0, allocated - spent);
    const daily = (spent30d ?? 0) / 30;
    if (daily <= 0) {
      return `${fmtNum(remaining)}-h left`;
    }
    const days = remaining / daily;
    return `${fmtNum(remaining)}-h left · ~${daily.toFixed(1)}/day → ~${Math.floor(days)} days`;
  }

  function daysLeft(spent, allocated, spent30d) {
    if (allocated === null || allocated <= 0) return null;
    const daily = (spent30d ?? 0) / 30;
    if (daily <= 0) return null;
    return Math.max(0, (allocated - spent) / daily);
  }

  function fmtNum(n) {
    return Math.round(n).toLocaleString("en-US");
  }

  // Efficiency flags for a finished job: over-requesting slows the queue.
  function effFlags(j) {
    const flags = [];
    if (j.cpu_util_pct != null && j.cpu_util_pct < 10) flags.push("low CPU");
    if (j.mem_ratio_pct != null && j.mem_ratio_pct < 10) flags.push("low mem");
    if (j.mem_ratio_pct != null && j.mem_ratio_pct > 100) flags.push("over mem");
    if (j.walltime_pct != null && j.walltime_pct < 25) flags.push("short run");
    return flags;
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
    <button class:active={showEfficiency} onclick={(e) => { e.stopPropagation(); toggleEfficiencyPanel(); }} title={showEfficiency ? "hide efficiency report" : "show efficiency of finished jobs (last 7 days)"}>⚡</button>
    <button class:active={showDisks} onclick={(e) => { e.stopPropagation(); toggleDisksPanel(); }} title={showDisks ? "hide disk usage" : "show disk usage (home and scratch)"}>💾</button>
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
              {#if burnText(gpuSpent, gpuAlloc, parseNum(p.gpu_hours_30d))}
                {@const gpuDays = daysLeft(gpuSpent, gpuAlloc, parseNum(p.gpu_hours_30d))}
                <div class="usage-burn {gpuDays !== null && gpuDays < 7 ? 'urgent' : ''}">
                  {burnText(gpuSpent, gpuAlloc, parseNum(p.gpu_hours_30d))}
                </div>
              {/if}
              <div class="usage-line" class:skip={cpuAlloc === null || cpuAlloc <= 0}
                   title={`CPU: ${hoursText(cpuSpent, cpuAlloc, "CPU")}`}>
                <span class="usage-label">CPU</span>
                <div class="usage-bar {tier(cpuSpent, cpuAlloc)}">
                  <div class="usage-fill" style={`width:${fillPct(cpuSpent, cpuAlloc)}%`}></div>
                </div>
                <span class="usage-text">{hoursText(cpuSpent, cpuAlloc, "CPU")}</span>
              </div>
              {#if burnText(cpuSpent, cpuAlloc, parseNum(p.cpu_hours_30d))}
                <div class="usage-burn">{burnText(cpuSpent, cpuAlloc, parseNum(p.cpu_hours_30d))}</div>
              {/if}
              {#if showBilling}
                <div class="usage-line" class:skip={billAlloc === null || billAlloc <= 0}
                     title={`Billing: ${hoursText(billSpent, billAlloc, "billing")} — cluster budget unit (GPU counts 16x CPU)`}>
                  <span class="usage-label">BILL</span>
                  <div class="usage-bar {tier(billSpent, billAlloc)}">
                    <div class="usage-fill" style={`width:${fillPct(billSpent, billAlloc)}%`}></div>
                  </div>
                  <span class="usage-text">{hoursText(billSpent, billAlloc, "billing")}</span>
                </div>
                {#if burnText(billSpent, billAlloc, parseNum(p.billing_hours_30d))}
                  {@const billDays = daysLeft(billSpent, billAlloc, parseNum(p.billing_hours_30d))}
                  <div class="usage-burn {billDays !== null && billDays < 7 ? 'urgent' : ''}">
                    {burnText(billSpent, billAlloc, parseNum(p.billing_hours_30d))}
                  </div>
                {/if}
              {/if}
            </div>
          </div>
        {/each}
      {:else}
        <div>No projects found.</div>
      {/if}
    </div>
  {/if}
  {#if showDisks}
    <div class="health disks">
      {#if disksLoading}
        <div>Loading disk usage…</div>
      {:else if disks?.error}
        <div class="err">{disks.error}</div>
        <button class="retry-btn" onclick={retryDisks} disabled={disksLoading}>↻ Retry</button>
      {:else if disks?.length}
        {#each disks as d (d.filesystem)}
          {@const pct = d.used_pct}
          <div class="usage-line" class:skip={pct == null}
               title={`${d.filesystem}: ${d.used} of ${d.size} used`}>
            <span class="usage-label disk" title={d.filesystem}>{d.filesystem}</span>
            <div class="usage-bar {tier(pct ?? 0, 100)}">
              <div class="usage-fill" style={`width:${pct ?? 0}%`}></div>
            </div>
            <span class="usage-text">{pct == null ? `${d.used} / ${d.size}` : `${pct}% of ${d.size}`}</span>
          </div>
        {/each}
      {:else}
        <div>No mounted filesystems matched the configured paths.</div>
      {/if}
    </div>
  {/if}
  {#if showEfficiency}
    <div class="health efficiency">
      {#if efficiencyLoading}
        <div>Loading finished jobs… (may take up to ~30s if the cluster is unreachable)</div>
      {:else if efficiency?.error}
        <div class="err">{efficiency.error}</div>
        <button class="retry-btn" onclick={retryEfficiency} disabled={efficiencyLoading}>↻ Retry</button>
      {:else if efficiency?.length}
        <div class="eff-hint">Finished jobs, last 7 days — over-requested jobs slow the queue</div>
        {#each efficiency as j (j.id)}
          <div class="eff-row">
            <span class="eff-name" title={`${j.id} · ${j.elapsed} · ${j.alloc_cpus} CPUs`}>{j.name}</span>
            <span class="eff-state {j.state === 'COMPLETED' ? 'ok' : 'bad'}">{j.state}</span>
            <span class="eff-metric" title="CPU utilization (CPUTime / reserved CPU-time)">
              {j.cpu_util_pct == null ? "CPU ?" : `CPU ${Math.round(j.cpu_util_pct)}%`}
            </span>
            <span class="eff-metric" title={`Memory: ${j.mem_used} used / ${j.mem_req} requested`}>
              {j.mem_ratio_pct == null ? "mem ?" : `mem ${Math.round(j.mem_ratio_pct)}%`}
            </span>
            <span class="eff-metric" title="Fraction of requested walltime used">
              {j.walltime_pct == null ? "wall ?" : `wall ${Math.round(j.walltime_pct)}%`}
            </span>
            {#each effFlags(j) as flag (flag)}<span class="eff-flag">{flag}</span>{/each}
          </div>
        {/each}
      {:else}
        <div>No finished jobs in the last 7 days.</div>
      {/if}
    </div>
  {/if}
  {#if showHealth && health}
    <div class="health">
      {#if health.fair_share}<div>Fair-share priority: {health.fair_share}</div>{/if}
      <div class="health-hint">Sorted by idle nodes — the top partition is the best place to submit right now</div>
      {#each health.partitions ?? [] as p}
        {@const [alloc, idle, other, total] = p.nodes.split("/")}
        <div>
          <strong>{p.partition}</strong> ({p.avail}) &mdash;
          {alloc} allocated · {idle} idle · {other} other · {total} total nodes ·
          max {p.max_time}
          {#if p.running_jobs || p.queued_jobs}· {p.running_jobs} running · {p.queued_jobs} queued{/if}
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
  .disks .usage-label.disk {
    width: 7em;
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .projects .usage-burn {
    font-size: var(--font-xs);
    color: var(--color-text-muted);
    margin: 0 0 2px calc(2.25em + var(--space-1));
    opacity: 0.85;
  }
  .projects .usage-burn.urgent {
    color: var(--color-danger, #f85149);
    font-weight: 600;
    opacity: 1;
  }
  .health .health-hint {
    font-size: var(--font-xs);
    color: var(--color-text-muted);
    opacity: 0.8;
    margin-bottom: var(--space-1);
  }
  .efficiency .eff-hint {
    font-size: var(--font-xs);
    color: var(--color-text-muted);
    margin-bottom: var(--space-1);
    opacity: 0.8;
  }
  .efficiency .eff-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    font-size: var(--font-xs);
    margin-bottom: 2px;
  }
  .efficiency .eff-name {
    font-weight: 600;
    max-width: 12em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .efficiency .eff-state.ok { color: var(--color-ok, #3fb950); }
  .efficiency .eff-state.bad { color: var(--color-danger, #f85149); }
  .efficiency .eff-metric {
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .efficiency .eff-flag {
    font-size: var(--font-xs);
    color: var(--color-warn, #d29922);
    border: 1px solid currentColor;
    border-radius: var(--radius-sm);
    padding: 0 4px;
  }
</style>
