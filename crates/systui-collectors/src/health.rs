//! Threshold checks and an explainable health score.
//!
//! Evaluation is a pure function of a [`SystemSnapshot`], a couple of counts and
//! the configured [`Thresholds`], so it is fully testable. The score starts at
//! 100 and each failing check subtracts an explainable number of points
//! (`Product.md` §7.1).

use serde::{Deserialize, Serialize};
use systui_core::{Severity, Thresholds};

use crate::system::SystemSnapshot;

/// A single failing check with the points it removes from the health score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub severity: Severity,
    pub message: String,
    pub points: u8,
}

/// The health score and the checks that explain it (worst first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub score: u8,
    pub checks: Vec<Check>,
}

/// Evaluate the snapshot (plus failed-unit and recent-error counts) against the
/// thresholds and produce a prioritized [`HealthReport`].
pub fn evaluate_health(
    snap: &SystemSnapshot,
    failed_units: usize,
    recent_errors: usize,
    thresholds: &Thresholds,
) -> HealthReport {
    let mut checks = Vec::new();

    for disk in &snap.disks {
        if disk.use_percent >= thresholds.disk_critical {
            checks.push(Check {
                severity: Severity::Critical,
                message: format!(
                    "{} at {}% (\u{2265} {}% critical)",
                    disk.mount, disk.use_percent, thresholds.disk_critical
                ),
                points: 15,
            });
        } else if disk.use_percent >= thresholds.disk_warning {
            checks.push(Check {
                severity: Severity::High,
                message: format!(
                    "{} at {}% (\u{2265} {}% warning)",
                    disk.mount, disk.use_percent, thresholds.disk_warning
                ),
                points: 8,
            });
        }
    }

    let ram = snap.memory.used_percent();
    if ram >= f64::from(thresholds.ram_warning) {
        checks.push(Check {
            severity: Severity::Medium,
            message: format!("RAM at {ram:.0}% (\u{2265} {}%)", thresholds.ram_warning),
            points: 6,
        });
    }

    if snap.swap.total_kb > 0 {
        let swap = snap.swap.used_percent();
        if swap >= 80.0 {
            checks.push(Check {
                severity: Severity::Medium,
                message: format!("swap at {swap:.0}%"),
                points: 6,
            });
        } else if swap >= 50.0 {
            checks.push(Check {
                severity: Severity::Low,
                message: format!("swap at {swap:.0}%"),
                points: 3,
            });
        }
    }

    let load_limit = (snap.cpu.cores.max(1) as f64) * thresholds.load_warning_multiplier;
    if snap.load.one > load_limit {
        checks.push(Check {
            severity: Severity::Medium,
            message: format!(
                "load {:.2} > {:.1} ({}\u{00d7} {} cores)",
                snap.load.one, load_limit, thresholds.load_warning_multiplier, snap.cpu.cores
            ),
            points: 6,
        });
    }

    if failed_units > 0 {
        let points = u8::try_from(failed_units * 4).unwrap_or(u8::MAX).min(16);
        checks.push(Check {
            severity: Severity::High,
            message: format!("{failed_units} failed systemd unit(s)"),
            points,
        });
    }

    if recent_errors >= 20 {
        checks.push(Check {
            severity: Severity::Medium,
            message: format!("{recent_errors} recent error logs"),
            points: 5,
        });
    } else if recent_errors > 0 {
        checks.push(Check {
            severity: Severity::Low,
            message: format!("{recent_errors} recent error logs"),
            points: 2,
        });
    }

    checks.sort_by(|a, b| b.severity.cmp(&a.severity).then(b.points.cmp(&a.points)));

    let deducted: u32 = checks.iter().map(|c| u32::from(c.points)).sum();
    let score = 100u32.saturating_sub(deducted) as u8;

    HealthReport { score, checks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{CpuUsage, Disk, LoadAverage, Memory, Swap};

    fn snapshot(disk_pct: u8, ram_avail_kb: u64, load_one: f64) -> SystemSnapshot {
        SystemSnapshot {
            hostname: "h".to_owned(),
            os: None,
            kernel: "k".to_owned(),
            uptime_secs: 0,
            load: LoadAverage {
                one: load_one,
                five: 0.0,
                fifteen: 0.0,
            },
            cpu: CpuUsage {
                busy_percent: 0.0,
                cores: 4,
            },
            memory: Memory {
                total_kb: 100,
                available_kb: ram_avail_kb,
            },
            swap: Swap {
                total_kb: 0,
                free_kb: 0,
            },
            disks: vec![Disk {
                filesystem: "/dev/sda1".to_owned(),
                size_kb: 100,
                used_kb: disk_pct as u64,
                avail_kb: 0,
                use_percent: disk_pct,
                mount: "/".to_owned(),
            }],
            users: Vec::new(),
            cpu_model: None,
            virtualization: None,
        }
    }

    #[test]
    fn healthy_system_scores_100() {
        let snap = snapshot(20, 80, 0.1);
        let report = evaluate_health(&snap, 0, 0, &Thresholds::default());
        assert_eq!(report.score, 100);
        assert!(report.checks.is_empty());
    }

    #[test]
    fn critical_disk_is_flagged_and_deducts() {
        let snap = snapshot(95, 80, 0.1);
        let report = evaluate_health(&snap, 0, 0, &Thresholds::default());
        assert_eq!(report.score, 85);
        assert_eq!(report.checks[0].severity, Severity::Critical);
        assert!(report.checks[0].message.contains("95%"));
    }

    #[test]
    fn multiple_issues_sort_worst_first_and_clamp_at_zero() {
        // disk critical (95%), high RAM (90% used), high load, many failures + errors
        let snap = snapshot(95, 10, 100.0);
        let report = evaluate_health(&snap, 10, 50, &Thresholds::default());
        assert_eq!(report.checks[0].severity, Severity::Critical);
        // worst-first ordering
        for pair in report.checks.windows(2) {
            assert!(pair[0].severity >= pair[1].severity);
        }
        // score never underflows
        assert!(report.score <= 100);
    }
}
