//! Concurrent collector groups shared by the dashboard refresh
//! (`systui-ui::data::gather`) and the headless report gather
//! ([`crate::gather_report`]).
//!
//! Each group keeps the *real* ordering dependency inside it
//! (exposures→security_scan, docker→inspects→stats, crons→cron_findings) and is
//! otherwise independent of the others, so callers drive the groups concurrently
//! with `tokio::join!`. Every group is best-effort: missing tools or permissions
//! degrade to empty data, never an error.

use std::time::Instant;

use systui_collectors::{
    Container, ContainerStats, CronEntry, DatabaseCollector, DatabaseSnapshot, DockerCollector,
    ExposureEntry, InspectSummary, NetStatics, NetworkCollector, NetworkSnapshot,
    collect_cron_entries, container_stats, exposure_map, inspect_container, timing,
};
use systui_core::{Collector, Finding, Transport};
use systui_security::{cron_findings, database_findings, docker_findings, security_scan};

/// Network snapshot → exposure map → security scan. The scan depends on the
/// exposures, so this chain stays ordered. Returns the snapshot, the exposures
/// and the security findings. `net_statics` reuses the slow-changing networking
/// (interfaces/routes/DNS) when present (tiered refresh); pass `None` for fresh.
pub async fn gather_network(
    transport: &dyn Transport,
    cert_warning_days: u32,
    net_statics: Option<NetStatics>,
) -> (Option<NetworkSnapshot>, Vec<ExposureEntry>, Vec<Finding>) {
    let network = timing::timed(
        "network",
        NetworkCollector::with_statics(net_statics).collect(transport),
    )
    .await
    .ok();
    let exposures = network
        .as_ref()
        .map(|net| exposure_map(&net.listeners))
        .unwrap_or_default();
    let findings = timing::timed(
        "security_scan",
        security_scan(transport, &exposures, cert_warning_days, &[]),
    )
    .await;
    (network, exposures, findings)
}

/// Database discovery and its findings.
pub async fn gather_databases(transport: &dyn Transport) -> (DatabaseSnapshot, Vec<Finding>) {
    let databases = timing::timed("databases", DatabaseCollector::new().collect(transport))
        .await
        .unwrap_or_default();
    let findings = database_findings(&databases);
    (databases, findings)
}

/// Docker containers → per-container inspect → stats, plus risk findings. An
/// unreachable daemon yields an empty, unavailable view. The returned bool is
/// whether Docker is available.
pub async fn gather_docker(
    transport: &dyn Transport,
) -> (
    Vec<Container>,
    Vec<InspectSummary>,
    Vec<ContainerStats>,
    bool,
    Vec<Finding>,
) {
    match timing::timed("docker", DockerCollector::new().collect(transport)).await {
        Ok(containers) => {
            let inspect_start = Instant::now();
            let mut inspects: Vec<InspectSummary> = Vec::new();
            for c in &containers {
                if let Ok(Some(summary)) = inspect_container(transport, &c.id).await {
                    inspects.push(summary);
                }
            }
            tracing::info!(
                target: timing::PERF_TARGET,
                collector = "docker_inspects",
                elapsed_ms = inspect_start.elapsed().as_secs_f64() * 1000.0,
            );
            let findings = docker_findings(&inspects);
            let stats = timing::timed("container_stats", container_stats(transport))
                .await
                .unwrap_or_default();
            (containers, inspects, stats, true, findings)
        }
        Err(_) => (Vec::new(), Vec::new(), Vec::new(), false, Vec::new()),
    }
}

/// Cron entries → cron risk findings (the findings depend on the entries).
pub async fn gather_crons(transport: &dyn Transport) -> (Vec<CronEntry>, Vec<Finding>) {
    let crons = timing::timed("crons", collect_cron_entries(transport)).await;
    let findings = timing::timed("cron_findings", cron_findings(transport, &crons)).await;
    (crons, findings)
}

/// Merge the per-group findings in a fixed order (security, database, docker,
/// cron) and sort worst-first by `(severity desc, id asc)`. The order the groups
/// *finished* in is irrelevant: the result is deterministic.
pub fn merge_findings(
    security: Vec<Finding>,
    database: Vec<Finding>,
    docker: Vec<Finding>,
    cron: Vec<Finding>,
) -> Vec<Finding> {
    let mut findings = security;
    findings.extend(database);
    findings.extend(docker);
    findings.extend(cron);
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}
