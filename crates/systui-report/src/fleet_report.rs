//! The fleet report: one document covering a whole selection of hosts
//! (`Product.md` §4.16, §11 "Fleet report").
//!
//! Consistent with the per-host reports: **JSON is the full structured model**
//! (every host's complete [`Report`]); **Markdown and HTML are a digest** — a
//! fleet overview plus a condensed per-host section (health, finding counts, top
//! findings, exposed ports). All host-derived text in the HTML is escaped.

use std::fmt::Write as _;

use serde::Serialize;

use crate::fleet::{FleetReview, findings_summary};
use crate::model::Report;
use crate::util::{escape_html as esc, severity_class, severity_label};

/// The full, serializable fleet report: a summary plus every host's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct FleetReport {
    pub generated_at: String,
    pub reviewed: usize,
    pub unreachable: usize,
    pub hosts: Vec<FleetReportHost>,
}

/// One host in a [`FleetReport`]: its identity plus the full report or the error.
#[derive(Debug, Clone, Serialize)]
pub struct FleetReportHost {
    pub id: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FleetReport {
    /// Assemble a report from a gathered [`FleetReview`]. Hosts are ordered
    /// worst-first (failed first, then ascending health), matching the overview.
    pub fn from_review(review: &FleetReview) -> Self {
        let overview = review.overview();
        let hosts = overview
            .hosts
            .iter()
            .map(|summary| {
                let entry = review.hosts.iter().find(|h| h.id == summary.id);
                let (report, error) = match entry.map(|h| &h.report) {
                    Some(Ok(report)) => (Some(report.clone()), None),
                    Some(Err(err)) => (None, Some(err.clone())),
                    None => (None, Some("no result".to_owned())),
                };
                FleetReportHost {
                    id: summary.id.clone(),
                    tags: summary.tags.clone(),
                    favorite: summary.favorite,
                    status: if report.is_some() {
                        "reviewed"
                    } else {
                        "unreachable"
                    },
                    report,
                    error,
                }
            })
            .collect();
        Self {
            generated_at: review.generated_at.clone(),
            reviewed: overview.reviewed_count(),
            unreachable: overview.failed_count(),
            hosts,
        }
    }
}

/// The top `n` findings of a report, worst-first (findings are already sorted).
fn top_findings(report: &Report, n: usize) -> impl Iterator<Item = &systui_core::Finding> {
    report.findings.iter().take(n)
}

fn open_ports_count(report: &Report) -> usize {
    report
        .network
        .as_ref()
        .map(|net| net.listeners.len())
        .unwrap_or(0)
}

/// Render the fleet report as pretty-printed JSON (the full structured model).
pub fn fleet_to_json(report: &FleetReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_owned())
}

/// Render the fleet report as a Markdown digest.
pub fn fleet_to_markdown(report: &FleetReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# SysTUI Fleet Report");
    let _ = writeln!(out);
    let _ = writeln!(out, "_Generated: {}_", report.generated_at);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- **{} host(s)** · {} reviewed · {} unreachable",
        report.hosts.len(),
        report.reviewed,
        report.unreachable
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "## Overview");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Host | Tags | Health | Findings / status |");
    let _ = writeln!(out, "|------|------|--------|-------------------|");
    for host in &report.hosts {
        let tags = if host.tags.is_empty() {
            "—".to_owned()
        } else {
            host.tags.join(", ")
        };
        let fav = if host.favorite { "★ " } else { "" };
        let (health, status) = match &host.report {
            Some(r) => (
                format!("{}/100", r.host.health.score),
                findings_summary(&r.finding_counts()),
            ),
            None => (
                "—".to_owned(),
                format!("unreachable: {}", host.error.as_deref().unwrap_or("?")),
            ),
        };
        let _ = writeln!(out, "| {fav}{} | {tags} | {health} | {status} |", host.id);
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Hosts");
    for host in &report.hosts {
        let _ = writeln!(out);
        let Some(r) = &host.report else {
            let _ = writeln!(
                out,
                "### {} — unreachable\n\n_{}_",
                host.id,
                host.error.as_deref().unwrap_or("unknown error")
            );
            continue;
        };
        let snap = &r.host.snapshot;
        let _ = writeln!(out, "### {}", host.id);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- {} · kernel {} · health {}/100",
            snap.os.as_deref().unwrap_or("unknown"),
            snap.kernel,
            r.host.health.score
        );
        let counts = r.finding_counts();
        let _ = writeln!(
            out,
            "- Findings: {} · Open ports: {}",
            findings_summary(&counts),
            open_ports_count(r)
        );
        for finding in top_findings(r, 5) {
            let _ = writeln!(
                out,
                "  - **{}** — {}",
                severity_label(finding.severity),
                finding.title
            );
        }
    }
    out
}

/// Render the fleet report as a single self-contained, escaped HTML document.
pub fn fleet_to_html(report: &FleetReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "<!DOCTYPE html>");
    let _ = writeln!(out, "<html lang=\"en\"><head><meta charset=\"utf-8\">");
    let _ = writeln!(
        out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    );
    let _ = writeln!(out, "<title>SysTUI Fleet Report</title>");
    let _ = writeln!(out, "<style>{}</style></head><body>", crate::html::STYLE);

    let _ = writeln!(out, "<h1>SysTUI Fleet Report</h1>");
    let _ = writeln!(
        out,
        "<p class=\"meta\">Generated: {} · {} host(s) · {} reviewed · {} unreachable</p>",
        esc(&report.generated_at),
        report.hosts.len(),
        report.reviewed,
        report.unreachable
    );

    let _ = writeln!(out, "<h2>Overview</h2>");
    let _ = writeln!(
        out,
        "<table><tr><th>Host</th><th>Tags</th><th>Health</th><th>Findings / status</th></tr>"
    );
    for host in &report.hosts {
        let tags = if host.tags.is_empty() {
            "—".to_owned()
        } else {
            esc(&host.tags.join(", "))
        };
        let fav = if host.favorite { "★ " } else { "" };
        let (health, status) = match &host.report {
            Some(r) => (
                format!("{}/100", r.host.health.score),
                esc(&findings_summary(&r.finding_counts())),
            ),
            None => (
                "—".to_owned(),
                format!("unreachable: {}", esc(host.error.as_deref().unwrap_or("?"))),
            ),
        };
        let _ = writeln!(
            out,
            "<tr><td>{fav}{}</td><td>{tags}</td><td>{health}</td><td>{status}</td></tr>",
            esc(&host.id)
        );
    }
    let _ = writeln!(out, "</table>");

    let _ = writeln!(out, "<h2>Hosts</h2>");
    for host in &report.hosts {
        let _ = writeln!(out, "<h3>{}</h3>", esc(&host.id));
        let Some(r) = &host.report else {
            let _ = writeln!(
                out,
                "<p class=\"muted\">unreachable: {}</p>",
                esc(host.error.as_deref().unwrap_or("unknown error"))
            );
            continue;
        };
        let snap = &r.host.snapshot;
        let _ = writeln!(
            out,
            "<p class=\"meta\">{} · kernel {} · health {}/100 · open ports {}</p>",
            esc(snap.os.as_deref().unwrap_or("unknown")),
            esc(&snap.kernel),
            r.host.health.score,
            open_ports_count(r)
        );
        let _ = writeln!(out, "<ul>");
        for finding in top_findings(r, 5) {
            let _ = writeln!(
                out,
                "<li><span class=\"sev {}\">{}</span> {}</li>",
                severity_class(finding.severity),
                severity_label(finding.severity),
                esc(&finding.title)
            );
        }
        let _ = writeln!(out, "</ul>");
    }

    let _ = writeln!(out, "</body></html>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::FleetHostReport;
    use crate::model::{Report, ReportMeta};
    use systui_collectors::{HealthReport, HostReport, SystemSnapshot};
    use systui_core::{ExecutionMode, Finding, ModuleId, Severity};

    fn sample_report(hostname: &str, score: u8) -> Report {
        Report {
            meta: ReportMeta {
                host_label: hostname.to_owned(),
                generated_at: "2026-05-24 10:00:00".to_owned(),
                mode: ExecutionMode::ReadOnly,
                capabilities: None,
                docker_available: false,
            },
            host: HostReport {
                snapshot: SystemSnapshot {
                    hostname: hostname.to_owned(),
                    os: Some("Debian 12".to_owned()),
                    kernel: "6.1.0".to_owned(),
                    uptime_secs: 0,
                    load: Default::default(),
                    cpu: Default::default(),
                    memory: Default::default(),
                    swap: Default::default(),
                    disks: Vec::new(),
                    users: Vec::new(),
                },
                health: HealthReport {
                    score,
                    checks: Vec::new(),
                },
                processes: Vec::new(),
                failed_units: Vec::new(),
                logs: Vec::new(),
            },
            network: None,
            exposures: Vec::new(),
            findings: vec![Finding::new(
                "ssh.root-login",
                Severity::High,
                ModuleId::Security,
                "SSH permits direct root login",
            )],
            containers: Vec::new(),
            container_inspects: Vec::new(),
            container_stats: Vec::new(),
            databases: Default::default(),
            crons: Vec::new(),
            timers: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn review() -> FleetReview {
        FleetReview {
            generated_at: "2026-05-24 10:00:00".to_owned(),
            hosts: vec![
                FleetHostReport {
                    id: "prod-01".to_owned(),
                    tags: vec!["web".to_owned()],
                    favorite: false,
                    report: Ok(sample_report("prod-01", 88)),
                },
                FleetHostReport {
                    id: "db-01".to_owned(),
                    tags: vec!["db".to_owned()],
                    favorite: true,
                    report: Err("connection refused".to_owned()),
                },
            ],
        }
    }

    #[test]
    fn from_review_counts_and_orders_worst_first() {
        let report = FleetReport::from_review(&review());
        assert_eq!(report.reviewed, 1);
        assert_eq!(report.unreachable, 1);
        // Failed host sorts first.
        assert_eq!(report.hosts[0].id, "db-01");
        assert_eq!(report.hosts[0].status, "unreachable");
        assert_eq!(report.hosts[1].status, "reviewed");
    }

    #[test]
    fn json_carries_full_reports_and_errors() {
        let report = FleetReport::from_review(&review());
        let json = fleet_to_json(&report);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["reviewed"], 1);
        assert_eq!(value["hosts"][0]["error"], "connection refused");
        assert_eq!(value["hosts"][1]["report"]["host"]["health"]["score"], 88);
    }

    #[test]
    fn markdown_lists_overview_and_per_host_sections() {
        let md = fleet_to_markdown(&FleetReport::from_review(&review()));
        assert!(md.contains("# SysTUI Fleet Report"));
        assert!(md.contains("1 reviewed · 1 unreachable"));
        assert!(md.contains("### prod-01"));
        assert!(md.contains("SSH permits direct root login"));
        assert!(md.contains("unreachable: connection refused"));
    }

    #[test]
    fn html_is_escaped_and_self_contained() {
        let mut rev = review();
        rev.hosts[0].report = Err("<script>bad</script>".to_owned());
        let html = fleet_to_html(&FleetReport::from_review(&rev));
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("&lt;script&gt;bad&lt;/script&gt;"));
        assert!(!html.contains("<script>bad"));
    }
}
