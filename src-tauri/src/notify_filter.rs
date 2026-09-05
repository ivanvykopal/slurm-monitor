use crate::config::NotificationConfig;

fn parse_hm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    Some((h.parse().ok()?, m.parse().ok()?))
}

fn in_quiet(now: (u8, u8), start: (u8, u8), end: (u8, u8)) -> bool {
    let (n, s, e) = (
        now.0 as u16 * 60 + now.1 as u16,
        start.0 as u16 * 60 + start.1 as u16,
        end.0 as u16 * 60 + end.1 as u16,
    );
    if s <= e {
        n >= s && n < e
    } else {
        n >= s || n < e // wraps midnight
    }
}

/// True when `now_hm` falls inside the configured quiet-hours window.
/// Applies to every notification, escalation notices included.
pub fn in_quiet_hours(cfg: &NotificationConfig, now_hm: (u8, u8)) -> bool {
    if let (Some(qs), Some(qe)) = (&cfg.quiet_start, &cfg.quiet_end) {
        if let (Some(s), Some(e)) = (parse_hm(qs), parse_hm(qe)) {
            return in_quiet(now_hm, s, e);
        }
    }
    false
}

pub fn should_notify(cfg: &NotificationConfig, to_state: &str, now_hm: (u8, u8)) -> bool {
    if !cfg.notify_states.is_empty() && !cfg.notify_states.iter().any(|s| s == to_state) {
        return false;
    }
    !in_quiet_hours(cfg, now_hm)
}

/// Parse a SLURM elapsed/walltime string ("MM", "MM:SS", "HH:MM:SS",
/// "D-HH:MM:SS", or "D-HH:MM:SS.uuu") into seconds. Returns None for
/// "N/A", "UNLIMITED", "NOT_SET", or anything unparsable.
pub fn parse_slurm_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || s == "N/A" || s == "UNLIMITED" || s == "NOT_SET" || s == "INVALID" {
        return None;
    }
    let s = s.split('.').next()?; // drop sub-second fraction
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let mut parts: Vec<u64> = Vec::new();
    for p in rest.split(':') {
        parts.push(p.parse::<u64>().ok()?);
    }
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    // The fields are big-endian: [HHH] [MM] SS — fill up to 3 slots.
    while parts.len() < 3 {
        parts.insert(0, 0);
    }
    Some(days * 86400 + parts[0] * 3600 + parts[1] * 60 + parts[2])
}

/// True when a pending job should trigger a "pending too long" notice:
/// it has been queued at least `threshold_secs` and its reason suggests
/// it is waiting on cluster resources or priority, not on the user
/// (a held job or a bad dependency is the user's to fix).
pub fn pending_too_long(elapsed_secs: u64, threshold_secs: u64, reason: &str) -> bool {
    if threshold_secs == 0 || elapsed_secs < threshold_secs {
        return false;
    }
    const WAITING_ON_CLUSTER: [&str; 5] = [
        "Priority", "Resources", "QOSMax", "AssocGrp", "Node",
    ];
    let r = reason.trim().trim_matches(|c| c == '(' || c == ')');
    WAITING_ON_CLUSTER.iter().any(|w| r.contains(w))
}

/// Percentage (0-100+) of walltime used by a running job, when both
/// elapsed time and time limit parse.
pub fn walltime_pct(elapsed: &str, limit: &str) -> Option<u64> {
    let e = parse_slurm_duration(elapsed)?;
    let l = parse_slurm_duration(limit)?;
    if e == 0 || l == 0 {
        return None;
    }
    Some(e * 100 / l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifies_when_state_list_empty() {
        let cfg = NotificationConfig::default();
        assert!(should_notify(&cfg, "COMPLETED", (12, 0)));
    }

    #[test]
    fn respects_state_allowlist() {
        let cfg = NotificationConfig {
            notify_states: vec!["FAILED".into()],
            ..Default::default()
        };
        assert!(should_notify(&cfg, "FAILED", (12, 0)));
        assert!(!should_notify(&cfg, "RUNNING", (12, 0)));
    }

    #[test]
    fn suppresses_during_quiet_hours() {
        let cfg = NotificationConfig {
            quiet_start: Some("22:00".into()),
            quiet_end: Some("08:00".into()),
            ..Default::default()
        };
        assert!(!should_notify(&cfg, "FAILED", (23, 0)));
        assert!(should_notify(&cfg, "FAILED", (12, 0)));
    }

    #[test]
    fn parses_slurm_durations() {
        assert_eq!(parse_slurm_duration("0:00"), Some(0));
        assert_eq!(parse_slurm_duration("01:23:45"), Some(5025));
        assert_eq!(parse_slurm_duration("1-00:00:00"), Some(86400));
        assert_eq!(parse_slurm_duration("2:30"), Some(150));
        assert_eq!(parse_slurm_duration("45"), Some(45));
        assert_eq!(parse_slurm_duration("01:23:45.123"), Some(5025));
        assert_eq!(parse_slurm_duration("N/A"), None);
        assert_eq!(parse_slurm_duration("UNLIMITED"), None);
        assert_eq!(parse_slurm_duration(""), None);
        assert_eq!(parse_slurm_duration("garbage"), None);
    }

    #[test]
    fn pending_too_long_requires_threshold_and_cluster_reason() {
        // Below threshold: no.
        assert!(!pending_too_long(100, 7200, "(Priority)"));
        // Threshold met + priority reason: yes.
        assert!(pending_too_long(7200, 7200, "(Priority)"));
        assert!(pending_too_long(9000, 7200, "(Resources)"));
        // Threshold met but reason is user-side (held job): no.
        assert!(!pending_too_long(9000, 7200, "(JobHeldUser)"));
        // Disabled feature: never.
        assert!(!pending_too_long(999_999, 0, "(Priority)"));
    }

    #[test]
    fn walltime_pct_computes_ratio() {
        assert_eq!(walltime_pct("00:54:00", "01:00:00"), Some(90));
        assert_eq!(walltime_pct("1-00:00:00", "2-00:00:00"), Some(50));
        assert_eq!(walltime_pct("02:00:00", "01:00:00"), Some(200)); // over limit
        assert_eq!(walltime_pct("N/A", "01:00:00"), None);
        assert_eq!(walltime_pct("00:10:00", "UNLIMITED"), None);
    }
}
