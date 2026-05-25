//! Concurrent collector groups shared by the dashboard refresh
//! (`systui-ui::data::gather`) and the headless report gather
//! ([`crate::gather_report`]).
//!
//! Each group keeps the *real* ordering dependency inside it
//! (exposures→security_scan, docker→inspects→stats, crons→cron_findings) and is
//! otherwise independent of the others, so callers drive the groups concurrently
//! with `tokio::join!`. Every group is best-effort: missing tools or permissions
//! degrade to empty data, never an error.

use std::future::Future;
use std::time::{Duration, Instant};

use systui_collectors::{
    ComposeProject, Container, ContainerStats, CronEntry, DatabaseCollector, DatabaseSnapshot,
    DockerCollector, ExposureEntry, FirewallCollector, FirewallSnapshot, HostReport, HostStatics,
    ImageHygiene, InspectSummary, LogQuery, NetStatics, NetworkCollector, NetworkSnapshot,
    PackageUpdates, PackagesCollector, ServiceCollector, ServiceUnit, SystemdTimer,
    UnitFilesCollector, collect_cron_entries, collect_host_report, collect_timers,
    compose_projects, container_stats, exposure_map, image_hygiene, inspect_container, timing,
};
use systui_core::{Collector, CoreError, Finding, Result, Thresholds, Transport};
use systui_security::{cron_findings, database_findings, docker_findings, security_scan};

/// Per-collector-group timeout. A slow or hung host degrades a group to partial
/// data (or fails the required host report) instead of stalling the refresh.
/// Generous enough not to trip a merely-slow host: a healthy group is sub-second
/// even over SSH.
pub const COLLECTOR_TIMEOUT: Duration = Duration::from_secs(12);

/// Drive `fut` under [`COLLECTOR_TIMEOUT`], returning `on_timeout` if it elapses.
/// Used for best-effort groups whose timeout degrades to empty data.
async fn within_timeout<T>(label: &str, on_timeout: T, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(COLLECTOR_TIMEOUT, fut).await {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(target: timing::PERF_TARGET, collector = label, "collector timed out");
            on_timeout
        }
    }
}

/// Run [`collect_host_report`] under [`COLLECTOR_TIMEOUT`]. The host report is
/// required, so a timeout maps to [`CoreError::Timeout`] (the caller keeps the
/// previous good data and surfaces the error) rather than degrading to empty.
pub async fn host_report_within_timeout(
    transport: &dyn Transport,
    thresholds: &Thresholds,
    log_query: &LogQuery,
    host_statics: Option<HostStatics>,
) -> Result<HostReport> {
    let fut = timing::timed(
        "host_report",
        collect_host_report(transport, thresholds, log_query, host_statics),
    );
    match tokio::time::timeout(COLLECTOR_TIMEOUT, fut).await {
        Ok(result) => result,
        Err(_) => Err(CoreError::Timeout(COLLECTOR_TIMEOUT)),
    }
}

/// Network snapshot → exposure map → security scan. The scan depends on the
/// exposures, so this chain stays ordered. Returns the snapshot, the exposures
/// and the security findings. `net_statics` reuses the slow-changing networking
/// (interfaces/routes/DNS) when present (tiered refresh); pass `None` for fresh.
pub async fn gather_network(
    transport: &dyn Transport,
    cert_warning_days: u32,
    net_statics: Option<NetStatics>,
) -> (
    Option<NetworkSnapshot>,
    Vec<ExposureEntry>,
    Vec<Finding>,
    FirewallSnapshot,
) {
    within_timeout(
        "network_group",
        (None, Vec::new(), Vec::new(), FirewallSnapshot::default()),
        async move {
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
            // The security scan depends on the exposures; the firewall read is
            // independent, so overlap them instead of running back-to-back.
            let firewall_collector = FirewallCollector::new();
            let (findings, firewall) = tokio::join!(
                timing::timed(
                    "security_scan",
                    security_scan(transport, &exposures, cert_warning_days, &[]),
                ),
                timing::timed("firewall", firewall_collector.collect(transport)),
            );
            let firewall = firewall.unwrap_or_default();
            (network, exposures, findings, firewall)
        },
    )
    .await
}

/// Database discovery and its findings.
pub async fn gather_databases(transport: &dyn Transport) -> (DatabaseSnapshot, Vec<Finding>) {
    within_timeout(
        "databases_group",
        (DatabaseSnapshot::default(), Vec::new()),
        async move {
            let databases = timing::timed("databases", DatabaseCollector::new().collect(transport))
                .await
                .unwrap_or_default();
            let findings = database_findings(&databases);
            (databases, findings)
        },
    )
    .await
}

/// Docker containers → per-container inspect → stats, plus risk findings. An
/// unreachable daemon yields an empty, unavailable view. The returned bool is
/// whether Docker is available.
pub async fn gather_docker(transport: &dyn Transport) -> DockerGather {
    within_timeout("docker_group", DockerGather::default(), async move {
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
                // Stats, compose projects and image hygiene are independent of
                // each other; overlap them instead of running back-to-back.
                let (stats, compose, hygiene) = tokio::join!(
                    timing::timed("container_stats", container_stats(transport)),
                    timing::timed("compose_projects", compose_projects(transport)),
                    timing::timed("image_hygiene", image_hygiene(transport)),
                );
                DockerGather {
                    containers,
                    inspects,
                    stats: stats.unwrap_or_default(),
                    available: true,
                    findings,
                    compose: compose.unwrap_or_default(),
                    hygiene: hygiene.unwrap_or_default(),
                }
            }
            Err(_) => DockerGather::default(),
        }
    })
    .await
}

/// The collected Docker view shared by the dashboard refresh and the report.
#[derive(Debug, Default)]
pub struct DockerGather {
    pub containers: Vec<Container>,
    pub inspects: Vec<InspectSummary>,
    pub stats: Vec<ContainerStats>,
    pub available: bool,
    pub findings: Vec<Finding>,
    pub compose: Vec<ComposeProject>,
    pub hygiene: ImageHygiene,
}

/// Cron entries → cron risk findings (the findings depend on the entries).
pub async fn gather_crons(transport: &dyn Transport) -> (Vec<CronEntry>, Vec<Finding>) {
    within_timeout("crons_group", (Vec::new(), Vec::new()), async move {
        let crons = timing::timed("crons", collect_cron_entries(transport)).await;
        let findings = timing::timed("cron_findings", cron_findings(transport, &crons)).await;
        (crons, findings)
    })
    .await
}

/// Full service unit list + the set of unit names enabled at boot. The
/// `--failed` fast path (in the host report) stays the live health signal; this
/// group backs the Services screen's ALL/RUNNING/INACTIVE/ENABLED filters. Both
/// reads run concurrently and degrade to empty.
pub async fn gather_services(transport: &dyn Transport) -> (Vec<ServiceUnit>, Vec<String>) {
    within_timeout("services_group", (Vec::new(), Vec::new()), async move {
        let service_collector = ServiceCollector::new();
        let unit_files_collector = UnitFilesCollector::new();
        let (units, files) = tokio::join!(
            timing::timed("service_units", service_collector.collect(transport)),
            timing::timed("unit_files", unit_files_collector.collect(transport)),
        );
        let units = units.unwrap_or_default();
        let enabled = files
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.is_enabled())
            .map(|f| f.name)
            .collect();
        (units, enabled)
    })
    .await
}

/// Pending package updates (apt/dnf/pacman/zypper), cache-only and best-effort.
/// Slow-changing, so it shares the concurrent gather but never blocks it.
pub async fn gather_packages(transport: &dyn Transport) -> PackageUpdates {
    within_timeout(
        "packages",
        PackageUpdates::default(),
        timing::timed("packages", async {
            PackagesCollector::new()
                .collect(transport)
                .await
                .unwrap_or_default()
        }),
    )
    .await
}

/// systemd timers (best-effort), bounded by the per-collector timeout.
pub async fn gather_timers(transport: &dyn Transport) -> Vec<SystemdTimer> {
    within_timeout(
        "timers",
        Vec::new(),
        timing::timed("timers", collect_timers(transport)),
    )
    .await
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
