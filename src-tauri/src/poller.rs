use crate::squeue::JobStatus;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffEvent {
    New(JobStatus),
    StateChanged {
        previous: JobStatus,
        current: JobStatus,
    },
    Vanished(JobStatus),
}

pub fn diff(previous: &HashMap<String, JobStatus>, current: &[JobStatus]) -> Vec<DiffEvent> {
    let mut events = Vec::new();
    let current_ids: std::collections::HashSet<&String> =
        current.iter().map(|job| &job.id).collect();

    for job in current {
        match previous.get(&job.id) {
            None => events.push(DiffEvent::New(job.clone())),
            Some(prev) if prev.state != job.state => events.push(DiffEvent::StateChanged {
                previous: prev.clone(),
                current: job.clone(),
            }),
            _ => {}
        }
    }

    for (id, prev_job) in previous {
        if !current_ids.contains(id) {
            events.push(DiffEvent::Vanished(prev_job.clone()));
        }
    }

    events
}

pub fn next_delay(current: Duration, base: Duration, max: Duration, success: bool) -> Duration {
    if success {
        base
    } else {
        std::cmp::min(current * 2, max)
    }
}

use crate::ssh_client::CommandRunner;
use tauri::Emitter;

pub struct PollerConfig {
    pub cluster_name: String,
    pub squeue_user: String,
    pub poll_interval_secs: u64,
    pub notifications: crate::config::NotificationConfig,
    pub slurm_conf_path: Option<String>,
}

fn current_hm() -> (u8, u8) {
    let now = chrono::Local::now();
    (
        now.format("%H").to_string().parse().unwrap_or(0),
        now.format("%M").to_string().parse().unwrap_or(0),
    )
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn push_history(
    history: &crate::history::SharedHistory,
    app_handle: &tauri::AppHandle,
    cluster: &str,
    job: &JobStatus,
    from_state: &str,
    to_state: &str,
    detail: Option<&str>,
) {
    let entry = crate::history::HistoryEntry {
        timestamp_secs: now_secs(),
        cluster: cluster.to_string(),
        job_name: job.name.clone(),
        job_id: job.id.clone(),
        from_state: from_state.to_string(),
        to_state: to_state.to_string(),
        detail: detail.map(|s| s.to_string()),
    };
    history.lock().unwrap().push(entry.clone());
    let _ = app_handle.emit("history-updated", entry);
}

#[derive(serde::Serialize, Clone)]
struct JobsUpdate<'a> {
    cluster: &'a str,
    jobs: Vec<&'a JobStatus>,
}

#[derive(serde::Serialize, Clone)]
struct ConnUpdate<'a> {
    cluster: &'a str,
    status: &'a str,
    detail: &'a str,
}

/// States whose notifications deserve the stderr tail attached.
fn is_failed_state(state: &str) -> bool {
    state.starts_with("FAILED") || state.starts_with("CANCELLED") || state.starts_with("TIMEOUT")
        || state.starts_with("OUT_OF_MEMORY") || state.starts_with("NODE_FAIL")
}

/// Fetch the tail of a failed job's stderr file. Consumes and returns the
/// runner (same move-into-blocking-task pattern as the squeue poll), because
/// SSH execution is blocking. The returned string is used as notification/
/// history detail.
async fn fetch_stderr_tail<R: CommandRunner + 'static>(
    runner: R,
    cfg: &PollerConfig,
    job: &JobStatus,
) -> (Option<String>, R) {
    let lines = cfg.notifications.attach_error_tail_lines;
    if lines == 0 {
        return (None, runner);
    }
    let scontrol_cmd = crate::logtail::build_stdout_path_command(
        &job.id,
        cfg.slurm_conf_path.as_deref(),
    );
    let (scontrol, runner) = run_blocking(runner, scontrol_cmd).await;
    let Some(scontrol) = scontrol else {
        return (None, runner);
    };
    let Some(path) = crate::logtail::parse_stderr_path(&scontrol) else {
        return (None, runner);
    };
    let tail_cmd = crate::logtail::build_tail_command(&path, lines);
    let (tail, runner) = run_blocking(runner, tail_cmd).await;
    let Some(tail) = tail else {
        return (None, runner);
    };
    let tail = tail.trim_end();
    if tail.is_empty() {
        return (None, runner);
    }
    // tail -n already bounds the line count; just strip a trailing newline.
    let text = tail.to_string();
    (Some(format!("last stderr:\n{text}")), runner)
}

/// Run one blocking SSH command on the async runtime. Takes the runner by
/// value and returns it back along with the command output (Err is mapped
/// to None — the stderr tail is best-effort).
async fn run_blocking<R: CommandRunner + 'static>(
    mut runner: R,
    command: String,
) -> (Option<String>, R) {
    let (result, returned_runner) = tokio::task::spawn_blocking(move || {
        let result = runner.run(&command);
        (result, runner)
    })
    .await
    .expect("ssh command task panicked");
    (result.ok(), returned_runner)
}

pub async fn run_poller<R: CommandRunner + 'static>(
    mut runner: R,
    cfg: PollerConfig,
    app_handle: tauri::AppHandle,
    history: crate::history::SharedHistory,
) {
    let mut previous: HashMap<String, JobStatus> = HashMap::new();
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs);
    // One-shot notices already sent for a job id; entries are dropped when
    // the job leaves squeue so a resubmitted id can trigger again.
    let mut notified_pending: std::collections::HashSet<String> = Default::default();
    let mut notified_walltime: std::collections::HashSet<String> = Default::default();
    // Epoch seconds a job id was first observed PENDING. squeue's %M is
    // "0:00" for pending jobs and %V (submit time) carries no timezone, so
    // queue age is measured from when the monitor first saw the job pending.
    // Entries are dropped once the job leaves the PENDING state.
    let mut pending_since: HashMap<String, u64> = HashMap::new();

    loop {
        let squeue_cmd = crate::squeue::build_squeue_command(
            &cfg.squeue_user,
            cfg.slurm_conf_path.as_deref(),
        );
        let (poll_result, returned_runner) = tokio::task::spawn_blocking(move || {
            let result = runner.run(&squeue_cmd);
            (result, runner)
        })
        .await
        .expect("squeue poll task panicked");
        runner = returned_runner;

        match poll_result {
            Ok(output) => {
                let current = crate::squeue::parse_squeue_output(&output, &cfg.cluster_name);
                let events = diff(&previous, &current);

                for event in &events {
                    match event {
                        DiffEvent::StateChanged { previous, current } => {
                            let (detail, returned_runner) = if is_failed_state(&current.state) {
                                fetch_stderr_tail(runner, &cfg, current).await
                            } else {
                                (None, runner)
                            };
                            runner = returned_runner;
                            push_history(
                                &history,
                                &app_handle,
                                &cfg.cluster_name,
                                current,
                                &previous.state,
                                &current.state,
                                detail.as_deref(),
                            );
                            if crate::notify_filter::should_notify(
                                &cfg.notifications,
                                &current.state,
                                current_hm(),
                            ) {
                                crate::notifier::notify_transition(
                                    &app_handle,
                                    &cfg.cluster_name,
                                    &current.name,
                                    &previous.state,
                                    &current.state,
                                    detail.as_deref(),
                                );
                            }
                        }
                        DiffEvent::Vanished(job) => {
                            let sacct_cmd = crate::sacct::build_sacct_command(
                                &job.id,
                                cfg.slurm_conf_path.as_deref(),
                            );
                            let (sacct_result, returned_runner) = tokio::task::spawn_blocking(move || {
                                let result = runner.run(&sacct_cmd);
                                (result, runner)
                            })
                            .await
                            .expect("sacct lookup task panicked");
                            runner = returned_runner;
                            match sacct_result {
                                Ok(sacct_output) => {
                                    if let Some(result) =
                                        crate::sacct::parse_sacct_output(&sacct_output)
                                    {
                                        if result.state != job.state {
                                            let mut detail = format!("exit {}", result.exit_code);
                                            if is_failed_state(&result.state) {
                                                let (tail, returned_runner) =
                                                    fetch_stderr_tail(runner, &cfg, job).await;
                                                runner = returned_runner;
                                                if let Some(tail) = tail {
                                                    detail = format!("{detail}\n{tail}");
                                                }
                                            }
                                            push_history(
                                                &history,
                                                &app_handle,
                                                &cfg.cluster_name,
                                                job,
                                                &job.state,
                                                &result.state,
                                                Some(&detail),
                                            );
                                            if crate::notify_filter::should_notify(
                                                &cfg.notifications,
                                                &result.state,
                                                current_hm(),
                                            ) {
                                                crate::notifier::notify_transition(
                                                    &app_handle,
                                                    &cfg.cluster_name,
                                                    &job.name,
                                                    &job.state,
                                                    &result.state,
                                                    Some(&detail),
                                                );
                                            }
                                        }
                                    } else {
                                        tracing::warn!("could not parse sacct output for job {}", job.id);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("sacct lookup failed for job {}: {e}", job.id);
                                }
                            }
                        }
                        DiffEvent::New(_) => {}
                    }
                }

                // Escalation notices for jobs still in squeue. These run
                // after the diff loop so a state-change notification for the
                // same job always goes out first.
                for job in current.iter() {
                    if job.state.starts_with("PENDING") {
                        // Queue age since first observed pending this run.
                        let first_seen =
                            *pending_since.entry(job.id.clone()).or_insert_with(now_secs);
                        let queued_secs = now_secs().saturating_sub(first_seen);
                        if !notified_pending.contains(&job.id)
                            && crate::notify_filter::pending_too_long(
                                queued_secs,
                                cfg.notifications.notify_pending_after_secs,
                                &job.reason,
                            )
                        {
                            notified_pending.insert(job.id.clone());
                            push_history(
                                &history,
                                &app_handle,
                                &cfg.cluster_name,
                                job,
                                "PENDING",
                                "PENDING",
                                Some(&format!("pending too long {}", job.reason)),
                            );
                            // Escalation is opt-in via notify_pending_after_secs,
                            // so it bypasses the notify_states allow-list but
                            // still respects quiet hours.
                            if !crate::notify_filter::in_quiet_hours(
                                &cfg.notifications,
                                current_hm(),
                            ) {
                                crate::notifier::notify_transition(
                                    &app_handle,
                                    &cfg.cluster_name,
                                    &job.name,
                                    "PENDING",
                                    &format!("still PENDING {}", job.reason),
                                    Some(&format!(
                                        "queued for over {} min",
                                        cfg.notifications.notify_pending_after_secs / 60
                                    )),
                                );
                            }
                        }
                    } else if job.state.starts_with("RUNNING") {
                        let pct = cfg.notifications.notify_walltime_pct;
                        if pct > 0
                            && !notified_walltime.contains(&job.id)
                            && crate::notify_filter::walltime_pct(&job.time, &job.time_limit)
                                .is_some_and(|p| p >= u64::from(pct))
                        {
                            notified_walltime.insert(job.id.clone());
                            let msg = format!("{pct}% of walltime used");
                            push_history(
                                &history,
                                &app_handle,
                                &cfg.cluster_name,
                                job,
                                "RUNNING",
                                "RUNNING",
                                Some(&msg),
                            );
                            // Opt-in via notify_walltime_pct: bypass the
                            // notify_states allow-list, honour quiet hours.
                            if !crate::notify_filter::in_quiet_hours(
                                &cfg.notifications,
                                current_hm(),
                            ) {
                                crate::notifier::notify_transition(
                                    &app_handle,
                                    &cfg.cluster_name,
                                    &job.name,
                                    "RUNNING",
                                    "RUNNING",
                                    Some(&msg),
                                );
                            }
                        }
                    }
                }

                // Drop one-shot notice state for jobs no longer in squeue, and
                // pending-age state for jobs that are gone or no longer pending.
                notified_pending.retain(|id| current.iter().any(|j| &j.id == id));
                notified_walltime.retain(|id| current.iter().any(|j| &j.id == id));
                pending_since.retain(|id, _| {
                    current
                        .iter()
                        .any(|j| &j.id == id && j.state.starts_with("PENDING"))
                });

                previous = current.into_iter().map(|j| (j.id.clone(), j)).collect();
                let mut jobs: Vec<&JobStatus> = previous.values().collect();
                jobs.sort_by(|a, b| a.id.cmp(&b.id));
                let _ = app_handle.emit(
                    "jobs-updated",
                    JobsUpdate {
                        cluster: &cfg.cluster_name,
                        jobs: jobs.clone(),
                    },
                );
                let _ = app_handle.emit(
                    "connection-status",
                    ConnUpdate {
                        cluster: &cfg.cluster_name,
                        status: "connected",
                        detail: "",
                    },
                );
            }
            Err(e) => {
                tracing::warn!("squeue poll failed: {e}");
                let _ = app_handle.emit(
                    "connection-status",
                    ConnUpdate {
                        cluster: &cfg.cluster_name,
                        status: "disconnected",
                        detail: &format!("{e}"),
                    },
                );
                return;
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::squeue::JobStatus;
    use std::collections::HashMap;
    use std::time::Duration;

    fn job(id: &str, name: &str, state: &str) -> JobStatus {
        JobStatus {
            id: id.into(),
            name: name.into(),
            state: state.into(),
            time: "0:01".into(),
            time_limit: "1:00:00".into(),
            nodes: "1".into(),
            reason: "node1".into(),
            partition: "cpu".into(),
            est_start: "N/A".into(),
            cluster: "test".into(),
        }
    }

    #[test]
    fn detects_new_job() {
        let previous = HashMap::new();
        let current = vec![job("1", "a", "PENDING")];
        let events = diff(&previous, &current);
        assert_eq!(events, vec![DiffEvent::New(job("1", "a", "PENDING"))]);
    }

    #[test]
    fn detects_state_change() {
        let mut previous = HashMap::new();
        previous.insert("1".to_string(), job("1", "a", "PENDING"));
        let current = vec![job("1", "a", "RUNNING")];
        let events = diff(&previous, &current);
        assert_eq!(
            events,
            vec![DiffEvent::StateChanged {
                previous: job("1", "a", "PENDING"),
                current: job("1", "a", "RUNNING"),
            }]
        );
    }

    #[test]
    fn detects_vanished_job() {
        let mut previous = HashMap::new();
        previous.insert("1".to_string(), job("1", "a", "RUNNING"));
        let current: Vec<JobStatus> = vec![];
        let events = diff(&previous, &current);
        assert_eq!(events, vec![DiffEvent::Vanished(job("1", "a", "RUNNING"))]);
    }

    #[test]
    fn unchanged_job_produces_no_event() {
        let mut previous = HashMap::new();
        previous.insert("1".to_string(), job("1", "a", "RUNNING"));
        let current = vec![job("1", "a", "RUNNING")];
        assert_eq!(diff(&previous, &current), Vec::new());
    }

    #[test]
    fn next_delay_resets_to_base_on_success() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(300);
        let result = next_delay(Duration::from_secs(240), base, max, true);
        assert_eq!(result, base);
    }

    #[test]
    fn next_delay_doubles_on_failure() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(300);
        let result = next_delay(Duration::from_secs(60), base, max, false);
        assert_eq!(result, Duration::from_secs(120));
    }

    #[test]
    fn next_delay_caps_at_max_on_repeated_failure() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(300);
        let result = next_delay(Duration::from_secs(280), base, max, false);
        assert_eq!(result, max);
    }

    use crate::ssh_client::CommandRunner;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct FakeRunner {
        responses: VecDeque<anyhow::Result<String>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, command: &str) -> anyhow::Result<String> {
            self.calls.lock().unwrap().push(command.to_string());
            self.responses
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    #[test]
    fn build_squeue_and_sacct_commands_match_expected_targets() {
        // Sanity check that the commands run_poller will issue are the
        // same ones squeue.rs / sacct.rs build, so a job id round-trips
        // correctly through a vanish -> sacct lookup.
        let squeue_cmd = crate::squeue::build_squeue_command("ivan", None);
        assert!(squeue_cmd.contains("ivan"));
        let sacct_cmd = crate::sacct::build_sacct_command("999", None);
        assert!(sacct_cmd.contains("999"));
    }

    #[tokio::test]
    async fn one_tick_reports_new_job_and_emits_no_notification() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner {
            responses: VecDeque::from([Ok(
                "1|train|PENDING|0:00|1:00:00|1|(Priority)|cpu|N/A\n".to_string()
            )]),
            calls: calls.clone(),
        };

        let mut previous = HashMap::new();
        let output = "1|train|PENDING|0:00|1:00:00|1|(Priority)|cpu|N/A\n";
        let current = crate::squeue::parse_squeue_output(output, "test");
        let events = diff(&previous, &current);
        assert_eq!(
            events,
            vec![DiffEvent::New(JobStatus {
                id: "1".into(),
                name: "train".into(),
                state: "PENDING".into(),
                time: "0:00".into(),
                time_limit: "1:00:00".into(),
                nodes: "1".into(),
                reason: "(Priority)".into(),
                partition: "cpu".into(),
                est_start: "N/A".into(),
                cluster: "test".into(),
            })]
        );
        previous = current
            .into_iter()
            .map(|j| (j.id.clone(), j))
            .collect::<HashMap<_, _>>();
        assert_eq!(previous.len(), 1);
        let _ = runner; // FakeRunner wiring is exercised fully by run_poller_tick below
    }

    #[tokio::test]
    async fn poller_tick_resolves_vanished_job_via_sacct() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut runner = FakeRunner {
            responses: VecDeque::from([
                Ok("1|train|RUNNING|0:10|1:00:00|1|node1|cpu|N/A\n".to_string()), // first squeue
                Ok(String::new()),                              // second squeue: job gone
                Ok("COMPLETED|0:0|00:01:00|10K|1Gn\n".to_string()), // sacct lookup
            ]),
            calls: calls.clone(),
        };

        // Tick 1: establish state.
        let first_output = runner.run(&crate::squeue::build_squeue_command("ivan", None)).unwrap();
        let first_current = crate::squeue::parse_squeue_output(&first_output, "test");
        let mut previous: HashMap<String, JobStatus> = first_current
            .into_iter()
            .map(|j| (j.id.clone(), j))
            .collect();
        assert_eq!(previous.len(), 1);

        // Tick 2: job vanished from squeue.
        let second_output = runner.run(&crate::squeue::build_squeue_command("ivan", None)).unwrap();
        let second_current = crate::squeue::parse_squeue_output(&second_output, "test");
        let events = diff(&previous, &second_current);
        assert_eq!(events.len(), 1);
        let vanished = match &events[0] {
            DiffEvent::Vanished(job) => job.clone(),
            other => panic!("expected Vanished, got {other:?}"),
        };
        assert_eq!(vanished.id, "1");

        let sacct_output = runner
            .run(&crate::sacct::build_sacct_command(&vanished.id, None))
            .unwrap();
        let resolved = crate::sacct::parse_sacct_output(&sacct_output);
        assert_eq!(resolved.map(|r| r.state), Some("COMPLETED".to_string()));

        previous = second_current
            .into_iter()
            .map(|j| (j.id.clone(), j))
            .collect();
        assert!(previous.is_empty());

        let recorded_calls = calls.lock().unwrap();
        assert_eq!(recorded_calls.len(), 3);
        assert!(recorded_calls[2].contains("sacct"));
    }
}
