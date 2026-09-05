pub fn build_stdout_path_command(job_id: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}scontrol show job {job_id}'")
}

pub fn parse_stdout_path(output: &str) -> Option<String> {
    parse_path(output, "StdOut=")
}

pub fn parse_stderr_path(output: &str) -> Option<String> {
    parse_path(output, "StdErr=")
}

fn parse_path(output: &str, prefix: &str) -> Option<String> {
    output
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix(prefix).map(|s| s.to_string()))
}

pub fn build_tail_command(path: &str, lines: u32) -> String {
    format!("bash -lc 'tail -n {lines} \"{path}\"'")
}

#[cfg(test)]
mod tests {
    use super::*;
    const OUT: &str =
        "JobId=42 JobName=train\n   StdOut=/home/u/slurm-42.out\n   StdErr=/home/u/slurm-42.err\n";

    #[test]
    fn parses_stdout_path() {
        assert_eq!(parse_stdout_path(OUT), Some("/home/u/slurm-42.out".to_string()));
    }

    #[test]
    fn parses_stderr_path() {
        assert_eq!(parse_stderr_path(OUT), Some("/home/u/slurm-42.err".to_string()));
    }
}
