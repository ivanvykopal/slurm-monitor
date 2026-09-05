use crate::config::Config;
use crate::config_path;
use crate::history::SharedHistory;
use crate::supervisor::Supervisor;
use notify::{RecursiveMode, Watcher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Manager;

pub fn spawn(app: tauri::AppHandle) {
    let path = config_path::resolve(&app);
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w, Err(e) => { tracing::warn!("watcher init failed: {e}"); return; }
        };
        if let Some(dir) = path.parent() {
            if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                tracing::warn!("watcher watch() failed: {e}");
                return;
            }
        }
        let file_name = path.file_name().map(|n| n.to_owned());
        loop {
            match rx.recv() {
                Ok(Ok(event))
                    if event
                        .paths
                        .iter()
                        .any(|p| p.file_name() == file_name.as_deref()) =>
                {
                    std::thread::sleep(Duration::from_millis(200)); // debounce partial writes
                    if let Ok(cfg) = Config::load(&path) {
                        tracing::info!("config changed on disk; reloading");
                        let cfg_state = app.state::<Mutex<Config>>();
                        *cfg_state.lock().unwrap() = cfg.clone();
                        let sup = app.state::<Arc<Mutex<Supervisor>>>();
                        let history = app.state::<SharedHistory>();
                        sup.lock()
                            .unwrap()
                            .restart(app.clone(), cfg.clusters, cfg.notifications, history.inner().clone());
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}
