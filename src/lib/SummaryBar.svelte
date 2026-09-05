<script>
  import { jobs, projectsByCluster } from "./stores.js";
  let counts = $derived.by(() => {
    const all = Object.values($jobs).flat();
    const running = all.filter(j => j.state === "RUNNING").length;
    const pending = all.filter(j => j.state === "PENDING").length;
    return { running, pending, other: all.length - running - pending, total: all.length };
  });

  // Per-cluster GPU-hours remaining, only for clusters whose projects panel
  // has been opened (that is when the allocation data is fetched).
  let gpuLeft = $derived.by(() => {
    const parts = [];
    for (const [cluster, projects] of Object.entries($projectsByCluster)) {
      let left = 0;
      let any = false;
      for (const p of projects ?? []) {
        const alloc = parseFloat(p.gpu_hours_allocated);
        const spent = parseFloat(p.gpu_hours);
        if (!isNaN(alloc) && alloc > 0 && !isNaN(spent)) {
          left += Math.max(0, alloc - spent);
          any = true;
        }
      }
      if (any) parts.push({ cluster, left });
    }
    return parts;
  });
</script>
<div class="summary">
  <span class="state-RUNNING">{counts.running} running</span> ·
  <span class="state-PENDING">{counts.pending} pending</span> ·
  <span>{counts.other} other</span>
  {#if gpuLeft.length}
    · <span class="gpu-left" title="GPU-hours remaining per cluster (open a cluster's projects panel to load its data)">
      GPU left: {#each gpuLeft as g, i (g.cluster)}{#if i > 0} · {/if}{g.cluster} {Math.round(g.left).toLocaleString("en-US")} h{/each}
    </span>
  {/if}
</div>
<style>
  .gpu-left {
    color: var(--color-text-muted);
  }
</style>
