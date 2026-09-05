<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  let cfg = $state({ clusters: [], notifications: { notify_states: [], quiet_start: null, quiet_end: null } });
  let saving = $state(false);
  let error = $state("");
  let autostart = $state(false);

  onMount(async () => {
    cfg = await invoke("get_config");
    autostart = await invoke("get_autostart_state");
  });

  async function toggleAutostart() {
    const next = !autostart;
    try {
      await invoke("set_autostart_state", { enable: next });
      autostart = next;
    } catch (e) {
      error = String(e);
    }
  }

  function addCluster() {
    cfg.clusters = [...cfg.clusters, { name: "", host: "", port: 22, username: "",
      key_path: "", key_passphrase: null, poll_interval_secs: 60, squeue_user: null }];
  }
  function removeCluster(i) { cfg.clusters = cfg.clusters.filter((_, j) => j !== i); }
  async function save() {
    saving = true; error = "";
    try { await invoke("save_config", { newConfig: cfg }); }
    catch (e) { error = String(e); }
    finally { saving = false; }
  }
</script>

<div class="settings">
  <h2>Settings</h2>

  <div class="clusters-section">
    <h3>Clusters</h3>
    {#each cfg.clusters as c, i}
      <fieldset>
        <legend>Cluster {i + 1}</legend>
        <div class="field">
          <label for="name-{i}">Name</label>
          <input id="name-{i}" placeholder="name" bind:value={c.name} />
        </div>
        <div class="field">
          <label for="host-{i}">Host</label>
          <input id="host-{i}" placeholder="host" bind:value={c.host} />
        </div>
        <div class="field">
          <label for="port-{i}">Port</label>
          <input id="port-{i}" type="number" placeholder="port" bind:value={c.port} />
        </div>
        <div class="field">
          <label for="username-{i}">Username</label>
          <input id="username-{i}" placeholder="username" bind:value={c.username} />
        </div>
        <div class="field">
          <label for="key-path-{i}">Key path</label>
          <input id="key-path-{i}" placeholder="key path" bind:value={c.key_path} />
        </div>
        <div class="field">
          <label for="poll-{i}">Poll interval (secs)</label>
          <input id="poll-{i}" type="number" placeholder="poll secs" bind:value={c.poll_interval_secs} />
        </div>
        <div class="field">
          <label for="slurm-conf-{i}">Custom slurm.conf path (optional)</label>
          <input id="slurm-conf-{i}" placeholder="~/slurm-custom/slurm/custom_slurm.conf" bind:value={c.slurm_conf_path} />
        </div>
        <button class="danger" onclick={() => removeCluster(i)}>Remove</button>
      </fieldset>
    {/each}
    <button class="secondary" onclick={addCluster}>+ Add cluster</button>
  </div>

  <div class="notifications-section">
    <h3>Notifications</h3>
    <label>Notify only on states (comma-sep, empty = all):
      <input value={cfg.notifications.notify_states.join(",")}
        oninput={(e) => cfg.notifications.notify_states =
          e.target.value.split(",").map(s => s.trim()).filter(Boolean)} />
    </label>
    <label>Quiet start <input placeholder="HH:MM" bind:value={cfg.notifications.quiet_start} /></label>
    <label>Quiet end <input placeholder="HH:MM" bind:value={cfg.notifications.quiet_end} /></label>
  </div>

  <div class="general-section">
    <h3>General</h3>
    <label class="field-row">
      <input type="checkbox" checked={autostart} onchange={toggleAutostart} />
      Start slurm-monitor on login
    </label>
  </div>

  {#if error}<p class="err">{error}</p>{/if}
  <button disabled={saving} onclick={save}>{saving ? "Saving..." : "Save"}</button>
</div>
