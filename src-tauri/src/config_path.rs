use crate::config::Config;
use std::path::PathBuf;
use tauri::Manager;

pub fn resolve(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.toml")
}

pub fn ui_state_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("ui-state.json")
}

pub fn load_or_migrate(app: &tauri::AppHandle) -> anyhow::Result<Config> {
    let path = resolve(app);
    if path.exists() {
        return Config::load(&path);
    }
    let legacy = PathBuf::from("config.toml"); // cwd (src-tauri) legacy location
    if legacy.exists() {
        if let Ok(text) = std::fs::read_to_string(&legacy) {
            if let Some(cfg) = Config::from_legacy(&text) {
                cfg.save(&path)?;
                tracing::info!("migrated legacy config.toml to {path:?}");
                return Ok(cfg);
            }
        }
    }
    Ok(Config {
        clusters: Vec::new(),
        notifications: Default::default(),
    })
}
