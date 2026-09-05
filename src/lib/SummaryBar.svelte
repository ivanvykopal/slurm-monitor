<script>
  import { jobs } from "./stores.js";
  let counts = $derived.by(() => {
    const all = Object.values($jobs).flat();
    const running = all.filter(j => j.state === "RUNNING").length;
    const pending = all.filter(j => j.state === "PENDING").length;
    return { running, pending, other: all.length - running - pending, total: all.length };
  });
</script>
<div class="summary">
  <span class="state-RUNNING">{counts.running} running</span> ·
  <span class="state-PENDING">{counts.pending} pending</span> ·
  <span>{counts.other} other</span>
</div>
