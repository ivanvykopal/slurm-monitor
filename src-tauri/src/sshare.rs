pub fn build_command(user: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}sshare -U -u {user} --noheader --parsable2 --format=FairShare'")
}

pub fn parse(output: &str) -> Option<String> {
    output
        .lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_fairshare() {
        assert_eq!(parse("0.123456\n"), Some("0.123456".to_string()));
    }
}
