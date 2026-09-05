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
}

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
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            notify_states: Vec::new(),
            quiet_start: None,
            quiet_end: None,
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
}
