#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PartitionInfo {
    pub partition: String,
    pub avail: String,
    pub nodes: String,
}

pub fn build_command(slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}sinfo --noheader --format=\"%P|%a|%F\"'")
}

pub fn parse(output: &str) -> Vec<PartitionInfo> {
    output
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('|').collect();
            if f.len() != 3 {
                return None;
            }
            Some(PartitionInfo {
                partition: f[0].trim().to_string(),
                avail: f[1].trim().to_string(),
                nodes: f[2].trim().to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_sinfo() {
        let out = "gpu|up|2/8/0/10\ncpu|up|20/4/0/24\n";
        let p = parse(out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].partition, "gpu");
        assert_eq!(p[0].nodes, "2/8/0/10");
    }
}
