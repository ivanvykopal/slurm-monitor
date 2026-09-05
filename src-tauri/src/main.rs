#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod commands;
mod config;
mod config_path;
mod config_watch;
mod disks;
mod efficiency;
mod history;
mod logtail;
mod notifier;
mod notify_filter;
mod poller;
mod projects;
mod quota;
mod sacct;
mod scancel;
mod sinfo;
mod squeue;
mod sshare;
mod ssh_client;
mod supervisor;
mod ui_state;

use std::path::PathBuf;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tracing_subscriber::EnvFilter;

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("slurm-monitor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let visible = window.is_visible().unwrap_or(false);
                    if visible {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn init_logging(app: &tauri::App) {
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(log_dir, "slurm-monitor.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so logging stays active for the process lifetime.
    Box::leak(Box::new(guard));

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .with_writer(non_blocking)
        .init();
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"])
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::cancel_job,
            commands::tail_log,
            commands::cluster_health,
            commands::cluster_projects,
            commands::cluster_efficiency,
            commands::cluster_disks,
            commands::hide_window,
            commands::get_ui_state,
            commands::save_ui_state,
            commands::get_autostart_state,
            commands::set_autostart_state,
            commands::get_history
        ])
        .setup(|app| {
            init_logging(app);
            setup_tray(app)?;

            let cfg = config_path::load_or_migrate(&app.handle()).unwrap_or_else(|e| {
                tracing::error!("config load failed: {e:?}");
                crate::config::Config {
                    clusters: vec![],
                    notifications: Default::default(),
                }
            });
            let supervisor = std::sync::Arc::new(std::sync::Mutex::new(supervisor::Supervisor::new()));
            let history: crate::history::SharedHistory = std::sync::Arc::new(std::sync::Mutex::new(crate::history::History::default()));
            supervisor.lock().unwrap().start(
                app.handle().clone(),
                cfg.clusters.clone(),
                cfg.notifications.clone(),
                history.clone(),
            );
            app.manage(supervisor.clone());
            app.manage(std::sync::Mutex::new(cfg));
            app.manage(history.clone());

            config_watch::spawn(app.handle().clone());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
