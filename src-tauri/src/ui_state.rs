use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub view_mode: String,
    #[serde(default)]
    pub collapsed_clusters: Vec<String>,
    #[serde(default)]
    pub open_health: Vec<String>,
    #[serde(default)]
    pub open_projects: Vec<String>,
    #[serde(default)]
    pub open_efficiency: Vec<String>,
    #[serde(default)]
    pub open_disks: Vec<String>,
}

impl UiState {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading ui state {path:?}"))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let text = serde_json::to_string_pretty(self).context("serializing ui state")?;
        std::fs::write(path, text).with_context(|| format!("writing ui state {path:?}"))?;
        Ok(())
    }
}
