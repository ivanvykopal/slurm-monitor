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

pub fn should_notify(cfg: &NotificationConfig, to_state: &str, now_hm: (u8, u8)) -> bool {
    if !cfg.notify_states.is_empty() && !cfg.notify_states.iter().any(|s| s == to_state) {
        return false;
    }
    if let (Some(qs), Some(qe)) = (&cfg.quiet_start, &cfg.quiet_end) {
        if let (Some(s), Some(e)) = (parse_hm(qs), parse_hm(qe)) {
            if in_quiet(now_hm, s, e) {
                return false;
            }
        }
    }
    true
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
}
