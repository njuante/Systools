//! Fleet overview model: aggregating per-host reviews into a single, sortable
//! snapshot of a fleet (`Product.md` §4.16).
//!
//! This is pure and serializable. Gathering the per-host [`Report`]s concurrently
//! is the caller's job — it owns the transports — so this module stays
//! transport-agnostic and easy to test. The overview is the headless equivalent of
//! the fleet table; both the CLI and a future TUI fleet view build on it.

use std::cmp::Ordering;

use serde::Serialize;
use systui_core::ExecutionMode;

use crate::Report;

/// The outcome of reviewing one fleet host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FleetOutcome {
    /// The host was reviewed; carries the headline numbers for the overview row.
    Reviewed {
        /// Health score (0–100).
        health: u8,
        /// Finding counts as `[critical, high, medium, low, info]`.
        finding_counts: [usize; 5],
        /// Execution mode after capability-based degradation.
        mode: ExecutionMode,
        docker_available: bool,
    },
    /// The host could not be reviewed (unreachable, auth failure, timeout, …).
    /// A failed host is *worst* — its state is unknown — so it sorts first.
    Failed { error: String },
}

/// A compact, human one-line summary of finding counts
/// `[critical, high, medium, low, info]`, e.g. `2 crit, 1 high` or `OK`.
/// Low/info are omitted to keep overview rows tight.
pub fn findings_summary(counts: &[usize; 5]) -> String {
    let mut parts = Vec::new();
    for (count, label) in [(counts[0], "crit"), (counts[1], "high"), (counts[2], "med")] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if parts.is_empty() {
        "OK".to_owned()
    } else {
        parts.join(", ")
    }
}

/// One host's row in the fleet overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FleetHostSummary {
    /// Inventory id.
    pub id: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub outcome: FleetOutcome,
}

impl FleetHostSummary {
    /// Build a summary from a successfully gathered [`Report`].
    pub fn reviewed(
        id: impl Into<String>,
        tags: Vec<String>,
        favorite: bool,
        report: &Report,
    ) -> Self {
        Self {
            id: id.into(),
            tags,
            favorite,
            outcome: FleetOutcome::Reviewed {
                health: report.host.health.score,
                finding_counts: report.finding_counts(),
                mode: report.meta.mode,
                docker_available: report.meta.docker_available,
            },
        }
    }

    /// Build a summary for a host that could not be reviewed.
    pub fn failed(
        id: impl Into<String>,
        tags: Vec<String>,
        favorite: bool,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            tags,
            favorite,
            outcome: FleetOutcome::Failed {
                error: error.into(),
            },
        }
    }
}

/// A complete fleet overview: every reviewed (or failed) host, worst-first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FleetOverview {
    pub generated_at: String,
    pub hosts: Vec<FleetHostSummary>,
}

impl FleetOverview {
    /// Assemble an overview from per-host summaries, sorted **worst-first**:
    /// failed hosts come first (unknown state needs attention), then reviewed hosts
    /// by ascending health, then by descending critical+high findings, then id.
    /// The ordering is total and deterministic, so the overview is reproducible.
    pub fn build(generated_at: impl Into<String>, mut hosts: Vec<FleetHostSummary>) -> Self {
        hosts.sort_by(Self::worst_first);
        Self {
            generated_at: generated_at.into(),
            hosts,
        }
    }

    fn worst_first(a: &FleetHostSummary, b: &FleetHostSummary) -> Ordering {
        Self::rank(&a.outcome)
            .cmp(&Self::rank(&b.outcome))
            .then_with(|| match (&a.outcome, &b.outcome) {
                (
                    FleetOutcome::Reviewed {
                        health: ha,
                        finding_counts: fa,
                        ..
                    },
                    FleetOutcome::Reviewed {
                        health: hb,
                        finding_counts: fb,
                        ..
                    },
                ) => ha
                    .cmp(hb)
                    .then_with(|| (fb[0] + fb[1]).cmp(&(fa[0] + fa[1]))),
                _ => Ordering::Equal,
            })
            .then_with(|| a.id.cmp(&b.id))
    }

    /// Failed hosts rank before reviewed ones.
    fn rank(outcome: &FleetOutcome) -> u8 {
        match outcome {
            FleetOutcome::Failed { .. } => 0,
            FleetOutcome::Reviewed { .. } => 1,
        }
    }

    /// Number of hosts that were reviewed (vs. failed).
    pub fn reviewed_count(&self) -> usize {
        self.hosts
            .iter()
            .filter(|h| matches!(h.outcome, FleetOutcome::Reviewed { .. }))
            .count()
    }

    /// Number of hosts that could not be reviewed.
    pub fn failed_count(&self) -> usize {
        self.hosts.len() - self.reviewed_count()
    }
}

/// A fleet review that keeps each host's full [`Report`] (or its failure reason),
/// so global search, comparison and drift can work over real per-host data. The
/// lightweight [`FleetOverview`] is derived from it.
#[derive(Debug, Clone)]
pub struct FleetReview {
    pub generated_at: String,
    pub hosts: Vec<FleetHostReport>,
}

/// One host's place in a [`FleetReview`]: its identity plus the gathered report or
/// a human reason it could not be gathered.
#[derive(Debug, Clone)]
pub struct FleetHostReport {
    pub id: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub report: std::result::Result<Report, String>,
}

impl FleetReview {
    /// Derive the worst-first overview (the cheap summary view).
    pub fn overview(&self) -> FleetOverview {
        let summaries = self
            .hosts
            .iter()
            .map(|h| match &h.report {
                Ok(report) => {
                    FleetHostSummary::reviewed(h.id.clone(), h.tags.clone(), h.favorite, report)
                }
                Err(error) => FleetHostSummary::failed(
                    h.id.clone(),
                    h.tags.clone(),
                    h.favorite,
                    error.clone(),
                ),
            })
            .collect();
        FleetOverview::build(self.generated_at.clone(), summaries)
    }

    /// The gathered report for a host id, if it was reviewed successfully.
    pub fn report(&self, id: &str) -> Option<&Report> {
        self.hosts
            .iter()
            .find(|h| h.id == id)
            .and_then(|h| h.report.as_ref().ok())
    }

    /// Global search: list hosts whose open ports or services match `term`.
    ///
    /// A numeric term matches an open port; any term is also matched
    /// case-insensitively against service names (listener processes/units,
    /// container names, database engines, failed units). Failed hosts are skipped.
    pub fn search(&self, term: &str) -> Vec<HostMatches> {
        let needle = term.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let as_port: Option<u16> = needle.parse().ok();
        let mut results = Vec::new();
        for host in &self.hosts {
            let Ok(report) = &host.report else { continue };
            let (ports, services) = HostFacts::from_report(report).search_matches(&needle, as_port);
            if !ports.is_empty() || !services.is_empty() {
                results.push(HostMatches {
                    id: host.id.clone(),
                    ports,
                    services,
                });
            }
        }
        results
    }
}

/// Hosts matched by a global search, with the matching ports and services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostMatches {
    pub id: String,
    pub ports: Vec<u16>,
    pub services: Vec<String>,
}

/// The key facts of one host, extracted from its [`Report`] for comparison,
/// drift and search. Services and ports are canonical (sorted, de-duplicated;
/// service names lower-cased) so two hosts diff cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostFacts {
    pub os: String,
    pub kernel: String,
    pub health: u8,
    pub open_ports: std::collections::BTreeSet<u16>,
    pub services: std::collections::BTreeSet<String>,
    pub finding_counts: [usize; 5],
}

impl HostFacts {
    pub fn from_report(report: &Report) -> Self {
        let mut open_ports = std::collections::BTreeSet::new();
        let mut services = std::collections::BTreeSet::new();

        if let Some(net) = &report.network {
            for listener in &net.listeners {
                open_ports.insert(listener.port);
                if let Some(process) = &listener.process {
                    services.insert(process.name.to_lowercase());
                }
                if let Some(unit) = &listener.unit {
                    services.insert(unit.to_lowercase());
                }
            }
        }
        for container in &report.containers {
            services.insert(container.name.to_lowercase());
        }
        for instance in &report.databases.instances {
            services.insert(instance.engine.id().to_lowercase());
        }
        for unit in &report.host.failed_units {
            services.insert(unit.name.to_lowercase());
        }

        Self {
            os: report
                .host
                .snapshot
                .os
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            kernel: report.host.snapshot.kernel.clone(),
            health: report.host.health.score,
            open_ports,
            services,
            finding_counts: report.finding_counts(),
        }
    }

    /// Ports and services in these facts matching a search needle. `needle` is
    /// already lower-cased; `as_port` is the needle parsed as a port when numeric.
    fn search_matches(&self, needle: &str, as_port: Option<u16>) -> (Vec<u16>, Vec<String>) {
        let ports = match as_port {
            Some(port) if self.open_ports.contains(&port) => vec![port],
            _ => Vec::new(),
        };
        let services = self
            .services
            .iter()
            .filter(|service| service.contains(needle))
            .cloned()
            .collect();
        (ports, services)
    }
}

/// A side-by-side comparison of two hosts, with helpers for the drift deltas
/// (what each host has that the other does not).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostComparison {
    pub left_id: String,
    pub right_id: String,
    pub left: HostFacts,
    pub right: HostFacts,
}

impl HostComparison {
    pub fn new(
        left_id: impl Into<String>,
        left: &Report,
        right_id: impl Into<String>,
        right: &Report,
    ) -> Self {
        Self {
            left_id: left_id.into(),
            right_id: right_id.into(),
            left: HostFacts::from_report(left),
            right: HostFacts::from_report(right),
        }
    }

    /// Ports open on the left host but not the right.
    pub fn ports_only_left(&self) -> Vec<u16> {
        self.left
            .open_ports
            .difference(&self.right.open_ports)
            .copied()
            .collect()
    }

    /// Ports open on the right host but not the left.
    pub fn ports_only_right(&self) -> Vec<u16> {
        self.right
            .open_ports
            .difference(&self.left.open_ports)
            .copied()
            .collect()
    }

    /// Services on the left host but not the right.
    pub fn services_only_left(&self) -> Vec<String> {
        self.left
            .services
            .difference(&self.right.services)
            .cloned()
            .collect()
    }

    /// Services on the right host but not the left.
    pub fn services_only_right(&self) -> Vec<String> {
        self.right
            .services
            .difference(&self.left.services)
            .cloned()
            .collect()
    }

    /// Whether the two hosts differ in any compared port or service.
    pub fn has_drift(&self) -> bool {
        self.left.open_ports != self.right.open_ports || self.left.services != self.right.services
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewed(id: &str, health: u8, critical: usize, high: usize) -> FleetHostSummary {
        FleetHostSummary {
            id: id.to_owned(),
            tags: Vec::new(),
            favorite: false,
            outcome: FleetOutcome::Reviewed {
                health,
                finding_counts: [critical, high, 0, 0, 0],
                mode: ExecutionMode::ReadOnly,
                docker_available: false,
            },
        }
    }

    fn failed(id: &str) -> FleetHostSummary {
        FleetHostSummary::failed(id, Vec::new(), false, "connection refused")
    }

    #[test]
    fn failed_hosts_sort_before_reviewed() {
        let overview =
            FleetOverview::build("t", vec![reviewed("prod-01", 90, 0, 0), failed("db-01")]);
        assert_eq!(overview.hosts[0].id, "db-01");
        assert_eq!(overview.hosts[1].id, "prod-01");
    }

    #[test]
    fn reviewed_hosts_sort_by_ascending_health() {
        let overview = FleetOverview::build(
            "t",
            vec![
                reviewed("a", 91, 0, 0),
                reviewed("b", 64, 0, 0),
                reviewed("c", 82, 0, 0),
            ],
        );
        let ids: Vec<_> = overview.hosts.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    #[test]
    fn equal_health_breaks_ties_by_severity_then_id() {
        let overview = FleetOverview::build(
            "t",
            vec![
                reviewed("z", 70, 0, 1),
                reviewed("a", 70, 2, 0),
                reviewed("m", 70, 0, 1),
            ],
        );
        let ids: Vec<_> = overview.hosts.iter().map(|h| h.id.as_str()).collect();
        // `a` has more critical+high (2) so it leads; then the two with 1, by id.
        assert_eq!(ids, ["a", "m", "z"]);
    }

    #[test]
    fn counts_split_reviewed_and_failed() {
        let overview =
            FleetOverview::build("t", vec![reviewed("a", 90, 0, 0), failed("b"), failed("c")]);
        assert_eq!(overview.reviewed_count(), 1);
        assert_eq!(overview.failed_count(), 2);
    }

    fn facts(ports: &[u16], services: &[&str]) -> HostFacts {
        HostFacts {
            os: "Debian 12".to_owned(),
            kernel: "6.1.0".to_owned(),
            health: 90,
            open_ports: ports.iter().copied().collect(),
            services: services.iter().map(|s| (*s).to_owned()).collect(),
            finding_counts: [0; 5],
        }
    }

    #[test]
    fn search_matches_numeric_port_and_service_substring() {
        let f = facts(&[22, 443, 6379], &["nginx", "redis-server", "sshd"]);
        // Numeric term hits an open port.
        assert_eq!(f.search_matches("443", Some(443)), (vec![443], Vec::new()));
        // A port that is not open does not match.
        assert_eq!(
            f.search_matches("8080", Some(8080)),
            (Vec::new(), Vec::new())
        );
        // Substring matches services (case already lower).
        let (ports, services) = f.search_matches("redis", None);
        assert!(ports.is_empty());
        assert_eq!(services, ["redis-server"]);
    }

    #[test]
    fn comparison_reports_drift_both_ways() {
        let cmp = HostComparison {
            left_id: "a".to_owned(),
            right_id: "b".to_owned(),
            left: facts(&[22, 80, 443], &["nginx", "sshd"]),
            right: facts(&[22, 443, 5432], &["postgres", "sshd"]),
        };
        assert_eq!(cmp.ports_only_left(), [80]);
        assert_eq!(cmp.ports_only_right(), [5432]);
        assert_eq!(cmp.services_only_left(), ["nginx"]);
        assert_eq!(cmp.services_only_right(), ["postgres"]);
        assert!(cmp.has_drift());
    }

    #[test]
    fn identical_hosts_have_no_drift() {
        let cmp = HostComparison {
            left_id: "a".to_owned(),
            right_id: "b".to_owned(),
            left: facts(&[22, 443], &["nginx"]),
            right: facts(&[443, 22], &["nginx"]),
        };
        assert!(!cmp.has_drift());
        assert!(cmp.ports_only_left().is_empty());
        assert!(cmp.services_only_right().is_empty());
    }
}
