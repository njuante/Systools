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
}
