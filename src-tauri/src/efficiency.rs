#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct JobEfficiency {
    pub id: String,
    pub name: String,
    pub state: String,
    /// Elapsed as printed by sacct, e.g. "01:23:45".
    pub elapsed: String,
    pub alloc_cpus: String,
    /// Requested memory ("4Gn") vs peak RSS ("2.1G"), both human-readable.
    pub mem_req: String,
    pub mem_used: String,
    /// TotalCPU (actual CPU-seconds used) / (ElapsedRAW * AllocCPUS) * 100,
    /// when computable — seff-style utilization.
    pub cpu_util_pct: Option<f64>,
    /// ElapsedRAW / TimelimitRAW * 100, when computable.
    pub walltime_pct: Option<f64>,
    /// MaxRSS bytes / requested-memory bytes, when both computable.
    pub mem_ratio_pct: Option<f64>,
}

/// CPU-seconds actually used per CPU-second reserved, as a percentage.
/// `total_cpu_secs` is sacct's TotalCPU (user+system time consumed), not the
/// reserved CPUTimeRAW (which is Elapsed*AllocCPUS by definition and would
/// make this ~100% for every job).
fn compute_cpu_util(elapsed_raw: u64, alloc_cpus: u64, total_cpu_secs: u64) -> Option<f64> {
    if elapsed_raw == 0 || alloc_cpus == 0 {
        return None;
    }
    Some(total_cpu_secs as f64 / (elapsed_raw as f64 * alloc_cpus as f64) * 100.0)
}

fn compute_walltime_pct(elapsed_raw: u64, timelimit_raw: u64) -> Option<f64> {
    if elapsed_raw == 0 || timelimit_raw == 0 {
        return None;
    }
    Some(elapsed_raw as f64 / timelimit_raw as f64 * 100.0)
}

fn compute_mem_ratio(max_rss: Option<u64>, req: Option<u64>) -> Option<f64> {
    let (r, q) = (max_rss?, req?);
    if q == 0 {
        return None;
    }
    Some(r as f64 / q as f64 * 100.0)
}

pub fn build_command(user: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!(
        "bash -lc '{env}sacct -u {user} --starttime now-7days --endtime now \
         --state COMPLETED,FAILED --noheader --parsable2 \
         --format=JobID,JobName%40,State,Elapsed,ElapsedRAW,AllocCPUS,TimelimitRAW,ReqMem,MaxRSS,TotalCPU'"
    )
}

/// Parse a memory figure like "4Gn", "500Mn", "1024K" into bytes.
/// The trailing type char (n/c) is ignored — it only says per-node vs
/// per-cpu, which does not change the byte count.
pub fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || s == "N/A" || !s.ends_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    // Some sites print "4G" (no type char) or "4Gc"; strip a second
    // trailing letter if present.
    let (num, unit) = if num.ends_with(|c: char| c.is_ascii_alphabetic()) {
        num.split_at(num.len() - 1)
    } else {
        (num, unit)
    };
    let num: f64 = num.trim().parse().ok()?;
    let mult = match unit.trim() {
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((num * mult) as u64)
}

pub fn format_bytes(b: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if b as f64 >= GB {
        format!("{:.1}G", b as f64 / GB)
    } else if b as f64 >= MB {
        format!("{:.0}M", b as f64 / MB)
    } else {
        format!("{:.0}K", b as f64 / 1024.0)
    }
}

/// ReqMem carries a per-node ("4Gn") or per-cpu ("4Gc") suffix. Per-cpu means
/// the byte figure applies to each allocated CPU, so the job's real request is
/// value * AllocCPUS; per-node (and the suffixless form) is already the total.
pub fn parse_reqmem_bytes(s: &str, alloc_cpus: Option<u64>) -> Option<u64> {
    let bytes = parse_bytes(s)?;
    if s.trim().ends_with('c') {
        Some(bytes.saturating_mul(alloc_cpus.unwrap_or(1)))
    } else {
        Some(bytes)
    }
}

#[derive(Default, Clone)]
struct Partial {
    name: String,
    state: String,
    elapsed: String,
    alloc_cpus: Option<u64>,
    elapsed_raw: Option<u64>,
    timelimit_raw: Option<u64>,
    total_cpu_secs: Option<u64>,
    mem_req: String,
    mem_req_bytes: Option<u64>,
    max_rss_bytes: Option<u64>,
}

/// Parse `sacct --parsable2` output with the fields from `build_command`.
/// Only main job lines (JobID without a dot) populate the record; MaxRSS is
/// reported on the `.batch`/`.extern` steps on most sites, so a step's MaxRSS
/// is folded into its main job when it is larger.
pub fn parse(output: &str) -> Vec<JobEfficiency> {
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, Partial> = Default::default();

    for line in output.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 10 {
            continue;
        }
        let (id, name, state, elapsed, elapsed_raw, alloc_cpus, timelimit_raw, mem_req, max_rss, total_cpu) = (
            f[0].trim(),
            f[1].trim(),
            f[2].trim(),
            f[3].trim(),
            f[4].trim(),
            f[5].trim(),
            f[6].trim(),
            f[7].trim(),
            f[8].trim(),
            f[9].trim(),
        );
        if id.is_empty() || name.is_empty() {
            continue;
        }
        let (main_id, step) = match id.split_once('.') {
            Some((m, s)) => (m, Some(s)),
            None => (id, None),
        };
        let entry = map.entry(main_id.to_string()).or_insert_with(|| {
            order.push(main_id.to_string());
            Partial::default()
        });
        let rss = parse_bytes(max_rss);
        if rss > entry.max_rss_bytes {
            entry.max_rss_bytes = rss;
        }
        // TotalCPU (actual CPU time) may land on the main line or the
        // .batch/.extern steps; fold in the largest seen, like MaxRSS.
        let tcpu = crate::notify_filter::parse_slurm_duration(total_cpu);
        if tcpu > entry.total_cpu_secs {
            entry.total_cpu_secs = tcpu;
        }
        if step.is_some() {
            continue; // only MaxRSS / TotalCPU fold in from step lines
        }
        entry.name = name.to_string();
        entry.state = state.split_whitespace().next().unwrap_or("").to_string();
        entry.elapsed = elapsed.to_string();
        entry.alloc_cpus = alloc_cpus.parse().ok();
        entry.elapsed_raw = elapsed_raw.parse().ok();
        entry.timelimit_raw = timelimit_raw.parse().ok();
        entry.mem_req = mem_req.to_string();
        entry.mem_req_bytes = parse_reqmem_bytes(mem_req, entry.alloc_cpus);
    }

    order
        .into_iter()
        .filter_map(|id| {
            let p = map.remove(&id)?;
            if p.state.is_empty() || p.elapsed.is_empty() {
                return None;
            }
            Some(JobEfficiency {
                id: id.clone(),
                name: p.name,
                state: p.state,
                elapsed: p.elapsed,
                alloc_cpus: p
                    .alloc_cpus
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "N/A".into()),
                mem_req: if p.mem_req.is_empty() { "N/A".into() } else { p.mem_req },
                mem_used: p
                    .max_rss_bytes
                    .map(format_bytes)
                    .unwrap_or_else(|| "N/A".into()),
                cpu_util_pct: match (p.elapsed_raw, p.alloc_cpus, p.total_cpu_secs) {
                    (Some(e), Some(c), Some(t)) => compute_cpu_util(e, c, t),
                    _ => None,
                },
                walltime_pct: match (p.elapsed_raw, p.timelimit_raw) {
                    (Some(e), Some(l)) => compute_walltime_pct(e, l),
                    _ => None,
                },
                mem_ratio_pct: compute_mem_ratio(p.max_rss_bytes, p.mem_req_bytes),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_command_with_user() {
        let cmd = build_command("ivan", None);
        assert!(cmd.contains("sacct -u ivan --starttime now-7days --endtime now"));
        assert!(cmd.contains("JobName%40"));
        assert!(cmd.contains("TotalCPU"));
    }

    #[test]
    fn parses_main_line_with_batch_step_rss() {
        // Main line then .batch step carrying the real MaxRSS. TotalCPU
        // "08:00:00" = 28800s = full use of 8 CPUs for 1h.
        let out = "123|train|COMPLETED|01:00:00|3600|8|7200|16Gn|1.50G|08:00:00\n\
                   123.batch|train|COMPLETED|01:00:00|3600|8|7200|16Gn|15.20G|08:00:00\n";
        let jobs = parse(out);
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.id, "123");
        assert_eq!(j.name, "train");
        assert_eq!(j.state, "COMPLETED");
        assert_eq!(j.mem_req, "16Gn");
        assert_eq!(j.mem_used, "15.2G");
        // TotalCPU 28800 / (3600 * 8) = 100%
        assert!((j.cpu_util_pct.unwrap() - 100.0).abs() < 0.01);
        // 3600 / 7200 = 50% of walltime
        assert!((j.walltime_pct.unwrap() - 50.0).abs() < 0.01);
        // 15.2G / 16G = 95%
        assert!((j.mem_ratio_pct.unwrap() - 95.0).abs() < 0.5);
    }

    #[test]
    fn over_provisioned_job_shows_low_utilization() {
        // Reserved 64 CPUs for 4h but TotalCPU is only "04:00:00" (14400s),
        // i.e. one core-equivalent — heavily over-provisioned.
        let out = "124|sweep|COMPLETED|04:00:00|14400|64|144000|64Gn|2.00G|04:00:00\n";
        let jobs = parse(out);
        let j = &jobs[0];
        // 14400 / (14400 * 64) = 1.5625%
        assert!((j.cpu_util_pct.unwrap() - 1.5625).abs() < 0.01);
        // 2G / 64G = 3.125%
        assert!((j.mem_ratio_pct.unwrap() - 3.125).abs() < 0.01);
        // 14400 / 144000 = 10% of walltime
        assert!((j.walltime_pct.unwrap() - 10.0).abs() < 0.01);
    }

    #[test]
    fn missing_raw_fields_yield_none_metrics() {
        let out = "125|odd|COMPLETED|00:10:00|600|N/A|N/A|N/A|N/A|600\n";
        let jobs = parse(out);
        assert_eq!(jobs.len(), 1);
        let j = &jobs[0];
        assert_eq!(j.cpu_util_pct, None);
        assert_eq!(j.walltime_pct, None);
        assert_eq!(j.mem_ratio_pct, None);
        assert_eq!(j.mem_req, "N/A");
        assert_eq!(j.mem_used, "N/A");
    }

    #[test]
    fn parses_byte_units() {
        assert_eq!(parse_bytes("4Gn"), Some(4 * 1024 * 1024 * 1024));
        assert_eq!(parse_bytes("500Mn"), Some(500 * 1024 * 1024));
        assert_eq!(parse_bytes("1024K"), Some(1024 * 1024));
        assert_eq!(parse_bytes("2T"), Some(2u64 << 40));
        assert_eq!(parse_bytes("N/A"), None);
        assert_eq!(parse_bytes(""), None);
    }

    #[test]
    fn skips_malformed_lines() {
        let out = "garbage\n126|ok|COMPLETED|00:01:00|60|1|3600|1Gn|100Mn|60\n";
        let jobs = parse(out);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "126");
    }

    #[test]
    fn reqmem_per_cpu_multiplies_by_alloc_cpus() {
        // ReqMem "4Gc" on an 8-CPU job is a 32G request; MaxRSS 32G => 100%.
        let out = "200|permem|COMPLETED|01:00:00|3600|8|7200|4Gc|32.00G|08:00:00\n";
        let jobs = parse(out);
        let j = &jobs[0];
        assert!((j.mem_ratio_pct.unwrap() - 100.0).abs() < 0.5);
        // Suffixless / per-node requests are taken as-is.
        assert_eq!(parse_reqmem_bytes("4Gn", Some(8)), Some(4 * 1024 * 1024 * 1024));
        assert_eq!(
            parse_reqmem_bytes("4Gc", Some(8)),
            Some(32u64 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn format_bytes_sub_mb_uses_kib() {
        // 512 KiB must render as "512K", not the raw byte count.
        assert_eq!(format_bytes(512 * 1024), "512K");
        assert_eq!(format_bytes(parse_bytes("512K").unwrap()), "512K");
    }
}
