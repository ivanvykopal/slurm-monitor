use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub fn notify_disk_almost_full(
    app: &AppHandle,
    cluster: &str,
    filesystem: &str,
    used_pct: Option<u8>,
) {
    let pct = used_pct.map(|p| format!(" ({p}% used)")).unwrap_or_default();
    let body = format!("[{cluster}] {filesystem} is almost full{pct}");
    if let Err(e) = app
        .notification()
        .builder()
        .title("Disk usage warning")
        .body(body)
        .show()
    {
        tracing::warn!("failed to show notification: {e}");
    }
}

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
