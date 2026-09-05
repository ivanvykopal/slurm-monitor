use crate::config::Config;
use crate::config_path;
use crate::history::{HistoryEntry, SharedHistory};
use crate::ssh_client::{CommandRunner, SshClient};
use crate::supervisor::Supervisor;
use crate::ui_state::UiState;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
pub fn get_config(cfg: State<'_, Mutex<Config>>) -> Config {
    cfg.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_config(
    app: tauri::AppHandle,
    new_config: Config,
    cfg: State<'_, Mutex<Config>>,
    supervisor: State<'_, Arc<Mutex<Supervisor>>>,
    history: State<'_, SharedHistory>,
) -> Result<(), String> {
    let path = config_path::resolve(&app);
    new_config.save(&path).map_err(|e| e.to_string())?;
    *cfg.lock().unwrap() = new_config.clone();
    supervisor.lock().unwrap().restart(
        app.clone(),
        new_config.clusters,
        new_config.notifications,
        history.inner().clone(),
    );
    Ok(())
}

#[tauri::command]
pub fn cancel_job(
    job_id: String,
    cluster: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<(), String> {
    let cluster_cfg = {
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == cluster)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    let mut client = SshClient::connect(
        &cluster_cfg.host,
        cluster_cfg.port,
        &cluster_cfg.username,
        &PathBuf::from(&cluster_cfg.key_path),
        cluster_cfg.key_passphrase.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    client
        .run(&crate::scancel::build_scancel_command(
            &job_id,
            cluster_cfg.slurm_conf_path.as_deref(),
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn tail_log(
    job_id: String,
    cluster: String,
    lines: u32,
    stream: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<String, String> {
    let cluster_cfg = {
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == cluster)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    let mut client = SshClient::connect(
        &cluster_cfg.host,
        cluster_cfg.port,
        &cluster_cfg.username,
        &PathBuf::from(&cluster_cfg.key_path),
        cluster_cfg.key_passphrase.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let scontrol = client
        .run(&crate::logtail::build_stdout_path_command(
            &job_id,
            cluster_cfg.slurm_conf_path.as_deref(),
        ))
        .map_err(|e| e.to_string())?;
    let path = match stream.as_str() {
        "stderr" => crate::logtail::parse_stderr_path(&scontrol)
            .ok_or_else(|| "could not resolve StdErr path".to_string())?,
        _ => crate::logtail::parse_stdout_path(&scontrol)
            .ok_or_else(|| "could not resolve StdOut path".to_string())?,
    };
    client
        .run(&crate::logtail::build_tail_command(&path, lines))
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct ClusterHealth {
    partitions: Vec<crate::sinfo::PartitionInfo>,
    fair_share: Option<String>,
}

#[tauri::command]
pub fn cluster_health(
    cluster: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<ClusterHealth, String> {
    let cluster_cfg = {
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == cluster)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    let mut client = SshClient::connect(
        &cluster_cfg.host,
        cluster_cfg.port,
        &cluster_cfg.username,
        &PathBuf::from(&cluster_cfg.key_path),
        cluster_cfg.key_passphrase.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    let mut partitions = crate::sinfo::parse(
        &client
            .run(&crate::sinfo::build_command(cluster_cfg.slurm_conf_path.as_deref()))
            .map_err(|e| e.to_string())?,
    );
    // Queue depth per partition — the where-to-submit signal. Degrades
    // gracefully: counts stay 0 when the query fails.
    match client.run(&crate::sinfo::build_squeue_partition_command(
        cluster_cfg.slurm_conf_path.as_deref(),
    )) {
        Ok(sq) => crate::sinfo::merge_job_counts(&mut partitions, &sq),
        Err(e) => tracing::warn!("[{cluster}] health: squeue partition counts failed: {e}"),
    }
    // Most idle nodes first: the best place to submit right now.
    partitions.sort_by_key(|p| std::cmp::Reverse(crate::sinfo::idle_nodes(&p.nodes).unwrap_or(0)));
    let fair_share = client
        .run(&crate::sshare::build_command(
            cluster_cfg.effective_squeue_user(),
            cluster_cfg.slurm_conf_path.as_deref(),
        ))
        .ok()
        .and_then(|o| crate::sshare::parse(&o));
    Ok(ClusterHealth {
        partitions,
        fair_share,
    })
}

#[tauri::command]
pub fn hide_window(window: tauri::Window) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct ClusterProjects {
    projects: Vec<crate::projects::ProjectInfo>,
}

#[tauri::command]
pub async fn cluster_projects(
    cluster: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<ClusterProjects, String> {
    let cluster_cfg = {
        let name = cluster.clone();
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    // SSH work is blocking (up to 30s timeouts); run it off the main
    // thread so the webview stays responsive while we fetch.
    let started = std::time::Instant::now();
    let cluster_name = cluster.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<ClusterProjects> {
        let mut client = SshClient::connect(
            &cluster_cfg.host,
            cluster_cfg.port,
            &cluster_cfg.username,
            &PathBuf::from(&cluster_cfg.key_path),
            cluster_cfg.key_passphrase.as_deref(),
        )?;
        tracing::info!("[{cluster_name}] projects: ssh connected");

        let output = client.run(&crate::projects::build_command(
            cluster_cfg.effective_squeue_user(),
            cluster_cfg.slurm_conf_path.as_deref(),
        ))?;
        tracing::info!(
            "[{cluster_name}] projects: sshare took {:?}, {} bytes",
            started.elapsed(),
            output.len()
        );
        let mut projects = crate::projects::parse(&output);

        // Drop expired allocations: an account whose association GrpTRES node
        // limit is 0 can no longer run jobs. Failures degrade gracefully —
        // filter_expired keeps every project when the query returns nothing.
        match client.run(&crate::projects::build_assoc_command(
            cluster_cfg.effective_squeue_user(),
            cluster_cfg.slurm_conf_path.as_deref(),
        )) {
            Ok(assoc_out) => crate::projects::filter_expired(&mut projects, &assoc_out),
            Err(e) => tracing::warn!("[{cluster_name}] projects: sacctmgr assoc failed: {e}"),
        }

        // Enrich with per-project / per-user GPU-hours from sreport.
        // Failures degrade gracefully: the panel still shows sshare data.
        if !projects.is_empty() {
            let accounts = projects
                .iter()
                .map(|p| p.account.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let start = std::time::Instant::now();
            match client.run(&crate::projects::build_sreport_command(
                &accounts,
                cluster_cfg.slurm_conf_path.as_deref(),
            )) {
                Ok(sreport_out) => {
                    tracing::info!(
                        "[{cluster_name}] projects: sreport took {:?}, {} bytes",
                        start.elapsed(),
                        sreport_out.len()
                    );
                    crate::projects::merge_sreport(&mut projects, &sreport_out);
                }
                Err(e) => {
                    tracing::warn!("[{cluster_name}] projects: sreport failed: {e}");
                }
            }

            // Allocated GPU-hours cap from the per-account QOS GrpTRESMins.
            // Failures degrade gracefully: allocated stays "0" (percentage hidden).
            let start = std::time::Instant::now();
            match client.run(&crate::projects::build_qos_command(
                &accounts,
                cluster_cfg.slurm_conf_path.as_deref(),
            )) {
                Ok(qos_out) => {
                    tracing::info!(
                        "[{cluster_name}] projects: sacctmgr qos took {:?}, {} bytes",
                        start.elapsed(),
                        qos_out.len()
                    );
                    crate::projects::merge_qos(&mut projects, &qos_out);
                }
                Err(e) => {
                    tracing::warn!("[{cluster_name}] projects: sacctmgr qos failed: {e}");
                }
            }

            // Last-30-day usage for the daily burn rate. Degrades
            // gracefully: *_hours_30d stay "0" (burn rate hidden).
            let start = std::time::Instant::now();
            match client.run(&crate::projects::build_sreport_30d_command(
                &accounts,
                cluster_cfg.slurm_conf_path.as_deref(),
            )) {
                Ok(sreport_out) => {
                    tracing::info!(
                        "[{cluster_name}] projects: sreport 30d took {:?}, {} bytes",
                        start.elapsed(),
                        sreport_out.len()
                    );
                    crate::projects::merge_sreport_30d(&mut projects, &sreport_out);
                }
                Err(e) => {
                    tracing::warn!("[{cluster_name}] projects: sreport 30d failed: {e}");
                }
            }
        }

        Ok(ClusterProjects { projects })
    })
    .await
    .map_err(|e| format!("projects task failed: {e}"))?
    .map_err(|e| e.to_string())?;
    tracing::info!("[{cluster}] projects: total {:?}", started.elapsed());
    Ok(result)
}

#[derive(serde::Serialize)]
pub struct ClusterEfficiency {
    jobs: Vec<crate::efficiency::JobEfficiency>,
}

/// Finished jobs of the last 7 days with utilization metrics (seff-style).
#[tauri::command]
pub async fn cluster_efficiency(
    cluster: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<ClusterEfficiency, String> {
    let cluster_cfg = {
        let name = cluster.clone();
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<ClusterEfficiency> {
        let mut client = SshClient::connect(
            &cluster_cfg.host,
            cluster_cfg.port,
            &cluster_cfg.username,
            &PathBuf::from(&cluster_cfg.key_path),
            cluster_cfg.key_passphrase.as_deref(),
        )?;
        let output = client.run(&crate::efficiency::build_command(
            cluster_cfg.effective_squeue_user(),
            cluster_cfg.slurm_conf_path.as_deref(),
        ))?;
        Ok(ClusterEfficiency {
            jobs: crate::efficiency::parse(&output),
        })
    })
    .await
    .map_err(|e| format!("efficiency task failed: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(result)
}

#[derive(serde::Serialize)]
pub struct ClusterDisks {
    disks: Vec<crate::disks::DiskUsage>,
}

/// Filesystems already notified as ≥90% full, per cluster. Cleared per
/// filesystem when usage falls back under 85% (hysteresis handled in
/// disks::crossing_threshold).
static DISK_NOTIFIED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::collections::HashSet<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[tauri::command]
pub async fn cluster_disks(
    app: tauri::AppHandle,
    cluster: String,
    cfg: State<'_, Mutex<Config>>,
) -> Result<ClusterDisks, String> {
    let cluster_cfg = {
        let name = cluster.clone();
        cfg.lock()
            .unwrap()
            .clusters
            .iter()
            .find(|c| c.name == name)
            .cloned()
            .ok_or_else(|| format!("unknown cluster {cluster}"))?
    };
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<ClusterDisks> {
        let mut client = SshClient::connect(
            &cluster_cfg.host,
            cluster_cfg.port,
            &cluster_cfg.username,
            &PathBuf::from(&cluster_cfg.key_path),
            cluster_cfg.key_passphrase.as_deref(),
        )?;
        let mut disks = Vec::new();

        // Quota-reported paths (opt-in). Show usage against the per-user
        // quota, not the whole filesystem; fall back to df for a path whose
        // quota query fails or reports no limit.
        let template = cluster_cfg.quota_command_template();
        for path in &cluster_cfg.quota_paths {
            let via_quota = client
                .run(&crate::quota::build_command(
                    &cluster_cfg.username,
                    path,
                    template,
                ))
                .ok()
                .and_then(|out| crate::quota::parse(&out, path));
            match via_quota {
                Some(d) => disks.push(d),
                None => {
                    tracing::info!("[{}] disks: no quota for {path}, using df", cluster_cfg.name);
                    if let Ok(out) =
                        client.run(&crate::disks::build_command(std::slice::from_ref(path)))
                    {
                        disks.extend(crate::disks::parse(&out));
                    }
                }
            }
        }

        // df-reported paths: everything in disk_paths not already covered by
        // a quota path.
        let df_paths: Vec<String> = cluster_cfg
            .disk_paths
            .iter()
            .filter(|p| !cluster_cfg.quota_paths.contains(p))
            .cloned()
            .collect();
        if !df_paths.is_empty() {
            let output = client.run(&crate::disks::build_command(&df_paths))?;
            disks.extend(crate::disks::parse(&output));
        }

        Ok(ClusterDisks { disks })
    })
    .await
    .map_err(|e| format!("disks task failed: {e}"))?
    .map_err(|e| e.to_string())?;

    // Notify on filesystems crossing 90% since the last check.
    let mut notified = DISK_NOTIFIED.lock().unwrap();
    let entry = notified.entry(cluster.clone()).or_default();
    for d in crate::disks::crossing_threshold(&result.disks, entry) {
        crate::notifier::notify_disk_almost_full(&app, &cluster, &d.filesystem, d.used_pct);
    }

    Ok(result)
}

#[tauri::command]
pub fn get_ui_state(app: tauri::AppHandle) -> Result<UiState, String> {
    let path = config_path::ui_state_path(&app);
    if path.exists() {
        UiState::load(&path).map_err(|e| e.to_string())
    } else {
        Ok(UiState::default())
    }
}

#[tauri::command]
pub fn save_ui_state(state: UiState, app: tauri::AppHandle) -> Result<(), String> {
    let path = config_path::ui_state_path(&app);
    state.save(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_autostart_state(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart_state(enable: bool, app: tauri::AppHandle) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if enable {
        autolaunch.enable().map_err(|e| e.to_string())
    } else {
        autolaunch.disable().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_history(history: State<'_, SharedHistory>, count: usize) -> Vec<HistoryEntry> {
    history.lock().unwrap().recent(count)
}
