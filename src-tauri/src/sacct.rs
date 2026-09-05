#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SacctResult {
    pub state: String,
    pub exit_code: String,
    pub elapsed: String,
    pub max_rss: String,
    pub req_mem: String,
}

pub fn build_sacct_command(job_id: &str, slurm_conf: Option<&str>) -> String {
    // See squeue::build_squeue_command: run through a login shell so
    // profile/module-load setup that puts `sacct` on PATH actually runs.
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!(
        "bash -lc '{env}sacct -j {job_id} --noheader --parsable2 --format=State,ExitCode,Elapsed,MaxRSS,ReqMem'"
    )
}

pub fn parse_sacct_output(output: &str) -> Option<SacctResult> {
    let line = output.lines().next()?; // main job line
    let f: Vec<&str> = line.split('|').collect();
    if f.len() < 5 {
        return None;
    }
    Some(SacctResult {
        state: f[0].split_whitespace().next().unwrap_or("").to_string(),
        exit_code: f[1].to_string(),
        elapsed: f[2].to_string(),
        max_rss: f[3].to_string(),
        req_mem: f[4].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_command_with_job_id() {
        let cmd = build_sacct_command("12345", None);
        assert_eq!(
            cmd,
            "bash -lc 'sacct -j 12345 --noheader --parsable2 --format=State,ExitCode,Elapsed,MaxRSS,ReqMem'"
        );
    }

    #[test]
    fn parses_simple_completed_state() {
        let output = "COMPLETED|0:0|00:01:00|10K|1Gn\nCOMPLETED|0:0|00:01:00|10K|1Gn\n";
        assert_eq!(
            parse_sacct_output(output).map(|r| r.state),
            Some("COMPLETED".to_string())
        );
    }

    #[test]
    fn strips_trailing_qualifier_on_cancelled() {
        let output = "CANCELLED by 1001|0:0|00:01:00|10K|1Gn\nCANCELLED by 1001|0:0|00:01:00|10K|1Gn\n";
        assert_eq!(
            parse_sacct_output(output).map(|r| r.state),
            Some("CANCELLED".to_string())
        );
    }

    #[test]
    fn empty_output_yields_none() {
        assert_eq!(parse_sacct_output(""), None);
    }

    #[test]
    fn parses_extended_sacct() {
        let output = "FAILED|1:0|00:05:12|1024K|4Gn\nFAILED|1:0|00:05:12|1024K|4Gn\n";
        let r = parse_sacct_output(output).unwrap();
        assert_eq!(r.state, "FAILED");
        assert_eq!(r.exit_code, "1:0");
        assert_eq!(r.elapsed, "00:05:12");
        assert_eq!(r.max_rss, "1024K");
    }
}
