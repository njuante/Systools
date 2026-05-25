//! Bridges the synchronous UI loop to the async collectors.
//!
//! Collectors are `async` (they run through a [`Transport`]), but the render
//! loop is synchronous. The loop spawns [`gather`] as a background task on the
//! shared runtime and applies its [`RefreshResult`] with [`apply_refresh`] on
//! the main thread, so a refresh never blocks input or drawing and rendering
//! stays a pure function of `App`.

use std::time::Instant;

use chrono::{Local, NaiveDateTime};
use systui_collectors::{
    ComposeProject, Container, ContainerStats, CronEntry, DatabaseSnapshot, ExposureEntry,
    FirewallSnapshot, HealthReport, HostStatics, ImageHygiene, InspectSummary, LogEntry, LogQuery,
    LogsCollector, NetStatics, NetworkSnapshot, PackageUpdates, Process, ServiceUnit,
    SystemSnapshot, SystemdTimer, timing,
};
use systui_core::{Collector, CoreError, Finding, Thresholds, Transport};
use systui_report::collect::{
    gather_crons, gather_databases, gather_docker, gather_network, gather_packages,
    gather_services, gather_timers, host_report_within_timeout, merge_findings,
};
use systui_security::{PolicyFacts, PolicySelection, policy_findings};
use tokio::runtime::Runtime;

use crate::app::{App, ConnectivityResult, ViewState};

/// A complete, self-contained result of one refresh gather. The background task
/// produces it off the UI thread; [`apply_refresh`] folds it into `App` on the
/// main thread so only a finished, consistent result is ever swapped in.
#[derive(Debug)]
pub struct RefreshResult {
    pub snapshot: SystemSnapshot,
    pub processes: Vec<Process>,
    pub failed_units: Vec<ServiceUnit>,
    pub all_units: Vec<ServiceUnit>,
    pub enabled_units: Vec<String>,
    pub logs: Vec<LogEntry>,
    pub health: HealthReport,
    pub network: Option<NetworkSnapshot>,
    pub exposures: Vec<ExposureEntry>,
    pub firewall: FirewallSnapshot,
    pub databases: DatabaseSnapshot,
    pub containers: Vec<Container>,
    pub container_inspects: Vec<InspectSummary>,
    pub container_stats: Vec<ContainerStats>,
    pub docker_available: bool,
    pub compose_projects: Vec<ComposeProject>,
    pub image_hygiene: ImageHygiene,
    pub crons: Vec<CronEntry>,
    pub timers: Vec<SystemdTimer>,
    pub packages: PackageUpdates,
    pub now: NaiveDateTime,
    pub findings: Vec<Finding>,
}

/// The outcome of a background gather, sent over the refresh channel.
pub type RefreshOutcome = Result<RefreshResult, CoreError>;

/// Run every collector and assemble a [`RefreshResult`]. Pure with respect to
/// `App`: it touches no UI state, so it can run on a background task. Mirrors
/// the report gather, but produces the dashboard's in-memory shape.
///
/// The system snapshot is required: if it fails, the whole refresh fails and
/// the caller keeps the previous (good) data. Everything else is best-effort
/// and degrades to empty.
pub async fn gather(
    transport: &dyn Transport,
    thresholds: &Thresholds,
    log_query: &LogQuery,
    cert_warning_days: u32,
    policy_selection: &PolicySelection,
    host_statics: Option<HostStatics>,
    net_statics: Option<NetStatics>,
) -> RefreshOutcome {
    let started = Instant::now();

    // Independent collector groups run concurrently. Real dependencies are kept
    // *inside* each group (exposures→security_scan, docker→inspects→stats,
    // crons→cron_findings); the groups themselves share nothing, so a slow one
    // (e.g. the SSH-heavy security scan) overlaps the others instead of adding to
    // the total. host_report's failure fails the whole refresh. The slow-changing
    // tiers (`host_statics`/`net_statics`) are reused when present so the tick
    // skips re-reading them.
    let (report, net, dbs, docker, crons_group, timers, services, packages) = tokio::join!(
        host_report_within_timeout(transport, thresholds, log_query, host_statics),
        gather_network(transport, cert_warning_days, net_statics),
        gather_databases(transport),
        gather_docker(transport),
        gather_crons(transport),
        gather_timers(transport),
        gather_services(transport),
        gather_packages(transport),
    );

    let report = report?;
    let (network, exposures, security_findings, firewall) = net;
    let (databases, database_findings_v) = dbs;
    let (crons, cron_findings_v) = crons_group;
    let (all_units, enabled_units) = services;

    let findings = merge_findings(
        security_findings,
        database_findings_v,
        docker.findings,
        cron_findings_v,
        policy_findings(
            policy_selection,
            PolicyFacts {
                host_label: &report.snapshot.hostname,
                snapshot: &report.snapshot,
                network: network.as_ref(),
                exposures: &exposures,
                services: &all_units,
                containers: &docker.containers,
                container_inspects: &docker.inspects,
                docker_available: docker.available,
            },
        ),
    );

    tracing::info!(
        target: timing::PERF_TARGET,
        collector = "refresh_total",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
    );

    Ok(RefreshResult {
        snapshot: report.snapshot,
        processes: report.processes,
        failed_units: report.failed_units,
        all_units,
        enabled_units,
        logs: report.logs,
        health: report.health,
        network,
        exposures,
        firewall,
        databases,
        containers: docker.containers,
        container_inspects: docker.inspects,
        container_stats: docker.stats,
        docker_available: docker.available,
        compose_projects: docker.compose,
        image_hygiene: docker.hygiene,
        crons,
        timers,
        packages,
        now: Local::now().naive_local(),
        findings,
    })
}

/// Fold a finished gather into `App` on the main thread. Marks the refresh as
/// no longer in flight. On error the message is surfaced via [`ViewState`] but
/// the previous (good) data is left untouched, so a failed background refresh
/// never wipes the screen.
pub fn apply_refresh(app: &mut App, outcome: RefreshOutcome) {
    app.refreshing = false;
    match outcome {
        Ok(result) => {
            app.push_history(
                result.snapshot.cpu.busy_percent,
                result.snapshot.memory.used_percent(),
            );
            app.snapshot = Some(result.snapshot);
            app.processes = result.processes;
            app.failed_units = result.failed_units;
            app.all_units = result.all_units;
            app.enabled_units = result.enabled_units;
            app.logs = result.logs;
            app.health = Some(result.health);
            app.network = result.network;
            app.exposures = result.exposures;
            app.firewall = result.firewall;
            app.databases = result.databases;
            app.containers = result.containers;
            app.container_inspects = result.container_inspects;
            app.container_stats = result.container_stats;
            app.docker_available = result.docker_available;
            app.compose_projects = result.compose_projects;
            app.image_hygiene = result.image_hygiene;
            app.crons = result.crons;
            app.timers = result.timers;
            app.packages = result.packages;
            app.now = result.now;
            app.findings = result.findings;
            app.record_health_snapshot();
            app.view_state = ViewState::Ready;
        }
        Err(err) => apply_error(app, err),
    }
}

/// Run a full refresh synchronously and apply it. Used by tests and any caller
/// that wants to block; the live UI uses [`gather`] + [`apply_refresh`] off the
/// UI thread instead.
pub fn refresh_blocking(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    app.view_state = ViewState::Loading;
    let (host_statics, net_statics) = cached_statics(app);
    let outcome = runtime.block_on(gather(
        transport,
        &app.thresholds,
        &app.log_query,
        app.cert_warning_days,
        &app.policy_selection,
        host_statics,
        net_statics,
    ));
    apply_refresh(app, outcome);
}

/// Derive the slow-changing tiers from the data already on screen, so the next
/// gather can reuse them instead of re-reading. `None` on the first refresh
/// (nothing cached yet) makes that gather collect them fresh.
pub fn cached_statics(app: &App) -> (Option<HostStatics>, Option<NetStatics>) {
    let host = app.snapshot.as_ref().map(HostStatics::from_snapshot);
    let net = app.network.as_ref().map(NetStatics::from_snapshot);
    (host, net)
}

/// Probe each target's reachability with a short ping. Pure with respect to
/// `App` so it can run on a background task; the caller folds the results in.
/// Targets are probed sequentially — this runs off the UI thread, so the loop
/// stays responsive while it completes.
pub async fn run_connectivity(
    transport: &dyn Transport,
    targets: Vec<(String, String)>,
) -> Vec<ConnectivityResult> {
    use std::time::Duration;
    let mut results = Vec::with_capacity(targets.len());
    for (target, label) in targets {
        let result =
            match systui_collectors::ping(transport, &target, 3, Duration::from_secs(3)).await {
                Ok(p) => {
                    let reachable = p.received > 0;
                    let detail = if reachable {
                        match p.rtt_avg_ms {
                            Some(rtt) => format!("{rtt:.1}ms avg · {:.0}% loss", p.loss_percent),
                            None => format!("{:.0}% loss", p.loss_percent),
                        }
                    } else {
                        "no reply".to_owned()
                    };
                    ConnectivityResult {
                        target,
                        label,
                        reachable,
                        detail,
                    }
                }
                Err(err) => ConnectivityResult {
                    target,
                    label,
                    reachable: false,
                    detail: err.to_string(),
                },
            };
        results.push(result);
    }
    results
}

/// Re-collect only the logs, using the current log query (best-effort).
pub fn reload_logs_blocking(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    let collector = LogsCollector::with_query(app.log_query.clone());
    if let Ok(logs) = runtime.block_on(collector.collect(transport)) {
        app.logs = logs;
    }
}

fn apply_error(app: &mut App, err: CoreError) {
    app.view_state = match err {
        CoreError::PermissionDenied(msg) => ViewState::PermissionDenied(msg),
        other => ViewState::Error(other.to_string()),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::ExecutionMode;
    use systui_transport::MockTransport;

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn ready_transport() -> MockTransport {
        MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0\n")
            .with_file("/proc/uptime", b"123456.78 0\n".to_vec())
            .with_file("/proc/loadavg", b"0.52 0.58 0.59 1/100 200\n".to_vec())
            .with_file(
                "/proc/meminfo",
                b"MemTotal: 100 kB\nMemAvailable: 40 kB\n".to_vec(),
            )
            .with_file(
                "/proc/stat",
                b"cpu  1 0 1 8 0 0 0 0 0 0\ncpu0 1 0 1 8 0 0 0 0 0 0\n".to_vec(),
            )
            .with_stdout(
                "ps -eo pid,ppid,user,pcpu,pmem,comm",
                "  PID PPID USER %CPU %MEM COMMAND\n  1 0 root 0.0 0.1 systemd\n",
            )
    }

    #[test]
    fn successful_refresh_populates_snapshot_and_processes() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);

        refresh_blocking(&runtime(), &ready_transport(), &mut app);

        assert_eq!(app.view_state, ViewState::Ready);
        assert!(!app.refreshing);
        let snap = app.snapshot.as_ref().expect("snapshot");
        assert_eq!(snap.hostname, "prod-01");
        assert_eq!(snap.kernel, "6.1.0");
        assert_eq!(snap.memory.total_kb, 100);
        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].command, "systemd");
    }

    #[test]
    fn failed_refresh_sets_error_state() {
        let transport = MockTransport::new(); // no responses configured
        let mut app = App::new("local", ExecutionMode::ReadOnly);

        refresh_blocking(&runtime(), &transport, &mut app);

        assert!(matches!(app.view_state, ViewState::Error(_)));
        assert!(app.snapshot.is_none());
    }

    #[test]
    fn failed_refresh_keeps_previous_good_data() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        refresh_blocking(&runtime(), &ready_transport(), &mut app);
        assert_eq!(app.view_state, ViewState::Ready);

        // A later refresh against an unreachable host fails, but must not wipe
        // the snapshot already on screen.
        refresh_blocking(&runtime(), &MockTransport::new(), &mut app);
        assert!(matches!(app.view_state, ViewState::Error(_)));
        assert!(app.snapshot.is_some());
        assert_eq!(app.snapshot.as_ref().unwrap().hostname, "prod-01");
    }

    #[test]
    fn refresh_includes_policy_findings() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.policy_selection = PolicySelection::Matched {
            name: "local-policy".to_owned(),
            policy: Box::new(systui_core::Policy {
                expected_ports: vec![443],
                ..systui_core::Policy::default()
            }),
            source: systui_core::PolicySource::ExplicitHost,
        };

        refresh_blocking(&runtime(), &ready_transport(), &mut app);

        let ids: Vec<&str> = app.findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"policy.port.missing.local-policy.443"));
    }
}
