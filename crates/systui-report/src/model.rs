//! The report data model: a serializable snapshot of everything a server review
//! produces. JSON is this model verbatim; Markdown and HTML summarise it.

use serde::Serialize;
use systui_collectors::{
    Container, ContainerStats, CronEntry, DatabaseSnapshot, ExposureEntry, HostCapabilities,
    HostReport, InspectSummary, NetworkSnapshot, SystemdTimer,
};
use systui_core::{ExecutionMode, Finding};

/// How much of the report a human-format renderer emits. JSON always carries the
/// full model; Markdown/HTML honour this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportScope {
    /// Every section.
    Full,
    /// A security-focused report: summary, findings, exposed ports, containers,
    /// databases, recommendations and notes — skipping health, services, crons
    /// and inventory.
    Security,
}

/// Metadata describing how and when the report was produced.
#[derive(Debug, Clone, Serialize)]
pub struct ReportMeta {
    /// Display label of the host (inventory id or `ssh://user@host`, or `local`).
    pub host_label: String,
    /// Caller-supplied timestamp, e.g. `2026-05-24 10:00:00`, for deterministic output.
    pub generated_at: String,
    /// Execution mode in effect (after capability-based degradation).
    pub mode: ExecutionMode,
    /// What the connected user can do; explains any partial data.
    pub capabilities: Option<HostCapabilities>,
    /// Whether Docker was reachable on the host.
    pub docker_available: bool,
}

/// A complete server report: metadata plus every collected view and the findings.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub meta: ReportMeta,
    /// System snapshot, health, processes, failed units and recent logs.
    pub host: HostReport,
    pub network: Option<NetworkSnapshot>,
    /// Risk-ranked listeners (open/exposed ports).
    pub exposures: Vec<ExposureEntry>,
    /// Merged security + docker + cron findings, worst-first.
    pub findings: Vec<Finding>,
    pub containers: Vec<Container>,
    pub container_inspects: Vec<InspectSummary>,
    pub container_stats: Vec<ContainerStats>,
    pub databases: DatabaseSnapshot,
    pub crons: Vec<CronEntry>,
    pub timers: Vec<SystemdTimer>,
    /// Free-form notes captured during the review.
    pub notes: Vec<String>,
}

impl Report {
    /// Count findings at each severity as `[critical, high, medium, low, info]`.
    pub fn finding_counts(&self) -> [usize; 5] {
        use systui_core::Severity;
        let mut counts = [0usize; 5];
        for finding in &self.findings {
            let idx = match finding.severity {
                Severity::Critical => 0,
                Severity::High => 1,
                Severity::Medium => 2,
                Severity::Low => 3,
                Severity::Info => 4,
            };
            counts[idx] += 1;
        }
        counts
    }
}
