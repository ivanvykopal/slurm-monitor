#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjectUser {
    pub login: String,
    pub name: String,
    pub gpu_hours: String,
    pub cpu_hours: String,
    pub billing_hours: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjectInfo {
    pub account: String,
    pub raw_usage: String,
    /// Total GPU-hours consumed by the account, from sreport.
    pub gpu_hours: String,
    /// Allocated GPU-hours cap for the account, derived from the SLURM
    /// association limit GrpTRESMins (gres/gpu minutes / 60). "0" means no
    /// cap is set (or it could not be read).
    pub gpu_hours_allocated: String,
    /// Per-user GPU-hours breakdown, from sreport (empty login rows are
    /// folded into gpu_hours; this holds only real users).
    pub users: Vec<ProjectUser>,
    /// Raw gres/gpu TRES usage from sshare's GrpTRESRaw (kept as a
    /// secondary figure; gpu_hours is the human-facing number).
    pub gpu_usage: String,
    /// Total CPU core-hours consumed by the account, from sreport (TRES cpu).
    pub cpu_hours: String,
    /// Allocated CPU core-hours cap from the QOS GrpTRESMins cpu minutes / 60.
    /// "0" means no cap is set (or it could not be read).
    pub cpu_hours_allocated: String,
    /// Billing TRES hours consumed (Devana budgets projects in billing:
    /// CPU=1.0, GPU=16.0 per hour). 0 where billing is not budgeted.
    pub billing_hours: String,
    /// Allocated billing-hours cap. On Devana a project's budget is split
    /// across the <account> and <account>_gpu QOS; this holds their sum.
    /// "0" means no billing cap is set (e.g. PERUN).
    pub billing_hours_allocated: String,
}

pub fn build_command(user: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!("bash -lc '{env}sshare -U -u {user} --noheader --parsable2 --format=Account,RawUsage,EffectvUsage,GrpTRESRaw'")
}

/// sreport gives the real per-project and per-user GPU-hour figures.
/// `accounts` is a comma-separated account list (from sshare).
pub fn build_sreport_command(accounts: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!(
        "bash -lc '{env}sreport cluster AccountUtilizationByUser Start=2024-01-01 End=now -t Hour -T cpu,gres/gpu,billing --parsable2 account={accounts}'"
    )
}

/// The allocated GPU-hours cap comes from a per-account QOS whose name matches
/// the account; its GrpTRESMins holds the gres/gpu budget in minutes.
/// On Devana a project's budget is split across the <account> and
/// <account>_gpu QOS, so both are queried and their billing budgets summed.
/// `accounts` is a comma-separated list, reused directly as the QOS name list.
pub fn build_qos_command(accounts: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    // Devana splits a project's budget across <account> and <account>_gpu
    // QOS; query both so merge_qos can sum the billing budget.
    let mut names: Vec<&str> = accounts.split(',').map(str::trim).collect();
    names.retain(|n| !n.is_empty());
    let mut list: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    list.extend(names.iter().map(|n| format!("{n}_gpu")));
    format!(
        "bash -lc '{env}sacctmgr -n show qos {} format=Name,GrpTRESMins --parsable2'",
        list.join(",")
    )
}

/// The per-user association GrpTRES holds each account's node budget. When a
/// project's allocation period ends, its GrpTRES node limit is set to 0, which
/// blocks submission (`sbatch` fails with `AssocGrpNodeLimit`). Active projects
/// carry a positive node limit. `accounts` is a comma-separated account list.
pub fn build_assoc_command(user: &str, slurm_conf: Option<&str>) -> String {
    let env = crate::config::slurm_env_prefix(slurm_conf);
    format!(
        "bash -lc '{env}sacctmgr -n show assoc user={user} format=Account,GrpTRES --parsable2'"
    )
}

/// Extract a single TRES value by key from a "k1=v1,k2=v2" string, e.g.
/// `extract_tres("cpu=8,node=0", "node")` → Some("0"). Returns None when the
/// key is absent.
fn extract_tres<'a>(tres: &'a str, key: &str) -> Option<&'a str> {
    tres.split(',').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Drop projects whose association GrpTRES node limit is 0 — those are expired
/// allocations that can no longer run jobs. Accounts not present in the assoc
/// output, or whose GrpTRES has no explicit node limit, are kept (we only
/// remove accounts we positively know are capped at node=0). If the assoc query
/// returned nothing usable, every project is kept.
///
/// sacctmgr --parsable2 assoc output: Account|GrpTRES
pub fn filter_expired(projects: &mut Vec<ProjectInfo>, assoc_output: &str) {
    let mut expired: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for line in assoc_output.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 2 {
            continue;
        }
        let account = f[0].trim();
        if account.is_empty() {
            continue;
        }
        if extract_tres(f[1].trim(), "node") == Some("0") {
            expired.insert(account);
        }
    }
    projects.retain(|p| !expired.contains(p.account.as_str()));
}

fn extract_gpu(tres: &str) -> String {
    // TRESUsage looks like "gres/gpu=123456,mem=789". Some sites print
    // "gres/gpu:a100=123456" — take everything up to the next ',' or end.
    if let Some(start) = tres.find("gres/gpu") {
        let rest = &tres[start..];
        if let Some(eq) = rest.find('=') {
            let value = rest[eq + 1..].split(',').next().unwrap_or("");
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "0".to_string()
}

pub fn parse(output: &str) -> Vec<ProjectInfo> {
    output
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('|').collect();
            // Account|RawUsage|EffectvUsage|GrpTRESRaw; tolerate sites that
            // only return Account|RawUsage (older SLURM without GrpTRESRaw).
            if f.len() < 2 {
                return None;
            }
            let account = f[0].trim();
            if account.is_empty() {
                return None;
            }
            Some(ProjectInfo {
                account: account.to_string(),
                raw_usage: f[1].trim().to_string(),
                gpu_hours: "0".to_string(),
                gpu_hours_allocated: "0".to_string(),
                users: Vec::new(),
                gpu_usage: match f.get(3) {
                    Some(tres) => extract_gpu(tres),
                    None => "0".to_string(),
                },
                cpu_hours: "0".to_string(),
                cpu_hours_allocated: "0".to_string(),
                billing_hours: "0".to_string(),
                billing_hours_allocated: "0".to_string(),
            })
        })
        .collect()
}

/// sreport --parsable2 output:
/// Cluster|Account|Login|Proper Name|TRES Name|Used
/// Account-total rows have an empty Login. Header lines contain '|'-joined
/// column names and are skipped by requiring a numeric Used field.
pub fn merge_sreport(projects: &mut [ProjectInfo], sreport_output: &str) {
    for line in sreport_output.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 6 {
            continue;
        }
        let account = f[1].trim();
        let login = f[2].trim();
        let tres = f[4].trim();
        let used = f[5].trim();
        if account.is_empty() || used.parse::<f64>().is_err() {
            continue; // header or malformed row
        }
        let Some(project) = projects.iter_mut().find(|p| p.account == account) else {
            continue;
        };
        if login.is_empty() {
            if tres == "cpu" {
                project.cpu_hours = used.to_string();
            } else if tres == "billing" {
                project.billing_hours = used.to_string();
            } else {
                project.gpu_hours = used.to_string();
            }
        } else if tres == "cpu" {
            // Find or create the user row; GPU data may have arrived first.
            let user = project.users.iter_mut().find(|u| u.login == login);
            match user {
                Some(u) => u.cpu_hours = used.to_string(),
                None => project.users.push(ProjectUser {
                    login: login.to_string(),
                    name: f[3].trim().to_string(),
                    gpu_hours: "0".to_string(),
                    cpu_hours: used.to_string(),
                    billing_hours: "0".to_string(),
                }),
            }
        } else if tres == "billing" {
            let user = project.users.iter_mut().find(|u| u.login == login);
            match user {
                Some(u) => u.billing_hours = used.to_string(),
                None => project.users.push(ProjectUser {
                    login: login.to_string(),
                    name: f[3].trim().to_string(),
                    gpu_hours: "0".to_string(),
                    cpu_hours: "0".to_string(),
                    billing_hours: used.to_string(),
                }),
            }
        } else {
            // Find or create the user row; CPU data may have arrived first.
            let user = project.users.iter_mut().find(|u| u.login == login);
            match user {
                Some(u) => u.gpu_hours = used.to_string(),
                None => project.users.push(ProjectUser {
                    login: login.to_string(),
                    name: f[3].trim().to_string(),
                    gpu_hours: used.to_string(),
                    cpu_hours: "0".to_string(),
                    billing_hours: "0".to_string(),
                }),
            }
        }
    }
    // Some clusters (e.g. Devana) emit only per-user rows, without the
    // empty-login account-total row PERUN produces. There the totals above
    // never get set, so fall back to the sum of the per-user rows.
    for project in projects.iter_mut() {
        if project.gpu_hours == "0" && !project.users.is_empty() {
            let sum: f64 = project
                .users
                .iter()
                .filter_map(|u| u.gpu_hours.parse::<f64>().ok())
                .sum();
            project.gpu_hours = sum.to_string();
        }
        if project.cpu_hours == "0" && !project.users.is_empty() {
            let sum: f64 = project
                .users
                .iter()
                .filter_map(|u| u.cpu_hours.parse::<f64>().ok())
                .sum();
            project.cpu_hours = sum.to_string();
        }
        if project.billing_hours == "0" && !project.users.is_empty() {
            let sum: f64 = project
                .users
                .iter()
                .filter_map(|u| u.billing_hours.parse::<f64>().ok())
                .sum();
            project.billing_hours = sum.to_string();
        }
    }
}

/// sacctmgr --parsable2 qos output: Name|GrpTRESMins
/// GrpTRESMins looks like "cpu=5308440,gres/gpu=336960" (minutes) or is empty.
/// The QOS name matches the account; the minutes / 60 give the allocated
/// GPU- and CPU-hours.
pub fn merge_qos(projects: &mut [ProjectInfo], qos_output: &str) {
    for line in qos_output.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 2 {
            continue;
        }
        let name = f[0].trim();
        let limits = f[1].trim();
        if name.is_empty() {
            continue;
        }
        let gpu_mins = extract_gpu(limits).parse::<f64>().ok().filter(|m| *m > 0.0);
        let cpu_mins = extract_tres(limits, "cpu")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|m| *m > 0.0);
        let billing_mins = extract_tres(limits, "billing")
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|m| *m > 0.0);
        if gpu_mins.is_none() && cpu_mins.is_none() && billing_mins.is_none() {
            continue; // no budget on this QOS
        }
        // Devana splits a project's budget across <account> and <account>_gpu
        // QOS (e.g. proj-gpu and proj-gpu_gpu); accumulate the billing
        // budget instead of overwriting. A QOS named exactly like the account
        // can match twice (its own line and the "_gpu" one is a distinct name,
        // but the account-name line is processed first) — summing handles both.
        let target = name.strip_suffix("_gpu").unwrap_or(name);
        let Some(project) = projects.iter_mut().find(|p| p.account == target) else {
            continue;
        };
        if let Some(m) = gpu_mins {
            project.gpu_hours_allocated = (m / 60.0).round().to_string();
        }
        if let Some(m) = cpu_mins {
            project.cpu_hours_allocated = (m / 60.0).round().to_string();
        }
        if let Some(m) = billing_mins {
            let prev: f64 = project.billing_hours_allocated.parse().unwrap_or(0.0);
            project.billing_hours_allocated = (prev + m / 60.0).round().to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_field_line_with_gpu() {
        let out = "proj-beta|5866769|1.000000|cpu=80851,mem=729536217,gres/gpu=16927,gres/gpuutil=0\n";
        let p = parse(out);
        assert_eq!(p.len(), 1);
        assert_eq!(
            p[0],
            ProjectInfo {
                account: "proj-beta".into(),
                raw_usage: "5866769".into(),
                gpu_hours: "0".into(),
                gpu_hours_allocated: "0".into(),
                users: Vec::new(),
                gpu_usage: "16927".into(),
                cpu_hours: "0".into(),
                cpu_hours_allocated: "0".into(),
                billing_hours: "0".into(),
                billing_hours_allocated: "0".into(),
            }
        );
    }

    #[test]
    fn parses_two_field_line_without_tres() {
        let out = "proj-a|123456\n";
        let p = parse(out);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].gpu_usage, "0");
    }

    #[test]
    fn parses_gpu_with_type_suffix() {
        let out = "proj-a|1|0.0|gres/gpu:a100=555,mem=1\n";
        let p = parse(out);
        assert_eq!(p[0].gpu_usage, "555");
    }

    #[test]
    fn gpu_defaults_to_zero_when_tres_has_no_gpu() {
        let out = "proj-a|1|0.0|mem=4096,billing=99\n";
        let p = parse(out);
        assert_eq!(p[0].gpu_usage, "0");
    }

    #[test]
    fn skips_empty_accounts_and_malformed_lines() {
        let out = "|123|0|cpu=1\nproj-a|1|0|gres/gpu=2\nnot-a-line\n";
        let p = parse(out);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].account, "proj-a");
    }

    #[test]
    fn parses_multiple_accounts() {
        let out = "proj-a|10|0.5|gres/gpu=2,mem=1\nproj-b|20|0.2|gres/gpu=4,mem=1\n";
        let p = parse(out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].account, "proj-b");
        assert_eq!(p[1].gpu_usage, "4");
    }

    #[test]
    fn empty_output_yields_no_projects() {
        assert_eq!(parse(""), Vec::<ProjectInfo>::new());
        assert_eq!(parse("\n\n"), Vec::<ProjectInfo>::new());
    }

    fn base_projects() -> Vec<ProjectInfo> {
        vec![
            ProjectInfo {
                account: "proj-alpha".into(),
                raw_usage: "0".into(),
                gpu_hours: "0".into(),
                gpu_hours_allocated: "0".into(),
                users: Vec::new(),
                gpu_usage: "0".into(),
                cpu_hours: "0".into(),
                cpu_hours_allocated: "0".into(),
                billing_hours: "0".into(),
                billing_hours_allocated: "0".into(),
            },
            ProjectInfo {
                account: "proj-beta".into(),
                raw_usage: "5874349".into(),
                gpu_hours: "0".into(),
                gpu_hours_allocated: "0".into(),
                users: Vec::new(),
                gpu_usage: "16922".into(),
                cpu_hours: "0".into(),
                cpu_hours_allocated: "0".into(),
                billing_hours: "0".into(),
                billing_hours_allocated: "0".into(),
            },
        ]
    }

    #[test]
    fn merges_sreport_totals_and_users() {
        let mut projects = base_projects();
        let out = "\
Cluster|Account|Login|Proper Name|TRES Name|Used
cluster-x|proj-alpha|||cpu|164940
cluster-x|proj-alpha|bob|Bob Doe|cpu|32900
cluster-x|proj-alpha|alice|Alice Doe|cpu|0
cluster-x|proj-alpha|||gres/gpu|8247
cluster-x|proj-alpha|bob|Bob Doe|gres/gpu|1645
cluster-x|proj-alpha|alice|Alice Doe|gres/gpu|0
cluster-x|proj-beta|||cpu|21560
cluster-x|proj-beta|alice|Alice Doe|cpu|21560
cluster-x|proj-beta|||gres/gpu|1078
cluster-x|proj-beta|alice|Alice Doe|gres/gpu|1078
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].gpu_hours, "8247");
        assert_eq!(projects[0].cpu_hours, "164940");
        assert_eq!(projects[0].users.len(), 2);
        assert_eq!(projects[0].users[0].login, "bob");
        assert_eq!(projects[0].users[0].gpu_hours, "1645");
        assert_eq!(projects[0].users[0].cpu_hours, "32900");
        assert_eq!(projects[1].gpu_hours, "1078");
        assert_eq!(projects[1].cpu_hours, "21560");
        assert_eq!(projects[1].users.len(), 1);
        assert_eq!(projects[1].users[0].gpu_hours, "1078");
        assert_eq!(projects[1].users[0].cpu_hours, "21560");
    }

    #[test]
    fn merges_sreport_user_only_rows_sum_into_totals() {
        // Devana shape: no empty-login account-total rows, only per-user.
        let mut projects = base_projects();
        let out = "\
Cluster|Account|Login|Proper Name|TRES Name|Used
devana|proj-alpha|bob|Bob Doe|cpu|32900
devana|proj-alpha|bob|Bob Doe|gres/gpu|1645
devana|proj-alpha|alice|Alice Doe|cpu|5100
devana|proj-alpha|alice|Alice Doe|gres/gpu|200
devana|proj-beta|alice|Alice Doe|cpu|21560
devana|proj-beta|alice|Alice Doe|gres/gpu|1078
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].gpu_hours, "1845");
        assert_eq!(projects[0].cpu_hours, "38000");
        assert_eq!(projects[1].gpu_hours, "1078");
        assert_eq!(projects[1].cpu_hours, "21560");
    }

    #[test]
    fn merges_sreport_cpu_only_user_rows() {
        let mut projects = base_projects();
        // User row present only for cpu — gpu_hours defaults to "0".
        let out = "\
Cluster|Account|Login|Proper Name|TRES Name|Used
cluster-x|proj-alpha|bob|Bob Doe|cpu|100
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].users.len(), 1);
        assert_eq!(projects[0].users[0].cpu_hours, "100");
        assert_eq!(projects[0].users[0].gpu_hours, "0");
    }

    #[test]
    fn merges_sreport_skipping_headers_and_unknown_accounts() {
        let mut projects = base_projects();
        let out = "\
--------------------------------------------------------------------------------
Cluster|Account|Login|Proper Name|TRES Name|Used
cluster-x|unknown-acct|||gres/gpu|999
cluster-x|proj-beta|||cpu|84
cluster-x|proj-beta|||gres/gpu|42
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].gpu_hours, "0"); // untouched
        assert_eq!(projects[0].cpu_hours, "0"); // untouched
        assert_eq!(projects[1].gpu_hours, "42");
        assert_eq!(projects[1].cpu_hours, "84");
        assert!(projects[1].users.is_empty());
    }

    #[test]
    fn merges_qos_gpu_and_cpu_budget_into_allocation() {
        let mut projects = base_projects();
        // gres/gpu: 712200 min / 60 = 11870 h; cpu: 5308080 / 60 = 88468 h.
        let out = "\
proj-alpha|cpu=5308080,gres/gpu=712200
proj-beta|cpu=5308440,gres/gpu=336960
";
        merge_qos(&mut projects, out);
        assert_eq!(projects[0].gpu_hours_allocated, "11870");
        assert_eq!(projects[0].cpu_hours_allocated, "88468");
        assert_eq!(projects[1].gpu_hours_allocated, "5616");
        assert_eq!(projects[1].cpu_hours_allocated, "88474");
    }

    #[test]
    fn merges_sreport_billing_rows() {
        let mut projects = base_projects();
        let out = "\
Cluster|Account|Login|Proper Name|TRES Name|Used
devana|proj-alpha|||billing|5000
devana|proj-alpha|bob|Bob Doe|billing|24215
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].billing_hours, "5000"); // total row wins
    }

    #[test]
    fn merges_sreport_billing_sums_user_rows() {
        let mut projects = base_projects();
        let out = "\
Cluster|Account|Login|Proper Name|TRES Name|Used
devana|proj-alpha|bob|Bob Doe|billing|100.5
devana|proj-alpha|alice|Alice Doe|billing|200
";
        merge_sreport(&mut projects, out);
        assert_eq!(projects[0].billing_hours, "300.5");
    }

    #[test]
    fn merges_qos_billing_budget_sums_gpu_variant() {
        // Devana: budget split across <account> and <account>_gpu QOS.
        let mut projects = base_projects();
        let out = "\
proj-gpu|billing=300000
proj-gpu_gpu|billing=19200000
proj-beta|cpu=5308440,gres/gpu=336960
";
        // base_projects uses perun-prefixed accounts; rename for the test.
        projects[0].account = "proj-gpu".into();
        merge_qos(&mut projects, out);
        // 300000/60 + 19200000/60 = 325000 h
        assert_eq!(projects[0].billing_hours_allocated, "325000");
        assert_eq!(projects[0].gpu_hours_allocated, "0");
        assert_eq!(projects[1].gpu_hours_allocated, "5616");
        assert_eq!(projects[1].billing_hours_allocated, "0");
    }

    #[test]
    fn merges_qos_cpu_only_budget() {
        let mut projects = base_projects();
        let out = "proj-alpha|cpu=5308080\nproj-beta|\n";
        merge_qos(&mut projects, out);
        assert_eq!(projects[0].gpu_hours_allocated, "0");
        assert_eq!(projects[0].cpu_hours_allocated, "88468");
        assert_eq!(projects[1].gpu_hours_allocated, "0");
        assert_eq!(projects[1].cpu_hours_allocated, "0");
    }

    #[test]
    fn filters_accounts_capped_at_zero_nodes() {
        let mut projects = base_projects();
        // proj-alpha is expired (node=0); proj-beta stays (node=4).
        let out = "proj-alpha|node=0\nproj-beta|cpu=192,node=4,gres/gpu=8\n";
        filter_expired(&mut projects, out);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].account, "proj-beta");
    }

    #[test]
    fn keeps_accounts_without_node_limit_or_missing_from_assoc() {
        let mut projects = base_projects();
        // proj-alpha has no explicit node= limit; proj-beta absent entirely.
        let out = "proj-alpha|cpu=100,gres/gpu=4\n";
        filter_expired(&mut projects, out);
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn empty_assoc_output_keeps_all_projects() {
        let mut projects = base_projects();
        filter_expired(&mut projects, "");
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn builds_assoc_command() {
        let cmd = build_assoc_command("alice", None);
        assert!(cmd.contains("sacctmgr -n show assoc user=alice"));
        assert!(cmd.contains("format=Account,GrpTRES"));
        assert!(cmd.contains("--parsable2"));
    }

    #[test]
    fn builds_qos_command() {
        let cmd = build_qos_command("a1,a2", None);
        assert!(cmd.contains("sacctmgr -n show qos a1,a2,a1_gpu,a2_gpu"));
        assert!(cmd.contains("format=Name,GrpTRESMins"));
        assert!(cmd.contains("--parsable2"));
    }

    #[test]
    fn builds_sreport_command() {
        let cmd = build_sreport_command(
            "a1,a2",
            Some("~/slurm-custom/slurm/custom_slurm.conf"),
        );
        assert!(cmd.contains("SLURM_CONF=~/slurm-custom/slurm/custom_slurm.conf"));
        assert!(cmd.contains("sreport cluster AccountUtilizationByUser"));
        assert!(cmd.contains("account=a1,a2"));
        assert!(cmd.contains("-T cpu,gres/gpu,billing"));
        assert!(cmd.contains("--parsable2"));
    }
}
