use anyhow::Context;
use std::path::Path;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ClusterConfig {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    pub key_path: String,
    #[serde(default)]
    pub key_passphrase: Option<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub squeue_user: Option<String>,
    /// Optional path to a custom slurm.conf on the cluster (e.g. one whose
    /// AccountingStorageHost points at the real slurmdbd). When set, every
    /// SLURM command runs with SLURM_CONF=<path>.
    #[serde(default)]
    pub slurm_conf_path: Option<String>,
    /// Filesystems to watch in the disks panel. Defaults to /home and
    /// /scratch.
    #[serde(default = "default_disk_paths")]
    pub disk_paths: Vec<String>,
    /// Paths reported via per-user quota instead of df (opt-in, empty by
    /// default). `df` shows the whole filesystem's capacity, not a user's
    /// quota, so on clusters where /home has a quota (e.g. perun's 500 GB)
    /// list it here to show usage against the real limit. A path that
    /// yields no quota falls back to df.
    #[serde(default)]
    pub quota_paths: Vec<String>,
    /// Override for the quota query, with `{user}` and `{path}` placeholders.
    /// Defaults to Lustre's `lfs quota`, then plain `quota -s`. Set this when
    /// the cluster uses a different quota tool.
    #[serde(default)]
    pub quota_command: Option<String>,
}

fn default_disk_paths() -> Vec<String> {
    vec!["/home".into(), "/scratch".into()]
}

/// Default quota query: try Lustre's per-path `lfs quota`, else plain
/// `quota -s` (all filesystems). Both print the same tabular layout.
pub const DEFAULT_QUOTA_COMMAND: &str =
    "lfs quota -h -u {user} {path} 2>/dev/null || quota -s 2>/dev/null";

/// Shell prefix setting SLURM_CONF for a command, when a custom config
/// path is configured. The path is inserted unquoted: it must be
/// shell-safe (typical `~/slurm-custom/slurm/custom_slurm.conf` is).
pub fn slurm_env_prefix(slurm_conf: Option<&str>) -> String {
    match slurm_conf.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("SLURM_CONF={p} "),
        None => String::new(),
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NotificationConfig {
    #[serde(default)]
    pub notify_states: Vec<String>, // empty = notify on all transitions
    #[serde(default)]
    pub quiet_start: Option<String>, // "HH:MM"
    #[serde(default)]
    pub quiet_end: Option<String>,
    /// Lines of stderr to attach to FAILED/CANCELLED notifications. 0 = off.
    #[serde(default = "default_error_tail_lines")]
    pub attach_error_tail_lines: u32,
    /// Notify once when a job stays PENDING longer than this (seconds).
    /// 0 = off.
    #[serde(default = "default_pending_after_secs")]
    pub notify_pending_after_secs: u64,
    /// Notify once when a RUNNING job crosses this percent of its walltime.
    /// 0 = off.
    #[serde(default = "default_walltime_pct")]
    pub notify_walltime_pct: u32,
}

fn default_error_tail_lines() -> u32 {
    50
}

fn default_pending_after_secs() -> u64 {
    7200
}

fn default_walltime_pct() -> u32 {
    90
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_states: Vec::new(),
            quiet_start: None,
            quiet_end: None,
            attach_error_tail_lines: default_error_tail_lines(),
            notify_pending_after_secs: default_pending_after_secs(),
            notify_walltime_pct: default_walltime_pct(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Config {
    #[serde(default)]
    pub clusters: Vec<ClusterConfig>,
    #[serde(default)]
    pub notifications: NotificationConfig,
}

fn default_port() -> u16 {
    22
}

fn default_poll_interval() -> u64 {
    60
}

impl ClusterConfig {
    pub fn effective_squeue_user(&self) -> &str {
        self.squeue_user.as_deref().unwrap_or(&self.username)
    }

    pub fn quota_command_template(&self) -> &str {
        self.quota_command.as_deref().unwrap_or(DEFAULT_QUOTA_COMMAND)
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {path:?}"))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing config file {path:?}"))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self).context("serializing config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, text).with_context(|| format!("writing config {path:?}"))?;
        Ok(())
    }

    pub fn from_legacy(legacy_toml: &str) -> Option<Config> {
        #[derive(serde::Deserialize)]
        struct Legacy {
            host: String,
            #[serde(default = "default_port")]
            port: u16,
            username: String,
            key_path: String,
            #[serde(default)]
            key_passphrase: Option<String>,
            #[serde(default = "default_poll_interval")]
            poll_interval_secs: u64,
            #[serde(default)]
            squeue_user: Option<String>,
            #[serde(default)]
            slurm_conf_path: Option<String>,
            #[serde(default)]
            disk_paths: Option<Vec<String>>,
        }
        let legacy: Legacy = toml::from_str(legacy_toml).ok()?;
        Some(Config {
            clusters: vec![ClusterConfig {
                name: "default".to_string(),
                host: legacy.host,
                port: legacy.port,
                username: legacy.username,
                key_path: legacy.key_path,
                key_passphrase: legacy.key_passphrase,
                poll_interval_secs: legacy.poll_interval_secs,
                squeue_user: legacy.squeue_user,
                slurm_conf_path: legacy.slurm_conf_path,
                disk_paths: legacy.disk_paths.unwrap_or_else(default_disk_paths),
                quota_paths: Vec::new(),
                quota_command: None,
            }],
            notifications: NotificationConfig::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("create temp file");
        file.write_all(contents.as_bytes()).expect("write temp file");
        file
    }

    #[test]
    fn loads_full_config() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            name = "devana"
            host = "login.cluster.example"
            port = 2222
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            poll_interval_secs = 30
            squeue_user = "ivan"
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(cfg.clusters[0].host, "login.cluster.example");
        assert_eq!(cfg.clusters[0].port, 2222);
        assert_eq!(cfg.clusters[0].username, "jdoe");
        assert_eq!(cfg.clusters[0].key_path, "/home/jdoe/.ssh/id_ed25519");
        assert_eq!(cfg.clusters[0].key_passphrase, None);
        assert_eq!(cfg.clusters[0].poll_interval_secs, 30);
        assert_eq!(cfg.clusters[0].effective_squeue_user(), "ivan");
    }

    #[test]
    fn applies_defaults_when_optional_fields_missing() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            name = "devana"
            host = "login.cluster.example"
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(cfg.clusters[0].port, 22);
        assert_eq!(cfg.clusters[0].poll_interval_secs, 60);
        assert_eq!(cfg.clusters[0].effective_squeue_user(), "jdoe");
    }

    #[test]
    fn errors_on_missing_required_field() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            "#,
        );
        let result = Config::load(file.path());
        assert!(result.is_err(), "missing `host` should fail to load");
    }

    #[test]
    fn errors_on_missing_file() {
        let result = Config::load(std::path::Path::new("/no/such/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn loads_multi_cluster_config() {
        let file = write_temp_config(
            r#"
        [[clusters]]
        name = "devana"
        host = "login.devana.example"
        username = "jdoe"
        key_path = "/home/jdoe/.ssh/id_ed25519"

        [[clusters]]
        name = "lumi"
        host = "lumi.example"
        port = 2222
        username = "ivan"
        key_path = "/home/jdoe/.ssh/id_rsa"
        poll_interval_secs = 30
    "#,
        );
        let cfg = Config::load(file.path()).expect("load");
        assert_eq!(cfg.clusters.len(), 2);
        assert_eq!(cfg.clusters[0].name, "devana");
        assert_eq!(cfg.clusters[0].port, 22);
        assert_eq!(cfg.clusters[1].poll_interval_secs, 30);
        assert_eq!(cfg.clusters[1].effective_squeue_user(), "ivan");
    }

    #[test]
    fn migrates_legacy_single_cluster() {
        let legacy = r#"
            host = "login.cluster.example"
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            poll_interval_secs = 30
        "#;
        let cfg = Config::from_legacy(legacy).expect("migrate");
        assert_eq!(cfg.clusters.len(), 1);
        assert_eq!(cfg.clusters[0].name, "default");
        assert_eq!(cfg.clusters[0].host, "login.cluster.example");
        assert_eq!(cfg.clusters[0].poll_interval_secs, 30);
    }

    #[test]
    fn slurm_env_prefix_variants() {
        assert_eq!(slurm_env_prefix(None), "");
        assert_eq!(slurm_env_prefix(Some("")), "");
        assert_eq!(slurm_env_prefix(Some("  ")), "");
        assert_eq!(
            slurm_env_prefix(Some("~/slurm-custom/slurm/custom_slurm.conf")),
            "SLURM_CONF=~/slurm-custom/slurm/custom_slurm.conf "
        );
    }

    #[test]
    fn loads_slurm_conf_path_when_set() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            name = "perun"
            host = "login.cluster.example"
            username = "alice"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            slurm_conf_path = "~/slurm-custom/slurm/custom_slurm.conf"
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(
            cfg.clusters[0].slurm_conf_path.as_deref(),
            Some("~/slurm-custom/slurm/custom_slurm.conf")
        );
    }

    #[test]
    fn notification_defaults_match_documented_values() {
        let cfg = NotificationConfig::default();
        assert_eq!(cfg.attach_error_tail_lines, 50);
        assert_eq!(cfg.notify_pending_after_secs, 7200);
        assert_eq!(cfg.notify_walltime_pct, 90);
    }

    #[test]
    fn notification_thresholds_parse_from_toml() {
        let file = write_temp_config(
            r#"
            [notifications]
            attach_error_tail_lines = 10
            notify_pending_after_secs = 3600
            notify_walltime_pct = 75
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(cfg.notifications.attach_error_tail_lines, 10);
        assert_eq!(cfg.notifications.notify_pending_after_secs, 3600);
        assert_eq!(cfg.notifications.notify_walltime_pct, 75);
    }

    #[test]
    fn disk_paths_default_to_home_and_scratch() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            name = "devana"
            host = "login.cluster.example"
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(cfg.clusters[0].disk_paths, vec!["/home", "/scratch"]);
    }

    #[test]
    fn disk_paths_parse_custom_list() {
        let file = write_temp_config(
            r#"
            [[clusters]]
            name = "devana"
            host = "login.cluster.example"
            username = "jdoe"
            key_path = "/home/jdoe/.ssh/id_ed25519"
            disk_paths = ["/home/ivan", "/fast_scratch"]
            "#,
        );
        let cfg = Config::load(file.path()).expect("config should load");
        assert_eq!(
            cfg.clusters[0].disk_paths,
            vec!["/home/ivan", "/fast_scratch"]
        );
    }
}
