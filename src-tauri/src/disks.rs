#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiskUsage {
    pub filesystem: String,
    pub size: String,
    pub used: String,
    /// "85" for 85% — plain number so the UI can threshold on it.
    pub used_pct: Option<u8>,
}

/// `df -h --output=target,size,used,pcent <paths...>`. `--output` is
/// GNU coreutils; on non-GNU login nodes the fallback is `df -hP` (POSIX
/// output — one line per filesystem, so long device names don't wrap),
/// whose columns are parsed by position.
///
/// Configured paths are filtered to those that actually exist first: a
/// cluster that lacks one of them (e.g. perun has no `/scratch`) would
/// otherwise make `df` exit non-zero and fail the whole panel instead of
/// reporting the paths that are present. When none exist, the command exits
/// 0 with no output and the panel shows the empty-state message.
pub fn build_command(paths: &[String]) -> String {
    let joined = paths
        .iter()
        .map(|p| format!("\"{p}\""))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "bash -lc 'sel=; for p in {joined}; do [ -e \"$p\" ] && sel=\"$sel $p\"; done; \
         [ -n \"$sel\" ] || exit 0; \
         df -h --output=target,size,used,pcent $sel 2>/dev/null || df -hP $sel'"
    )
}

/// Parse either shape (the header line is skipped):
/// - `--output=target,size,used,pcent`: 4 columns
///   `Mounted-on  Size  Used  Use%`
/// - plain `df -h`: 6 columns `Filesystem  Size  Used  Avail  Use%  Mounted-on`
///
/// GNU coreutils prints Use% with a trailing `%` in *both* shapes, so the
/// two are told apart by column count, not by the presence of `%`.
pub fn parse(output: &str) -> Vec<DiskUsage> {
    let mut out = Vec::new();
    for line in output.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.len() {
            // --output shape: target size used pcent
            4 => out.push(DiskUsage {
                filesystem: f[0].to_string(),
                size: f[1].to_string(),
                used: f[2].to_string(),
                used_pct: f[3].trim_end_matches('%').parse().ok(),
            }),
            // plain df shape: fs size used avail use% mount
            6 => out.push(DiskUsage {
                filesystem: f[5].to_string(),
                size: f[1].to_string(),
                used: f[2].to_string(),
                used_pct: f[4].trim_end_matches('%').parse().ok(),
            }),
            _ => {}
        }
    }
    out
}

/// Filesystems that crossed 90% usage in this snapshot and had not been
/// notified, given the set of already-notified filesystem names. A
/// filesystem drops out of `notified` once it falls back under 85% so a
/// later re-fill notifies again.
pub fn crossing_threshold(
    disks: &[DiskUsage],
    notified: &mut std::collections::HashSet<String>,
) -> Vec<DiskUsage> {
    let mut fires = Vec::new();
    for d in disks {
        match d.used_pct {
            Some(p) if p >= 90 => {
                if notified.insert(d.filesystem.clone()) {
                    fires.push(d.clone());
                }
            }
            Some(p) if p < 85 => {
                notified.remove(&d.filesystem);
            }
            _ => {}
        }
    }
    fires
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn builds_command_with_quoted_paths() {
        let cmd = build_command(&["/home".into(), "/scratch".into()]);
        assert!(cmd.contains("df -h --output=target,size,used,pcent"));
        // Configured paths appear in the existence-filter loop.
        assert!(cmd.contains("for p in \"/home\" \"/scratch\""));
        // Missing paths are dropped before df runs.
        assert!(cmd.contains("[ -e \"$p\" ]"));
        // POSIX fallback avoids line-wrapping on long device names.
        assert!(cmd.contains("|| df -hP $sel"));
    }

    #[test]
    fn parses_output_variant() {
        // GNU `df --output=...,pcent` prints Use% with a trailing `%`.
        let out = "\
Mounted on  Size  Used Use%
/home        100G   85G  85%
/scratch      50T    2G   1%
";
        let d = parse(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].filesystem, "/home");
        assert_eq!(d[0].size, "100G");
        assert_eq!(d[0].used, "85G");
        assert_eq!(d[0].used_pct, Some(85));
        assert_eq!(d[1].used_pct, Some(1));
    }

    #[test]
    fn parses_plain_df_variant() {
        let out = "\
Filesystem      Size  Used Avail Use% Mounted on
/dev/sda1       100G   95G  5.0G  95% /home
nas:/scratch     50T   10T   40T  20% /scratch
";
        let d = parse(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].filesystem, "/home");
        assert_eq!(d[0].used_pct, Some(95));
        assert_eq!(d[1].filesystem, "/scratch");
        assert_eq!(d[1].used_pct, Some(20));
        assert_eq!(d[1].size, "50T");
    }

    #[test]
    fn threshold_fires_once_until_recovery() {
        let disks = vec![
            DiskUsage { filesystem: "/home".into(), size: "100G".into(), used: "95G".into(), used_pct: Some(95) },
            DiskUsage { filesystem: "/scratch".into(), size: "50T".into(), used: "2G".into(), used_pct: Some(1) },
        ];
        let mut notified = HashSet::new();
        let fires = crossing_threshold(&disks, &mut notified);
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].filesystem, "/home");

        // Second check at the same level: no re-notify.
        assert!(crossing_threshold(&disks, &mut notified).is_empty());

        // Recovery below 85% clears the latch...
        let recovered = vec![DiskUsage {
            filesystem: "/home".into(),
            size: "100G".into(),
            used: "50G".into(),
            used_pct: Some(50),
        }];
        crossing_threshold(&recovered, &mut notified);
        // ...so re-filling notifies again.
        let refilled = vec![DiskUsage {
            filesystem: "/home".into(),
            size: "100G".into(),
            used: "91G".into(),
            used_pct: Some(91),
        }];
        let fires = crossing_threshold(&refilled, &mut notified);
        assert_eq!(fires.len(), 1);
    }

    #[test]
    fn hysteresis_between_85_and_90_keeps_state() {
        let mut notified = HashSet::new();
        notified.insert("/home".to_string());
        // 88% — above recovery, below threshold: stays latched.
        let disks = vec![DiskUsage {
            filesystem: "/home".into(),
            size: "100G".into(),
            used: "88G".into(),
            used_pct: Some(88),
        }];
        assert!(crossing_threshold(&disks, &mut notified).is_empty());
        assert!(notified.contains("/home"));
    }
}
