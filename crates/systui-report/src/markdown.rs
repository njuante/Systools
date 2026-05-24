//! Markdown rendering of a [`Report`]: a human-readable review document with an
//! executive summary, health, security findings (with evidence and
//! recommendations), open ports, docker, failed services, crons, host inventory,
//! recommendations and session notes.

use std::fmt::Write as _;

use systui_core::Severity;

use crate::model::{Report, ReportScope};
use crate::util::{human_kb, human_uptime, listener_owner, severity_label, unique_recommendations};

/// Number of risky (High/Critical) externally reachable exposures.
fn risky_exposures(report: &Report) -> usize {
    report
        .exposures
        .iter()
        .filter(|e| e.severity >= Severity::High)
        .count()
}

/// Render a [`Report`] as a Markdown document. Output is deterministic — the
/// timestamp comes from `report.meta.generated_at`. [`ReportScope::Security`]
/// emits only the security-relevant sections.
pub fn to_markdown(report: &Report, scope: ReportScope) -> String {
    let full = scope == ReportScope::Full;
    let mut out = String::new();
    let meta = &report.meta;
    let snap = &report.host.snapshot;

    let _ = writeln!(out, "# SysTUI Report — {}", meta.host_label);
    let _ = writeln!(out);
    let _ = writeln!(out, "_Generated: {}_", meta.generated_at);
    let _ = writeln!(out);
    let caps = meta
        .capabilities
        .as_ref()
        .map(|c| format!(" · {}", c.label()))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "- **Host:** {} · {} · kernel {}",
        snap.hostname,
        snap.os.as_deref().unwrap_or("unknown"),
        snap.kernel
    );
    let _ = writeln!(out, "- **Mode:** {}{caps}", meta.mode);
    let _ = writeln!(out);

    executive_summary(&mut out, report);
    if full {
        health(&mut out, report);
    }
    security_findings(&mut out, report);
    open_ports(&mut out, report);
    docker(&mut out, report);
    if full {
        failed_services(&mut out, report);
        crons(&mut out, report);
        host_inventory(&mut out, report);
    }
    recommendations(&mut out, report);
    notes(&mut out, report);

    out
}

fn executive_summary(out: &mut String, report: &Report) {
    let [crit, high, med, low, info] = report.finding_counts();
    let running = report.containers.iter().filter(|c| c.is_running()).count();

    let _ = writeln!(out, "## Executive summary");
    let _ = writeln!(out);
    let _ = writeln!(out, "- **Health score:** {}/100", report.host.health.score);
    let _ = writeln!(
        out,
        "- **Findings:** critical {crit} · high {high} · medium {med} · low {low} · info {info}"
    );
    let _ = writeln!(
        out,
        "- **Exposed ports:** {} ({} risky)",
        report.exposures.len(),
        risky_exposures(report)
    );
    let _ = writeln!(
        out,
        "- **Failed services:** {}",
        report.host.failed_units.len()
    );
    if report.meta.docker_available {
        let _ = writeln!(
            out,
            "- **Containers:** {running} running / {} total",
            report.containers.len()
        );
    } else {
        let _ = writeln!(out, "- **Containers:** Docker unavailable");
    }
    let _ = writeln!(
        out,
        "- **Cron jobs:** {} · **timers:** {}",
        report.crons.len(),
        report.timers.len()
    );
    let _ = writeln!(out);
}

fn health(out: &mut String, report: &Report) {
    let snap = &report.host.snapshot;
    let _ = writeln!(out, "## Health");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- **Uptime:** {} · **Load:** {:.2} {:.2} {:.2} · {} cores",
        human_uptime(snap.uptime_secs),
        snap.load.one,
        snap.load.five,
        snap.load.fifteen,
        snap.cpu.cores
    );
    let _ = writeln!(
        out,
        "- **CPU:** {:.0}% · **RAM:** {:.0}% · **Swap:** {:.0}%",
        snap.cpu.busy_percent,
        snap.memory.used_percent(),
        snap.swap.used_percent()
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "### Disks");
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

    let _ = writeln!(out, "### Health checks");
    let _ = writeln!(out);
    if report.host.health.checks.is_empty() {
        let _ = writeln!(out, "No threshold issues.");
    } else {
        for check in &report.host.health.checks {
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
}

fn security_findings(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Security findings ({})", report.findings.len());
    let _ = writeln!(out);
    if report.findings.is_empty() {
        let _ = writeln!(out, "No findings — nothing flagged.");
        let _ = writeln!(out);
        return;
    }
    for finding in &report.findings {
        let _ = writeln!(
            out,
            "- **[{}]** {}",
            severity_label(finding.severity),
            finding.title
        );
        if let Some(evidence) = finding.evidence.first() {
            let _ = writeln!(out, "  - _evidence:_ {evidence}");
        }
        if !finding.recommendation.is_empty() {
            let _ = writeln!(out, "  - → {}", finding.recommendation);
        }
    }
    let _ = writeln!(out);
}

fn open_ports(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Open ports ({})", report.exposures.len());
    let _ = writeln!(out);
    if report.exposures.is_empty() {
        let _ = writeln!(out, "No listening sockets detected.");
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(out, "| Risk | Proto | Address | Owner |");
    let _ = writeln!(out, "| --- | --- | --- | --- |");
    for entry in &report.exposures {
        let proto = format!("{:?}", entry.listener.protocol).to_lowercase();
        let _ = writeln!(
            out,
            "| {} | {} | {}:{} | {} |",
            severity_label(entry.severity),
            proto,
            entry.listener.local_ip,
            entry.listener.port,
            listener_owner(&entry.listener),
        );
    }
    let _ = writeln!(out);
}

fn docker(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Docker");
    let _ = writeln!(out);
    if !report.meta.docker_available {
        let _ = writeln!(out, "Docker unavailable on this host.");
        let _ = writeln!(out);
        return;
    }
    if report.containers.is_empty() {
        let _ = writeln!(out, "No containers.");
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(out, "| Name | Image | State | Health | Ports |");
    let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
    for c in &report.containers {
        let health = c
            .health
            .map(|h| format!("{h:?}").to_lowercase())
            .unwrap_or_else(|| "-".to_owned());
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            c.name, c.image, c.state, health, c.ports
        );
    }
    let _ = writeln!(out);
}

fn failed_services(out: &mut String, report: &Report) {
    let units = &report.host.failed_units;
    let _ = writeln!(out, "## Failed services ({})", units.len());
    let _ = writeln!(out);
    if units.is_empty() {
        let _ = writeln!(out, "None.");
    } else {
        for unit in units {
            let _ = writeln!(out, "- `{}` — {}", unit.name, unit.description);
        }
    }
    let _ = writeln!(out);
}

fn crons(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Crons");
    let _ = writeln!(out);
    if report.crons.is_empty() {
        let _ = writeln!(out, "No cron jobs.");
    } else {
        let _ = writeln!(out, "| Schedule | User | Command |");
        let _ = writeln!(out, "| --- | --- | --- |");
        for entry in &report.crons {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                entry.schedule,
                entry.user.as_deref().unwrap_or("—"),
                entry.command,
            );
        }
    }
    let _ = writeln!(out);
    if !report.timers.is_empty() {
        let _ = writeln!(out, "### Timers");
        let _ = writeln!(out);
        let _ = writeln!(out, "| Timer | Next | Activates |");
        let _ = writeln!(out, "| --- | --- | --- |");
        for timer in &report.timers {
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                timer.unit, timer.next, timer.activates
            );
        }
        let _ = writeln!(out);
    }
}

fn host_inventory(out: &mut String, report: &Report) {
    let snap = &report.host.snapshot;
    let _ = writeln!(out, "## Host inventory");
    let _ = writeln!(out);
    let _ = writeln!(out, "- **Label:** {}", report.meta.host_label);
    let _ = writeln!(out, "- **Hostname:** {}", snap.hostname);
    let _ = writeln!(
        out,
        "- **OS:** {} · **Kernel:** {}",
        snap.os.as_deref().unwrap_or("unknown"),
        snap.kernel
    );
    let _ = writeln!(
        out,
        "- **CPU cores:** {} · **RAM:** {}",
        snap.cpu.cores,
        human_kb(snap.memory.total_kb)
    );
    let _ = writeln!(out, "- **Uptime:** {}", human_uptime(snap.uptime_secs));
    if let Some(caps) = &report.meta.capabilities {
        let _ = writeln!(out, "- **Access:** {}", caps.label());
    }
    let _ = writeln!(out);
}

fn recommendations(out: &mut String, report: &Report) {
    let recs = unique_recommendations(&report.findings);
    let _ = writeln!(out, "## Recommendations");
    let _ = writeln!(out);
    if recs.is_empty() {
        let _ = writeln!(out, "No actions recommended.");
    } else {
        for rec in recs.iter().take(15) {
            let _ = writeln!(out, "- {rec}");
        }
    }
    let _ = writeln!(out);
}

fn notes(out: &mut String, report: &Report) {
    if report.notes.is_empty() {
        return;
    }
    let _ = writeln!(out, "## Notes");
    let _ = writeln!(out);
    for note in &report.notes {
        let _ = writeln!(out, "- {note}");
    }
    let _ = writeln!(out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReportMeta;
    use systui_collectors::{
        Check, CpuUsage, Disk, HealthReport, HostReport, LoadAverage, Memory, ServiceUnit, Swap,
        SystemSnapshot,
    };
    use systui_core::{ExecutionMode, Finding, ModuleId};

    fn sample() -> Report {
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
                processes: Vec::new(),
                failed_units: vec![ServiceUnit {
                    name: "docker.service".to_owned(),
                    load: "loaded".to_owned(),
                    active: "failed".to_owned(),
                    sub: "failed".to_owned(),
                    description: "Docker Application Container Engine".to_owned(),
                }],
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
                .with_evidence("/etc/ssh/sshd_config: PermitRootLogin yes")
                .recommendation("Set PermitRootLogin to no and reload sshd."),
            ],
            containers: Vec::new(),
            container_inspects: Vec::new(),
            container_stats: Vec::new(),
            crons: Vec::new(),
            timers: Vec::new(),
            notes: vec!["errors originate upstream on api-02".to_owned()],
        }
    }

    #[test]
    fn renders_all_sections() {
        let md = to_markdown(&sample(), ReportScope::Full);
        assert!(md.contains("# SysTUI Report — prod-01"));
        assert!(md.contains("2026-05-24 10:00:00"));
        assert!(md.contains("## Executive summary"));
        assert!(md.contains("**Health score:** 85/100"));
        assert!(md.contains("critical 0 · high 1 · medium 0"));
        assert!(md.contains("## Health"));
        assert!(md.contains("/dev/sda1"));
        assert!(md.contains("**[CRITICAL]** / at 89%"));
        assert!(md.contains("## Security findings (1)"));
        assert!(md.contains("**[HIGH]** SSH permits direct root login"));
        assert!(md.contains("_evidence:_ /etc/ssh/sshd_config: PermitRootLogin yes"));
        assert!(md.contains("## Open ports (0)"));
        assert!(md.contains("## Docker"));
        assert!(md.contains("Docker unavailable on this host."));
        assert!(md.contains("## Failed services (1)"));
        assert!(md.contains("docker.service"));
        assert!(md.contains("## Crons"));
        assert!(md.contains("## Host inventory"));
        assert!(md.contains("## Recommendations"));
        assert!(md.contains("Set PermitRootLogin to no and reload sshd."));
        assert!(md.contains("## Notes"));
        assert!(md.contains("errors originate upstream on api-02"));
    }

    #[test]
    fn clean_report_reads_as_healthy() {
        let mut report = sample();
        report.host.health.checks.clear();
        report.host.failed_units.clear();
        report.findings.clear();
        report.notes.clear();
        let md = to_markdown(&report, ReportScope::Full);
        assert!(md.contains("No threshold issues."));
        assert!(md.contains("No findings — nothing flagged."));
        assert!(md.contains("## Failed services (0)"));
        assert!(md.contains("No actions recommended."));
        // No notes section when there are none.
        assert!(!md.contains("## Notes"));
    }

    #[test]
    fn security_scope_drops_operational_sections() {
        let md = to_markdown(&sample(), ReportScope::Security);
        // Security-relevant sections stay.
        assert!(md.contains("## Executive summary"));
        assert!(md.contains("## Security findings (1)"));
        assert!(md.contains("## Open ports"));
        assert!(md.contains("## Recommendations"));
        assert!(md.contains("## Notes"));
        // Operational sections are omitted.
        assert!(!md.contains("## Health"));
        assert!(!md.contains("## Failed services"));
        assert!(!md.contains("## Crons"));
        assert!(!md.contains("## Host inventory"));
    }
}
