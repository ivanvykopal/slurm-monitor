use crate::disks::DiskUsage;

/// Build the quota query for one path from a template with `{user}` and
/// `{path}` placeholders (see `config::DEFAULT_QUOTA_COMMAND`). The path is
/// quoted; the whole thing runs through a login shell like the other probes.
pub fn build_command(user: &str, path: &str, template: &str) -> String {
    let cmd = template
        .replace("{user}", user)
        .replace("{path}", &format!("\"{path}\""));
    format!("bash -lc '{cmd}'")
}

/// A quota size token: human-readable ("290G", "1.2T", "500M", with an
/// optional `*` over-quota marker) or a bare integer, which `quota`/`lfs
/// quota` report in 1 KiB blocks.
fn parse_quota_size(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('*').trim();
    if s.is_empty() || s == "-" {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u64>().ok().map(|kib| kib.saturating_mul(1024));
    }
    crate::efficiency::parse_bytes(s)
}

/// Parse `quota -s` / `lfs quota -h` output for one path. Both share the
/// layout `Filesystem  used  soft-quota  hard-limit  grace  files …`, so the
/// first data row with a non-zero limit wins. Usage is measured against the
/// soft quota (the number users are told), or the hard limit when there is no
/// soft quota. Returns None when no row carries a limit (path has no quota),
/// so the caller can fall back to df.
pub fn parse(output: &str, path: &str) -> Option<DiskUsage> {
    let lines: Vec<&str> = output.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let lower = l.to_lowercase();
        // Skip the "Disk quotas for user …" banner and the column header.
        if lower.starts_with("disk quotas") || lower.contains("filesystem") {
            continue;
        }
        let mut fields: Vec<&str> = l.split_whitespace().collect();
        // A long device name wraps onto its own line; the numbers follow on
        // the next line. Pull them in.
        if fields.len() == 1 {
            if let Some(next) = lines.get(i + 1) {
                fields.extend(next.split_whitespace());
            }
        }
        // Need at least: filesystem, used, soft, hard.
        if fields.len() < 4 {
            continue;
        }
        let Some(used) = parse_quota_size(fields[1]) else {
            continue;
        };
        let soft = parse_quota_size(fields[2]).unwrap_or(0);
        let hard = parse_quota_size(fields[3]).unwrap_or(0);
        let limit = if soft > 0 { soft } else { hard };
        if limit == 0 {
            continue; // no quota on this row — let df handle the path
        }
        let pct = (used as f64 / limit as f64 * 100.0).round();
        return Some(DiskUsage {
            filesystem: path.to_string(),
            size: crate::efficiency::format_bytes(limit),
            used: crate::efficiency::format_bytes(used),
            used_pct: Some(pct.clamp(0.0, 255.0) as u8),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_command_substitutes_user_and_quoted_path() {
        let cmd = build_command("ivan", "/home", crate::config::DEFAULT_QUOTA_COMMAND);
        assert!(cmd.starts_with("bash -lc '"));
        assert!(cmd.contains("lfs quota -h -u ivan \"/home\""));
        assert!(cmd.contains("|| quota -s"));
    }

    #[test]
    fn parses_quota_s_human_output() {
        let out = "\
Disk quotas for user ivan (uid 1000):
     Filesystem   space   quota   limit   grace   files   quota   limit   grace
      /dev/sdb1    290G    500G    550G           12345       0       0
";
        let d = parse(out, "/home").expect("should parse a quota row");
        assert_eq!(d.filesystem, "/home");
        assert_eq!(d.size, "500.0G"); // soft quota is the shown limit
        assert_eq!(d.used, "290.0G");
        assert_eq!(d.used_pct, Some(58)); // 290 / 500 = 58%
    }

    #[test]
    fn parses_lfs_quota_human_output() {
        let out = "\
Disk quotas for usr ivan (uid 1000):
     Filesystem  used   quota   limit   grace   files   quota   limit   grace
          /home   290G    500G    550G       -   12345       0       0       -
";
        let d = parse(out, "/home").unwrap();
        assert_eq!(d.used_pct, Some(58));
        assert_eq!(d.size, "500.0G");
    }

    #[test]
    fn parses_block_counts_in_kib() {
        // lfs quota / quota without -s report 1 KiB blocks.
        // 524288000 KiB = 500 GiB; 304087040 KiB ≈ 58%.
        let out = "\
Disk quotas for usr ivan (uid 1000):
     Filesystem  kbytes   quota      limit      grace  files
       /home   304087040 524288000  576716800   -      12345
";
        let d = parse(out, "/home").unwrap();
        assert_eq!(d.size, "500.0G");
        assert_eq!(d.used_pct, Some(58));
    }

    #[test]
    fn no_quota_returns_none_for_df_fallback() {
        let out = "\
Disk quotas for user ivan (uid 1000):
     Filesystem   space   quota   limit   grace   files   quota   limit   grace
      /dev/sda1     12G       0       0           1000       0       0
";
        assert_eq!(parse(out, "/home"), None);
    }

    #[test]
    fn handles_wrapped_filesystem_name() {
        let out = "\
Disk quotas for user ivan (uid 1000):
     Filesystem   space   quota   limit   grace   files
/dev/mapper/vg0-home
              290G    500G    550G           12345
";
        let d = parse(out, "/home").unwrap();
        assert_eq!(d.used_pct, Some(58));
    }

    #[test]
    fn over_quota_marker_is_stripped() {
        assert_eq!(parse_quota_size("290G*"), crate::efficiency::parse_bytes("290G"));
        assert_eq!(parse_quota_size("-"), None);
        assert_eq!(parse_quota_size("1048576"), Some(1024 * 1024 * 1024));
    }
}
