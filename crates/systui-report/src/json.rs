//! JSON rendering: the [`Report`] model serialized verbatim (pretty-printed).

use crate::model::Report;

/// Render a [`Report`] as pretty-printed JSON — the full structured model that
/// the Markdown and HTML renderers summarise.
pub fn to_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{
        CpuUsage, Disk, HealthReport, HostReport, LoadAverage, Memory, Swap, SystemSnapshot,
    };
    use systui_core::{ExecutionMode, Finding, ModuleId, Severity};

    use crate::model::{Report, ReportMeta};

    fn sample_report() -> Report {
        Report {
            meta: ReportMeta {
                host_label: "prod-01".to_owned(),
                generated_at: "2026-05-24 10:00:00".to_owned(),
                mode: ExecutionMode::ReadOnly,
                capabilities: None,
                docker_available: false,
            },
            host: HostReport {
                snapshot: SystemSnapshot {
                    hostname: "prod-01".to_owned(),
                    os: Some("Debian GNU/Linux 12".to_owned()),
                    kernel: "6.1.0".to_owned(),
                    uptime_secs: 100,
                    load: LoadAverage {
                        one: 0.1,
                        five: 0.2,
                        fifteen: 0.3,
                    },
                    cpu: CpuUsage {
                        busy_percent: 5.0,
                        cores: 4,
                    },
                    memory: Memory {
                        total_kb: 100,
                        available_kb: 50,
                    },
                    swap: Swap {
                        total_kb: 0,
                        free_kb: 0,
                    },
                    disks: vec![Disk {
                        filesystem: "/dev/sda1".to_owned(),
                        size_kb: 100,
                        used_kb: 50,
                        avail_kb: 50,
                        use_percent: 50,
                        mount: "/".to_owned(),
                    }],
                    users: Vec::new(),
                },
                health: HealthReport {
                    score: 100,
                    checks: Vec::new(),
                },
                processes: Vec::new(),
                failed_units: Vec::new(),
                logs: Vec::new(),
            },
            network: None,
            exposures: Vec::new(),
            findings: vec![
                Finding::new(
                    "ssh.root-login",
                    Severity::High,
                    ModuleId::Security,
                    "SSH permits direct root login",
                )
                .with_evidence("/etc/ssh/sshd_config: PermitRootLogin yes"),
            ],
            containers: Vec::new(),
            container_inspects: Vec::new(),
            container_stats: Vec::new(),
            crons: Vec::new(),
            timers: Vec::new(),
            notes: vec!["looks ok".to_owned()],
        }
    }

    #[test]
    fn json_is_valid_and_round_trips_to_a_value() {
        let json = to_json(&sample_report());
        // Pretty-printed and parseable.
        assert!(json.contains("\n  \"meta\""));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["meta"]["host_label"], "prod-01");
        assert_eq!(value["meta"]["mode"], "read-only");
        assert_eq!(value["host"]["snapshot"]["kernel"], "6.1.0");
        assert_eq!(value["findings"][0]["id"], "ssh.root-login");
        assert_eq!(value["findings"][0]["severity"], "high");
        assert_eq!(value["notes"][0], "looks ok");
    }

    #[test]
    fn finding_counts_bucket_by_severity() {
        let report = sample_report();
        // One High finding.
        assert_eq!(report.finding_counts(), [0, 1, 0, 0, 0]);
    }
}
