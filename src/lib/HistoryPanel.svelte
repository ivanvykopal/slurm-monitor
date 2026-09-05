<script>
  import { history } from "./stores.js";
  function fmt(ts) {
    return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
</script>

<div class="history-panel">
  <h3>Recent changes</h3>
  {#if $history.length === 0}
    <p class="empty">No recent state changes.</p>
  {:else}
    <ul>
      {#each $history as e}
        <li class="history-entry">
          <span class="history-time">{fmt(e.timestamp_secs)}</span>
          <span class="history-cluster">[{e.cluster}]</span>
          <span class="history-name">{e.job_name}</span>
          <span class="history-transition">{e.from_state} → {e.to_state}</span>
          {#if e.detail}<span class="history-detail">({e.detail})</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
