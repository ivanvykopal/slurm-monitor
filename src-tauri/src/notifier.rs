use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub fn notify_transition(
    app: &AppHandle,
    cluster: &str,
    job_name: &str,
    from_state: &str,
    to_state: &str,
    detail: Option<&str>,
) {
    let extra = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    let body = format!("[{cluster}] {job_name}: {from_state} -> {to_state}{extra}");
    if let Err(e) = app
        .notification()
        .builder()
        .title("SLURM job update")
        .body(body)
        .show()
    {
        tracing::warn!("failed to show notification: {e}");
    }
}
