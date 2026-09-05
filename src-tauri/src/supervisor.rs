use crate::config::{ClusterConfig, NotificationConfig};
use crate::history::SharedHistory;
use crate::poller::{self, PollerConfig};
use crate::ssh_client::SshClient;
use std::path::PathBuf;
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::Emitter;

pub struct Supervisor {
    handles: Vec<JoinHandle<()>>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    pub fn start(
        &mut self,
        app: tauri::AppHandle,
        clusters: Vec<ClusterConfig>,
        notifications: NotificationConfig,
        history: SharedHistory,
    ) {
        for cluster in clusters {
            let app = app.clone();
            let notifications = notifications.clone();
            let history = history.clone();
            self.handles.push(tauri::async_runtime::spawn(supervise_one(
                app,
                cluster,
                notifications,
                history,
            )));
        }
    }

    pub fn stop(&mut self) {
        for h in self.handles.drain(..) {
            h.abort();
        }
    }

    pub fn restart(
        &mut self,
        app: tauri::AppHandle,
        clusters: Vec<ClusterConfig>,
        notifications: NotificationConfig,
        history: SharedHistory,
    ) {
        self.stop();
        self.start(app, clusters, notifications, history);
    }
}

/// Categorize a connection error into a user-facing detail string.
fn categorize(err: &anyhow::Error) -> &'static str {
    let s = err.to_string().to_lowercase();
    if s.contains("auth") {
        "auth failed"
    } else if s.contains("connect") || s.contains("network") || s.contains("timed out") {
        "network / host unreachable"
    } else {
        "connection error"
    }
}

async fn supervise_one(
    app: tauri::AppHandle,
    cluster: ClusterConfig,
    notifications: NotificationConfig,
    history: SharedHistory,
) {
    let key_path = PathBuf::from(&cluster.key_path);
    let base = Duration::from_secs(5);
    let max = Duration::from_secs(300);
    let mut delay = base;
    #[derive(serde::Serialize, Clone)]
    struct ConnUpdate<'a> {
        cluster: &'a str,
        status: &'a str,
        detail: &'a str,
        next_retry_secs: u64,
    }
    loop {
        match SshClient::connect(
            &cluster.host,
            cluster.port,
            &cluster.username,
            &key_path,
            cluster.key_passphrase.as_deref(),
        ) {
            Ok(client) => {
                delay = base;
                let _ = app.emit(
                    "connection-status",
                    ConnUpdate {
                        cluster: &cluster.name,
                        status: "connected",
                        detail: "",
                        next_retry_secs: 0,
                    },
                );
                let pcfg = PollerConfig {
                    cluster_name: cluster.name.clone(),
                    squeue_user: cluster.effective_squeue_user().to_string(),
                    poll_interval_secs: cluster.poll_interval_secs,
                    notifications: notifications.clone(),
                    slurm_conf_path: cluster.slurm_conf_path.clone(),
                };
                poller::run_poller(client, pcfg, app.clone(), history.clone()).await;
            }
            Err(e) => {
                tracing::error!("[{}] SSH connect failed: {e:?}", cluster.name);
                let _ = app.emit(
                    "connection-status",
                    ConnUpdate {
                        cluster: &cluster.name,
                        status: "disconnected",
                        detail: categorize(&e),
                        next_retry_secs: delay.as_secs(),
                    },
                );
            }
        }
        tokio::time::sleep(delay).await;
        delay = std::cmp::min(delay * 2, max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_detects_auth_failure() {
        let err = anyhow::anyhow!("authenticating as bob with key \"id_rsa\": authentication failed");
        assert_eq!(categorize(&err), "auth failed");
    }

    #[test]
    fn categorize_detects_network_unreachable() {
        let err = anyhow::anyhow!("connecting to host:22: connection refused");
        assert_eq!(categorize(&err), "network / host unreachable");
    }

    #[test]
    fn categorize_detects_timeout_as_network() {
        let err = anyhow::anyhow!("ssh handshake failed: operation timed out");
        assert_eq!(categorize(&err), "network / host unreachable");
    }

    #[test]
    fn categorize_falls_back_to_generic_connection_error() {
        let err = anyhow::anyhow!("something completely unexpected happened");
        assert_eq!(categorize(&err), "connection error");
    }
}
