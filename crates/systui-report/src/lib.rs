//! SysTUI report generation. A [`Report`] (assembled by [`gather_report`]) is
//! rendered to JSON (the full model), Markdown or HTML. [`to_markdown`] currently
//! renders the v0.1 health snapshot; the richer per-section renderers land in S6.3.

pub mod gather;
pub mod json;
pub mod model;

pub use gather::gather_report;
pub use json::to_json;
pub use model::{Report, ReportMeta};

use std::fmt::Write as _;

use systui_collectors::HostReport;

/// Render a [`HostReport`] as a Markdown health report.
///
/// `generated_at` is supplied by the caller so the output is deterministic.
pub fn to_markdown(report: &HostReport, generated_at: &str) -> String {
    let snap = &report.snapshot;
    let mut out = String::new();

    let _ = writeln!(out, "# SysTUI Health Report — {}", snap.hostname);
    let _ = writeln!(out);
    let _ = writeln!(out, "_Generated: {generated_at}_");
    let _ = writeln!(out);

    // Summary
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(out, "- **Health score:** {}/100", report.health.score);
    let _ = writeln!(
        out,
        "- **OS:** {} · kernel {}",
        snap.os.as_deref().unwrap_or("unknown"),
        snap.kernel
    );
    let _ = writeln!(out, "- **Uptime:** {}", human_uptime(snap.uptime_secs));
    let _ = writeln!(
        out,
        "- **Load:** {:.2} {:.2} {:.2} · {} cores",
        snap.load.one, snap.load.five, snap.load.fifteen, snap.cpu.cores
    );
    let _ = writeln!(
        out,
        "- **CPU:** {:.0}% · **RAM:** {:.0}% · **Swap:** {:.0}%",
        snap.cpu.busy_percent,
        snap.memory.used_percent(),
        snap.swap.used_percent()
    );
    let _ = writeln!(out);

    // Findings
    let _ = writeln!(out, "## Findings");
    let _ = writeln!(out);
    if report.health.checks.is_empty() {
        let _ = writeln!(out, "No findings — healthy.");
    } else {
        for check in &report.health.checks {
            let _ = writeln!(
                out,
                "- **[{}]** {} (-{})",
                severity_label(check.severity),
                check.message,
                check.points
            );
        }
    }
    let _ = writeln!(out);

    // Disks
    let _ = writeln!(out, "## Disks");
    let _ = writeln!(out);
    if snap.disks.is_empty() {
        let _ = writeln!(out, "No disk data.");
    } else {
        let _ = writeln!(out, "| Mount | Use% | Used | Size | Filesystem |");
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
        for disk in &snap.disks {
            let _ = writeln!(
                out,
                "| {} | {}% | {} | {} | {} |",
                disk.mount,
                disk.use_percent,
                human_kb(disk.used_kb),
                human_kb(disk.size_kb),
                disk.filesystem
            );
        }
    }
    let _ = writeln!(out);

    // Failed units
    let _ = writeln!(out, "## Failed units ({})", report.failed_units.len());
    let _ = writeln!(out);
    if report.failed_units.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for unit in &report.failed_units {
            let _ = writeln!(out, "- `{}` — {}", unit.name, unit.description);
        }
    }
    let _ = writeln!(out);

    // Recent errors
    let errors = report.logs.iter().filter(|e| e.is_error()).count();
    let _ = writeln!(out, "## Recent errors ({errors})");
    let _ = writeln!(out);
    if report.logs.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for entry in report.logs.iter().take(10) {
            let _ = writeln!(
                out,
                "- `{}` {} {}: {}",
                entry.time,
                entry.priority_label(),
                entry.identifier,
                entry.message
            );
        }
    }
    let _ = writeln!(out);

    // Top processes
    let _ = writeln!(out, "## Top processes");
    let _ = writeln!(out);
    if report.processes.is_empty() {
        let _ = writeln!(out, "No process data.");
    } else {
        let mut procs: Vec<_> = report.processes.iter().collect();
        procs.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let _ = writeln!(out, "| PID | User | %CPU | %MEM | Command |");
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
        for p in procs.iter().take(10) {
            let _ = writeln!(
                out,
                "| {} | {} | {:.1} | {:.1} | {} |",
                p.pid, p.user, p.cpu_percent, p.mem_percent, p.command
            );
        }
    }

    out
}

fn severity_label(severity: systui_core::Severity) -> &'static str {
    use systui_core::Severity;
    match severity {
        Severity::Critical => "CRITICAL",
        Severity::High => "HIGH",
        Severity::Medium => "MEDIUM",
        Severity::Low => "LOW",
        Severity::Info => "INFO",
    }
}

fn human_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    format!("{days}d {hours}h {mins}m")
}

fn human_kb(kb: u64) -> String {
    const MIB: f64 = 1024.0;
    const GIB: f64 = 1024.0 * 1024.0;
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let kb_f = kb as f64;
    if kb_f >= TIB {
        format!("{:.1} TiB", kb_f / TIB)
    } else if kb_f >= GIB {
        format!("{:.1} GiB", kb_f / GIB)
    } else if kb_f >= MIB {
        format!("{:.1} MiB", kb_f / MIB)
    } else {
        format!("{kb} KiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{
        Check, CpuUsage, Disk, HealthReport, LoadAverage, Memory, Process, ServiceUnit, Swap,
        SystemSnapshot,
    };
    use systui_core::Severity;

    fn sample() -> HostReport {
        HostReport {
            snapshot: SystemSnapshot {
                hostname: "prod-01".to_owned(),
                os: Some("Debian GNU/Linux 12".to_owned()),
                kernel: "6.1.0".to_owned(),
                uptime_secs: 90_000,
                load: LoadAverage {
                    one: 0.5,
                    five: 0.6,
                    fifteen: 0.7,
                },
                cpu: CpuUsage {
                    busy_percent: 12.0,
                    cores: 4,
                },
                memory: Memory {
                    total_kb: 16_000_000,
                    available_kb: 8_000_000,
                },
                swap: Swap {
                    total_kb: 0,
                    free_kb: 0,
                },
                disks: vec![Disk {
                    filesystem: "/dev/sda1".to_owned(),
                    size_kb: 100,
                    used_kb: 89,
                    avail_kb: 11,
                    use_percent: 89,
                    mount: "/".to_owned(),
                }],
                users: Vec::new(),
            },
            health: HealthReport {
                score: 85,
                checks: vec![Check {
                    severity: Severity::Critical,
                    message: "/ at 89% (>= 80% warning)".to_owned(),
                    points: 15,
                }],
            },
            processes: vec![Process {
                pid: 1132,
                ppid: 842,
                user: "www-data".to_owned(),
                cpu_percent: 5.6,
                mem_percent: 3.1,
                command: "nginx".to_owned(),
            }],
            failed_units: vec![ServiceUnit {
                name: "docker.service".to_owned(),
                load: "loaded".to_owned(),
                active: "failed".to_owned(),
                sub: "failed".to_owned(),
                description: "Docker Application Container Engine".to_owned(),
            }],
            logs: Vec::new(),
        }
    }

    #[test]
    fn markdown_contains_key_sections() {
        let md = to_markdown(&sample(), "2026-05-24 10:00:00");
        assert!(md.contains("# SysTUI Health Report — prod-01"));
        assert!(md.contains("2026-05-24 10:00:00"));
        assert!(md.contains("**Health score:** 85/100"));
        assert!(md.contains("## Findings"));
        assert!(md.contains("[CRITICAL]"));
        assert!(md.contains("## Disks"));
        assert!(md.contains("/dev/sda1"));
        assert!(md.contains("## Failed units (1)"));
        assert!(md.contains("docker.service"));
        assert!(md.contains("## Top processes"));
        assert!(md.contains("nginx"));
    }

    #[test]
    fn healthy_report_says_no_findings() {
        let mut report = sample();
        report.health.checks.clear();
        let md = to_markdown(&report, "t");
        assert!(md.contains("No findings — healthy."));
    }
}
