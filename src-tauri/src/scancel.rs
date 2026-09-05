pub fn build_scancel_command(job_id: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}scancel {job_id}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_scancel() {
        assert_eq!(build_scancel_command("999", None), "bash -lc 'scancel 999'");
    }
}
