#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JobStatus {
    pub id: String,
    pub name: String,
    pub state: String,
    pub time: String,
    pub time_limit: String,
    pub nodes: String,
    pub reason: String,
    pub partition: String,
    pub est_start: String,
    pub cluster: String,
}

pub fn build_squeue_command(user: &str, slurm_conf: Option<&str>) -> String {
    // Run through a login shell so profile/module-load setup that puts
    // `squeue` on PATH (common on HPC clusters) actually runs — a bare
    // `ssh host squeue ...` exec does not source login profile scripts.
    // The --format value's `|` separators must be quoted, or the shell
    // that runs this command treats them as pipe operators and splits
    // it into (squeue ... --format=%i) | %j | %T | %M | %l | %D | %R | %P,
    // none of which are real commands.
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}squeue -u {user} --noheader --format=\"%i|%j|%T|%M|%l|%D|%R|%P|%S\"'")
}

pub fn parse_squeue_output(output: &str, cluster: &str) -> Vec<JobStatus> {
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('|').collect();
            if fields.len() != 9 {
                tracing::warn!("skipping malformed squeue line (expected 9 fields): {line}");
                return None;
            }
            Some(JobStatus {
                id: fields[0].trim().to_string(),
                name: fields[1].trim().to_string(),
                state: fields[2].trim().to_string(),
                time: fields[3].trim().to_string(),
                time_limit: fields[4].trim().to_string(),
                nodes: fields[5].trim().to_string(),
                reason: fields[6].trim().to_string(),
                partition: fields[7].trim().to_string(),
                est_start: fields[8].trim().to_string(),
                cluster: cluster.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_command_with_user_and_format() {
        let cmd = build_squeue_command("ivan", None);
        assert_eq!(
            cmd,
            "bash -lc 'squeue -u ivan --noheader --format=\"%i|%j|%T|%M|%l|%D|%R|%P|%S\"'"
        );
    }

    #[test]
    fn builds_command_with_slurm_conf_prefix() {
        let cmd = build_squeue_command("ivan", Some("~/slurm-custom/slurm/custom_slurm.conf"));
        assert_eq!(
            cmd,
            "bash -lc 'SLURM_CONF=~/slurm-custom/slurm/custom_slurm.conf squeue -u ivan --noheader --format=\"%i|%j|%T|%M|%l|%D|%R|%P|%S\"'"
        );
    }

    #[test]
    fn parses_multiple_job_lines() {
        let output = "12345|train-model|RUNNING|01:23:45|1-00:00:00|2|node042|gpu|N/A\n\
                       12346|preprocess|PENDING|0:00|1:00:00|1|(Priority)|cpu|2024-01-15T14:32:00\n";
        let jobs = parse_squeue_output(output, "test");
        assert_eq!(
            jobs,
            vec![
                JobStatus {
                    id: "12345".into(),
                    name: "train-model".into(),
                    state: "RUNNING".into(),
                    time: "01:23:45".into(),
                    time_limit: "1-00:00:00".into(),
                    nodes: "2".into(),
                    reason: "node042".into(),
                    partition: "gpu".into(),
                    est_start: "N/A".into(),
                    cluster: "test".into(),
                },
                JobStatus {
                    id: "12346".into(),
                    name: "preprocess".into(),
                    state: "PENDING".into(),
                    time: "0:00".into(),
                    time_limit: "1:00:00".into(),
                    nodes: "1".into(),
                    reason: "(Priority)".into(),
                    partition: "cpu".into(),
                    est_start: "2024-01-15T14:32:00".into(),
                    cluster: "test".into(),
                },
            ]
        );
    }

    #[test]
    fn empty_output_yields_no_jobs() {
        assert_eq!(parse_squeue_output("", "test"), Vec::new());
    }

    #[test]
    fn skips_malformed_lines() {
        let output =
            "not-enough-fields|here\n12345|ok-job|RUNNING|0:05|1:00:00|1|node1|cpu|N/A\n";
        let jobs = parse_squeue_output(output, "test");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "12345");
        assert_eq!(jobs[0].cluster, "test");
    }

    #[test]
    fn parses_extended_fields() {
        let output = "12345|train|RUNNING|01:23:45|1-00:00:00|2|node042|gpu|N/A\n";
        let jobs = parse_squeue_output(output, "devana");
        assert_eq!(jobs[0].time_limit, "1-00:00:00");
        assert_eq!(jobs[0].nodes, "2");
        assert_eq!(jobs[0].reason, "node042"); // %R is node list when running
        assert_eq!(jobs[0].partition, "gpu");
    }

    #[test]
    fn parses_estimated_start() {
        let output = "12345|train|PENDING|0:00|1-00:00:00|1|(Priority)|gpu|2024-01-15T14:32:00\n";
        let jobs = parse_squeue_output(output, "devana");
        assert_eq!(jobs[0].est_start, "2024-01-15T14:32:00");
    }
}
