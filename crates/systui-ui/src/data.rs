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
    Container, ContainerStats, CronEntry, DatabaseCollector, DatabaseSnapshot, DockerCollector,
    ExposureEntry, HealthReport, InspectSummary, LogEntry, LogQuery, LogsCollector,
    NetworkCollector, NetworkSnapshot, Process, ServiceUnit, SystemSnapshot, SystemdTimer,
    collect_cron_entries, collect_host_report, collect_timers, container_stats, exposure_map,
    inspect_container, timing,
};
use systui_core::{Collector, CoreError, Finding, Thresholds, Transport};
use systui_security::{cron_findings, database_findings, docker_findings, security_scan};
use tokio::runtime::Runtime;

use crate::app::{App, ViewState};

/// A complete, self-contained result of one refresh gather. The background task
/// produces it off the UI thread; [`apply_refresh`] folds it into `App` on the
/// main thread so only a finished, consistent result is ever swapped in.
#[derive(Debug)]
pub struct RefreshResult {
    pub snapshot: SystemSnapshot,
    pub processes: Vec<Process>,
    pub failed_units: Vec<ServiceUnit>,
    pub logs: Vec<LogEntry>,
    pub health: HealthReport,
    pub network: Option<NetworkSnapshot>,
    pub exposures: Vec<ExposureEntry>,
    pub databases: DatabaseSnapshot,
    pub containers: Vec<Container>,
    pub container_inspects: Vec<InspectSummary>,
    pub container_stats: Vec<ContainerStats>,
    pub docker_available: bool,
    pub crons: Vec<CronEntry>,
    pub timers: Vec<SystemdTimer>,
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
) -> RefreshOutcome {
    let started = Instant::now();
    let report = timing::timed(
        "host_report",
        collect_host_report(transport, thresholds, log_query),
    )
    .await?;

    let network = timing::timed("network", NetworkCollector::new().collect(transport))
        .await
        .ok();
    let exposures = network
        .as_ref()
        .map(|net| exposure_map(&net.listeners))
        .unwrap_or_default();
    let mut findings = timing::timed(
        "security_scan",
        security_scan(transport, &exposures, cert_warning_days, &[]),
    )
    .await;
    let databases = timing::timed("databases", DatabaseCollector::new().collect(transport))
        .await
        .unwrap_or_default();
    findings.extend(database_findings(&databases));

    // Docker (best-effort; an unreachable daemon yields an empty, unavailable view).
    let (containers, container_inspects, container_stats_data, docker_available) =
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
                findings.extend(docker_findings(&inspects));
                let stats = timing::timed("container_stats", container_stats(transport))
                    .await
                    .unwrap_or_default();
                (containers, inspects, stats, true)
            }
            Err(_) => (Vec::new(), Vec::new(), Vec::new(), false),
        };

    let crons = timing::timed("crons", collect_cron_entries(transport)).await;
    findings.extend(timing::timed("cron_findings", cron_findings(transport, &crons)).await);
    let timers = timing::timed("timers", collect_timers(transport)).await;

    // Merged findings (security + docker + cron) sorted worst-first, deterministic.
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));

    tracing::info!(
        target: timing::PERF_TARGET,
        collector = "refresh_total",
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
    );

    Ok(RefreshResult {
        snapshot: report.snapshot,
        processes: report.processes,
        failed_units: report.failed_units,
        logs: report.logs,
        health: report.health,
        network,
        exposures,
        databases,
        containers,
        container_inspects,
        container_stats: container_stats_data,
        docker_available,
        crons,
        timers,
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
            app.logs = result.logs;
            app.health = Some(result.health);
            app.network = result.network;
            app.exposures = result.exposures;
            app.databases = result.databases;
            app.containers = result.containers;
            app.container_inspects = result.container_inspects;
            app.container_stats = result.container_stats;
            app.docker_available = result.docker_available;
            app.crons = result.crons;
            app.timers = result.timers;
            app.now = result.now;
            app.findings = result.findings;
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
    let outcome = runtime.block_on(gather(
        transport,
        &app.thresholds,
        &app.log_query,
        app.cert_warning_days,
    ));
    apply_refresh(app, outcome);
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
}
