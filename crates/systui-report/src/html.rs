//! HTML rendering of a [`Report`]: a single self-contained file with inline CSS
//! (no external assets, no JavaScript), readable and printable. Every value that
//! originates from the host is HTML-escaped — SysTUI renders strings collected
//! from untrusted remote hosts, so escaping is a security requirement.

use std::fmt::Write as _;

use systui_collectors::{BindScope, DatabaseInstance};
use systui_core::Severity;

use crate::model::{Report, ReportScope};
use crate::util::{
    escape_html as esc, human_kb, human_uptime, listener_owner, severity_class, severity_label,
    unique_recommendations,
};

pub(crate) const STYLE: &str = "\
body{font-family:system-ui,-apple-system,Segoe UI,sans-serif;max-width:920px;\
margin:2rem auto;padding:0 1rem;color:#1a1a1a;line-height:1.5}\
h1{margin-bottom:.2rem}h2{border-bottom:1px solid #ddd;padding-bottom:.2rem;margin-top:2rem}\
h3{margin-top:1.2rem}.meta{color:#666;font-size:.9rem}\
table{border-collapse:collapse;width:100%;margin:.5rem 0}\
th,td{border:1px solid #ddd;padding:.35rem .6rem;text-align:left;font-size:.9rem}\
th{background:#f5f5f5}ul{padding-left:1.2rem}\
.sev{font-weight:700;padding:.05rem .45rem;border-radius:.25rem;font-size:.75rem;white-space:nowrap}\
.sev-critical{background:#b00020;color:#fff}.sev-high{background:#d32f2f;color:#fff}\
.sev-medium{background:#f57c00;color:#fff}.sev-low{background:#fbc02d;color:#000}\
.sev-info{background:#e0e0e0;color:#000}\
.finding{margin:.6rem 0}.evidence{color:#555;font-family:ui-monospace,monospace;font-size:.82rem}\
.rec{color:#1b5e20}.muted{color:#888}code{font-family:ui-monospace,monospace}";

/// A severity badge `<span>`.
fn badge(severity: Severity) -> String {
    format!(
        "<span class=\"sev {}\">{}</span>",
        severity_class(severity),
        severity_label(severity)
    )
}

/// Render a [`Report`] as a single self-contained HTML document.
/// [`ReportScope::Security`] emits only the security-relevant sections.
pub fn to_html(report: &Report, scope: ReportScope) -> String {
    let full = scope == ReportScope::Full;
    let meta = &report.meta;
    let snap = &report.host.snapshot;
    let mut out = String::new();

    let _ = writeln!(out, "<!DOCTYPE html>");
    let _ = writeln!(out, "<html lang=\"en\"><head><meta charset=\"utf-8\">");
    let _ = writeln!(
        out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    );
    let _ = writeln!(
        out,
        "<title>SysTUI Report — {}</title>",
        esc(&meta.host_label)
    );
    let _ = writeln!(out, "<style>{STYLE}</style></head><body>");

    let _ = writeln!(out, "<h1>SysTUI Report — {}</h1>", esc(&meta.host_label));
    let caps = meta
        .capabilities
        .as_ref()
        .map(|c| format!(" · {}", esc(&c.label())))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "<p class=\"meta\">Generated: {} · {} · {} · kernel {} · mode {}{caps}</p>",
        esc(&meta.generated_at),
        esc(&snap.hostname),
        esc(snap.os.as_deref().unwrap_or("unknown")),
        esc(&snap.kernel),
        meta.mode,
    );

    executive_summary(&mut out, report);
    if full {
        health(&mut out, report);
    }
    security_findings(&mut out, report);
    open_ports(&mut out, report);
    docker(&mut out, report);
    databases(&mut out, report);
    if full {
        failed_services(&mut out, report);
        crons(&mut out, report);
        host_inventory(&mut out, report);
    }
    recommendations(&mut out, report);
    notes(&mut out, report);

    let _ = writeln!(out, "</body></html>");
    out
}

fn executive_summary(out: &mut String, report: &Report) {
    let [crit, high, med, low, info] = report.finding_counts();
    let running = report.containers.iter().filter(|c| c.is_running()).count();
    let risky = report
        .exposures
        .iter()
        .filter(|e| e.severity >= Severity::High)
        .count();
    let containers = if report.meta.docker_available {
        format!("{running} running / {} total", report.containers.len())
    } else {
        "Docker unavailable".to_owned()
    };

    let _ = writeln!(out, "<h2>Executive summary</h2><ul>");
    let _ = writeln!(
        out,
        "<li><b>Health score:</b> {}/100</li>",
        report.host.health.score
    );
    let _ = writeln!(
        out,
        "<li><b>Findings:</b> critical {crit} · high {high} · medium {med} · low {low} · info {info}</li>"
    );
    let _ = writeln!(
        out,
        "<li><b>Exposed ports:</b> {} ({risky} risky)</li>",
        report.exposures.len()
    );
    let _ = writeln!(
        out,
        "<li><b>Failed services:</b> {}</li>",
        report.host.failed_units.len()
    );
    let _ = writeln!(out, "<li><b>Containers:</b> {containers}</li>");
    let _ = writeln!(
        out,
        "<li><b>Cron jobs:</b> {} · <b>timers:</b> {}</li>",
        report.crons.len(),
        report.timers.len()
    );
    let _ = writeln!(
        out,
        "<li><b>Databases:</b> {} detected</li>",
        report.databases.instances.len()
    );
    let _ = writeln!(out, "</ul>");
}

fn health(out: &mut String, report: &Report) {
    let snap = &report.host.snapshot;
    let _ = writeln!(out, "<h2>Health</h2><ul>");
    let _ = writeln!(
        out,
        "<li><b>Uptime:</b> {} · <b>Load:</b> {:.2} {:.2} {:.2} · {} cores</li>",
        human_uptime(snap.uptime_secs),
        snap.load.one,
        snap.load.five,
        snap.load.fifteen,
        snap.cpu.cores
    );
    let _ = writeln!(
        out,
        "<li><b>CPU:</b> {:.0}% · <b>RAM:</b> {:.0}% · <b>Swap:</b> {:.0}%</li>",
        snap.cpu.busy_percent,
        snap.memory.used_percent(),
        snap.swap.used_percent()
    );
    let _ = writeln!(out, "</ul>");

    let _ = writeln!(out, "<h3>Disks</h3>");
    if snap.disks.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No disk data.</p>");
    } else {
        let _ = writeln!(
            out,
            "<table><tr><th>Mount</th><th>Use%</th><th>Used</th><th>Size</th><th>Filesystem</th></tr>"
        );
        for disk in &snap.disks {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}%</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&disk.mount),
                disk.use_percent,
                human_kb(disk.used_kb),
                human_kb(disk.size_kb),
                esc(&disk.filesystem)
            );
        }
        let _ = writeln!(out, "</table>");
    }

    let _ = writeln!(out, "<h3>Health checks</h3>");
    if report.host.health.checks.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No threshold issues.</p>");
    } else {
        let _ = writeln!(out, "<ul>");
        for check in &report.host.health.checks {
            let _ = writeln!(
                out,
                "<li>{} {} (-{})</li>",
                badge(check.severity),
                esc(&check.message),
                check.points
            );
        }
        let _ = writeln!(out, "</ul>");
    }
}

fn security_findings(out: &mut String, report: &Report) {
    let _ = writeln!(
        out,
        "<h2>Security findings ({})</h2>",
        report.findings.len()
    );
    if report.findings.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No findings — nothing flagged.</p>");
        return;
    }
    for finding in &report.findings {
        let _ = writeln!(out, "<div class=\"finding\">");
        let _ = writeln!(
            out,
            "{} <span class=\"muted\">[{}]</span> <b>{}</b>",
            badge(finding.severity),
            esc(finding.status.label()),
            esc(&finding.title)
        );
        if let Some(evidence) = finding.evidence.first() {
            let _ = writeln!(out, "<div class=\"evidence\">{}</div>", esc(evidence));
        }
        if !finding.recommendation.is_empty() {
            let _ = writeln!(
                out,
                "<div class=\"rec\">→ {}</div>",
                esc(&finding.recommendation)
            );
        }
        let _ = writeln!(out, "</div>");
    }
}

fn open_ports(out: &mut String, report: &Report) {
    let _ = writeln!(out, "<h2>Open ports ({})</h2>", report.exposures.len());
    if report.exposures.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No listening sockets detected.</p>");
        return;
    }
    let _ = writeln!(
        out,
        "<table><tr><th>Risk</th><th>Proto</th><th>Address</th><th>Owner</th></tr>"
    );
    for entry in &report.exposures {
        let proto = format!("{:?}", entry.listener.protocol).to_lowercase();
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{proto}</td><td>{}:{}</td><td>{}</td></tr>",
            badge(entry.severity),
            esc(&entry.listener.local_ip),
            entry.listener.port,
            esc(&listener_owner(&entry.listener)),
        );
    }
    let _ = writeln!(out, "</table>");
}

fn docker(out: &mut String, report: &Report) {
    let _ = writeln!(out, "<h2>Docker</h2>");
    if !report.meta.docker_available {
        let _ = writeln!(
            out,
            "<p class=\"muted\">Docker unavailable on this host.</p>"
        );
        return;
    }
    if report.containers.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No containers.</p>");
        return;
    }
    let _ = writeln!(
        out,
        "<table><tr><th>Name</th><th>Image</th><th>State</th><th>Health</th><th>Ports</th></tr>"
    );
    for c in &report.containers {
        let health = c
            .health
            .map(|h| format!("{h:?}").to_lowercase())
            .unwrap_or_else(|| "-".to_owned());
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&c.name),
            esc(&c.image),
            esc(&c.state),
            esc(&health),
            esc(&c.ports)
        );
    }
    let _ = writeln!(out, "</table>");
}

fn databases(out: &mut String, report: &Report) {
    let instances = &report.databases.instances;
    let _ = writeln!(out, "<h2>Databases ({})</h2>", instances.len());
    if instances.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No database services detected.</p>");
        return;
    }
    let _ = writeln!(
        out,
        "<table><tr><th>Engine</th><th>Service</th><th>Endpoint</th><th>Exposure</th><th>Connections</th><th>Size</th><th>Replication</th><th>Locks</th></tr>"
    );
    for db in instances {
        let op = &db.operational;
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(db.engine.label()),
            esc(db.service.as_ref().map(|s| s.unit.as_str()).unwrap_or("-")),
            esc(&db.endpoint().unwrap_or_else(|| "-".to_owned())),
            database_exposure_label(db),
            esc(op.connection_summary.as_deref().unwrap_or("-")),
            esc(op.size_summary.as_deref().unwrap_or("-")),
            esc(op.replication_summary.as_deref().unwrap_or("-")),
            esc(op.lock_summary.as_deref().unwrap_or("-")),
        );
    }
    let _ = writeln!(out, "</table>");

    let credential_lines = instances
        .iter()
        .filter(|db| !db.credential_sources.is_empty())
        .map(|db| {
            let labels = db
                .credential_sources
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            (db.engine.label(), labels)
        })
        .collect::<Vec<_>>();
    if !credential_lines.is_empty() {
        let _ = writeln!(out, "<h3>Credential sources</h3><ul>");
        for (engine, labels) in credential_lines {
            let _ = writeln!(out, "<li><b>{}</b>: {}</li>", esc(engine), esc(&labels));
        }
        let _ = writeln!(out, "</ul>");
    }

    let errors: Vec<(&DatabaseInstance, _)> = instances
        .iter()
        .flat_map(|db| {
            db.operational
                .recent_errors
                .iter()
                .map(move |entry| (db, entry))
        })
        .collect();
    if !errors.is_empty() {
        let _ = writeln!(out, "<h3>Recent database errors</h3><ul>");
        for (db, entry) in errors.iter().take(10) {
            let _ = writeln!(
                out,
                "<li><b>{} [{}]</b> {} {}</li>",
                esc(db.engine.label()),
                entry.priority_label(),
                esc(&entry.time),
                esc(&entry.message)
            );
        }
        let _ = writeln!(out, "</ul>");
    }
}

fn database_exposure_label(db: &DatabaseInstance) -> &'static str {
    match db.exposure {
        Some(BindScope::External) => "external",
        Some(BindScope::Loopback) => "loopback",
        None => "unknown",
    }
}

fn failed_services(out: &mut String, report: &Report) {
    let units = &report.host.failed_units;
    let _ = writeln!(out, "<h2>Failed services ({})</h2>", units.len());
    if units.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">None.</p>");
        return;
    }
    let _ = writeln!(out, "<ul>");
    for unit in units {
        let _ = writeln!(
            out,
            "<li><code>{}</code> — {}</li>",
            esc(&unit.name),
            esc(&unit.description)
        );
    }
    let _ = writeln!(out, "</ul>");
}

fn crons(out: &mut String, report: &Report) {
    let _ = writeln!(out, "<h2>Crons</h2>");
    if report.crons.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No cron jobs.</p>");
    } else {
        let _ = writeln!(
            out,
            "<table><tr><th>State</th><th>Schedule</th><th>User</th><th>Command</th></tr>"
        );
        for entry in &report.crons {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                if entry.enabled { "enabled" } else { "disabled" },
                esc(&entry.schedule),
                esc(entry.user.as_deref().unwrap_or("—")),
                esc(&entry.command)
            );
        }
        let _ = writeln!(out, "</table>");
    }
    if !report.timers.is_empty() {
        let _ = writeln!(out, "<h3>Timers</h3>");
        let _ = writeln!(
            out,
            "<table><tr><th>Timer</th><th>Next</th><th>Activates</th></tr>"
        );
        for timer in &report.timers {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&timer.unit),
                esc(&timer.next),
                esc(&timer.activates)
            );
        }
        let _ = writeln!(out, "</table>");
    }
}

fn host_inventory(out: &mut String, report: &Report) {
    let snap = &report.host.snapshot;
    let _ = writeln!(out, "<h2>Host inventory</h2><ul>");
    let _ = writeln!(
        out,
        "<li><b>Label:</b> {}</li>",
        esc(&report.meta.host_label)
    );
    let _ = writeln!(out, "<li><b>Hostname:</b> {}</li>", esc(&snap.hostname));
    let _ = writeln!(
        out,
        "<li><b>OS:</b> {} · <b>Kernel:</b> {}</li>",
        esc(snap.os.as_deref().unwrap_or("unknown")),
        esc(&snap.kernel)
    );
    let _ = writeln!(
        out,
        "<li><b>CPU cores:</b> {} · <b>RAM:</b> {}</li>",
        snap.cpu.cores,
        human_kb(snap.memory.total_kb)
    );
    let _ = writeln!(
        out,
        "<li><b>Uptime:</b> {}</li>",
        human_uptime(snap.uptime_secs)
    );
    if let Some(caps) = &report.meta.capabilities {
        let _ = writeln!(out, "<li><b>Access:</b> {}</li>", esc(&caps.label()));
    }
    let _ = writeln!(out, "</ul>");
}

fn recommendations(out: &mut String, report: &Report) {
    let recs = unique_recommendations(&report.findings);
    let _ = writeln!(out, "<h2>Recommendations</h2>");
    if recs.is_empty() {
        let _ = writeln!(out, "<p class=\"muted\">No actions recommended.</p>");
        return;
    }
    let _ = writeln!(out, "<ul>");
    for rec in recs.iter().take(15) {
        let _ = writeln!(out, "<li>{}</li>", esc(rec));
    }
    let _ = writeln!(out, "</ul>");
}

fn notes(out: &mut String, report: &Report) {
    if report.notes.is_empty() {
        return;
    }
    let _ = writeln!(out, "<h2>Notes</h2><ul>");
    for note in &report.notes {
        let _ = writeln!(out, "<li>{}</li>", esc(note));
    }
    let _ = writeln!(out, "</ul>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReportMeta;
    use systui_collectors::{
        Container, ContainerHealth, CpuUsage, DatabaseCredentialKind, DatabaseCredentialSource,
        DatabaseEngine, DatabaseInstance, DatabaseOperational, DatabaseSnapshot, HealthReport,
        HostReport, LoadAverage, LogEntry, Memory, Swap, SystemSnapshot,
    };
    use systui_core::{ExecutionMode, Finding, ModuleId};

    fn report_with(findings: Vec<Finding>, containers: Vec<Container>) -> Report {
        Report {
            meta: ReportMeta {
                host_label: "prod-01".to_owned(),
                generated_at: "2026-05-24 10:00:00".to_owned(),
                mode: ExecutionMode::ReadOnly,
                capabilities: None,
                docker_available: !containers.is_empty(),
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
                    disks: Vec::new(),
                    users: Vec::new(),
                    cpu_model: None,
                    virtualization: None,
                },
                health: HealthReport {
                    score: 85,
                    checks: Vec::new(),
                },
                processes: Vec::new(),
                failed_units: Vec::new(),
                logs: Vec::new(),
            },
            network: None,
            exposures: Vec::new(),
            findings,
            containers,
            container_inspects: Vec::new(),
            container_stats: Vec::new(),
            databases: DatabaseSnapshot {
                instances: vec![DatabaseInstance {
                    engine: DatabaseEngine::Redis,
                    service: None,
                    listener: None,
                    version: Some("Redis server v=7.0.15".to_owned()),
                    exposure: None,
                    credential_sources: vec![DatabaseCredentialSource {
                        kind: DatabaseCredentialKind::Environment,
                        label: "REDISCLI_AUTH environment variable (value redacted)".to_owned(),
                    }],
                    operational: DatabaseOperational {
                        connection_summary: Some("12 connected clients".to_owned()),
                        recent_errors: vec![LogEntry {
                            time: "09:00:00".to_owned(),
                            priority: 3,
                            identifier: "redis-server".to_owned(),
                            message: "background save failed <unsafe>".to_owned(),
                        }],
                        ..Default::default()
                    },
                    detected_by: Vec::new(),
                }],
            },
            crons: Vec::new(),
            timers: Vec::new(),
            notes: Vec::new(),
        }
    }

    #[test]
    fn renders_self_contained_html_with_badges() {
        let report = report_with(
            vec![
                Finding::new(
                    "ssh.root-login",
                    Severity::High,
                    ModuleId::Security,
                    "SSH permits direct root login",
                )
                .recommendation("Set PermitRootLogin to no."),
            ],
            Vec::new(),
        );
        let html = to_html(&report, ReportScope::Full);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<style>")); // inline CSS, self-contained
        assert!(html.contains("<title>SysTUI Report — prod-01</title>"));
        assert!(html.contains("class=\"sev sev-high\">HIGH<"));
        assert!(html.contains("SSH permits direct root login"));
        assert!(html.contains("<h2>Databases (1)</h2>"));
        assert!(html.contains("12 connected clients"));
        assert!(html.contains("REDISCLI_AUTH environment variable"));
        assert!(html.contains("Set PermitRootLogin to no."));
        assert!(html.trim_end().ends_with("</html>"));
        // No external resources.
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn escapes_host_derived_text() {
        let report = report_with(
            vec![
                Finding::new(
                    "x",
                    Severity::Info,
                    ModuleId::Docker,
                    "Container <evil> flagged",
                )
                .with_evidence("name=<script>alert(1)</script>"),
            ],
            vec![Container {
                id: "abc".to_owned(),
                name: "a&b".to_owned(),
                image: "img<tag>".to_owned(),
                state: "running".to_owned(),
                status: String::new(),
                health: Some(ContainerHealth::Healthy),
                ports: String::new(),
                created: String::new(),
            }],
        );
        let html = to_html(&report, ReportScope::Full);
        // Raw metacharacters from host data never reach the output unescaped.
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<unsafe>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("background save failed &lt;unsafe&gt;"));
        assert!(html.contains("Container &lt;evil&gt; flagged"));
        assert!(html.contains("a&amp;b"));
        assert!(html.contains("img&lt;tag&gt;"));
    }
}
