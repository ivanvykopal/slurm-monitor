#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PartitionInfo {
    pub partition: String,
    pub avail: String,
    pub nodes: String,
    /// Max walltime ("3-00:00:00"); "infinite" on unrestricted partitions.
    pub max_time: String,
    pub queued_jobs: u32,
    pub running_jobs: u32,
}

pub fn build_command(slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}sinfo --noheader --format=\"%P|%a|%F|%l\"'")
}

/// One line per partition: name (with possible `*` default marker), state,
/// running/idle/other/total, and max walltime.
pub fn parse(output: &str) -> Vec<PartitionInfo> {
    output
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('|').collect();
            if f.len() != 4 {
                return None;
            }
            Some(PartitionInfo {
                partition: f[0].trim().trim_end_matches('*').to_string(),
                avail: f[1].trim().to_string(),
                nodes: f[2].trim().to_string(),
                max_time: f[3].trim().to_string(),
                queued_jobs: 0,
                running_jobs: 0,
            })
        })
        .collect()
}

/// Count running and pending jobs per partition from
/// `squeue -h -o %P|%T --states PD,RUNNING`. A single job in a mixed state
/// (e.g. RUNNING with pending components) is counted by its whole state
/// string prefix.
pub fn merge_job_counts(partitions: &mut [PartitionInfo], squeue_output: &str) {
    for line in squeue_output.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 2 {
            continue;
        }
        let part = f[0].trim().trim_end_matches('*');
        let state = f[1].trim();
        let Some(p) = partitions.iter_mut().find(|p| p.partition == part) else {
            continue;
        };
        if state.starts_with("RUNNING") {
            p.running_jobs += 1;
        } else if state.starts_with("PENDING") {
            p.queued_jobs += 1;
        }
    }
}

pub fn build_squeue_partition_command(slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    // Whole-cluster counts (no -u): the "where to submit" signal is the
    // queue depth everyone sees, not just ours.
    format!("bash -lc '{env}squeue --noheader --format=\"%P|%T\" --states PD,RUNNING'")
}

/// Idle node count from sinfo's A/I/O/T field ("2/8/0/10" → 8).
pub fn idle_nodes(nodes: &str) -> Option<u32> {
    nodes.split('/').nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sinfo() {
        let out = "gpu|up|2/8/0/10|3-00:00:00\ncpu|up|20/4/0/24|infinite\n";
        let p = parse(out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].partition, "gpu");
        assert_eq!(p[0].nodes, "2/8/0/10");
        assert_eq!(p[0].max_time, "3-00:00:00");
        assert_eq!(p[1].max_time, "infinite");
    }

    #[test]
    fn strips_default_partition_star() {
        let out = "cpu*|up|20/4/0/24|1-00:00:00\n";
        let p = parse(out);
        assert_eq!(p[0].partition, "cpu");
    }

    #[test]
    fn merges_running_and_queued_counts() {
        let mut p = parse("gpu|up|2/8/0/10|3-00:00:00\ncpu|up|0/24/0/24|infinite\n");
        let sq = "\
gpu|RUNNING
gpu|RUNNING
gpu|PENDING
gpu|PENDING
gpu|PENDING
cpu|RUNNING
longjob|RUNNING
";
        merge_job_counts(&mut p, sq);
        assert_eq!(p[0].running_jobs, 2);
        assert_eq!(p[0].queued_jobs, 3);
        assert_eq!(p[1].running_jobs, 1);
        assert_eq!(p[1].queued_jobs, 0);
        // unknown partitions are dropped, not counted anywhere
    }

    #[test]
    fn extracts_idle_nodes() {
        assert_eq!(idle_nodes("2/8/0/10"), Some(8));
        assert_eq!(idle_nodes("0/0/24/24"), Some(0));
        assert_eq!(idle_nodes("bad"), None);
    }
}
