//! Bridges the synchronous UI loop to the async collectors.
//!
//! Collectors are `async` (they run through a [`Transport`]), but the render
//! loop is synchronous, so we drive them with `Runtime::block_on`. This is the
//! foundation's single-collector wiring; phase 1 generalises it into a proper
//! controller with background refresh.

use chrono::Local;
use systui_collectors::{
    DatabaseCollector, DockerCollector, InspectSummary, LogsCollector, NetworkCollector,
    collect_cron_entries, collect_host_report, collect_timers, container_stats, exposure_map,
    inspect_container,
};
use systui_core::{Collector, CoreError, Transport};
use systui_security::{cron_findings, database_findings, docker_findings, security_scan};
use tokio::runtime::Runtime;

use crate::app::{App, ViewState};

/// Re-run the collectors and fold the result into the app state.
///
/// The system snapshot is the core view: if it fails, the whole refresh fails.
/// Other collectors are best-effort and degrade to empty.
pub fn refresh_blocking(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    app.view_state = ViewState::Loading;
    match runtime.block_on(collect_host_report(
        transport,
        &app.thresholds,
        &app.log_query,
    )) {
        Ok(report) => {
            app.push_history(
                report.snapshot.cpu.busy_percent,
                report.snapshot.memory.used_percent(),
            );
            app.snapshot = Some(report.snapshot);
            app.processes = report.processes;
            app.failed_units = report.failed_units;
            app.logs = report.logs;
            app.health = Some(report.health);
            app.view_state = ViewState::Ready;
            refresh_network_security(runtime, transport, app);
            refresh_docker_crons(runtime, transport, app);
        }
        Err(err) => apply_error(app, err),
    }
}

/// Collect the network snapshot, exposure map and security findings. All are
/// read-only and best-effort: missing tools or permissions yield partial data,
/// never a failed refresh.
fn refresh_network_security(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    let network = runtime
        .block_on(NetworkCollector::new().collect(transport))
        .ok();
    let exposures = network
        .as_ref()
        .map(|net| exposure_map(&net.listeners))
        .unwrap_or_default();
    let findings = runtime.block_on(security_scan(
        transport,
        &exposures,
        app.cert_warning_days,
        &[],
    ));
    app.network = network;
    app.exposures = exposures;
    app.findings = findings;
    app.databases = runtime
        .block_on(DatabaseCollector::new().collect(transport))
        .unwrap_or_default();
    app.findings.extend(database_findings(&app.databases));
}

/// Collect Docker containers and cron jobs, fold their risk findings into the
/// shared findings list and store the data for the Docker/Crons tabs. All
/// read-only and best-effort: a missing `docker` or unreadable crontab degrades
/// to empty rather than failing the refresh. Must run after
/// [`refresh_network_security`], whose findings it appends to.
fn refresh_docker_crons(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    match runtime.block_on(DockerCollector::new().collect(transport)) {
        Ok(containers) => {
            let inspects: Vec<InspectSummary> = containers
                .iter()
                .filter_map(|c| {
                    runtime
                        .block_on(inspect_container(transport, &c.id))
                        .ok()
                        .flatten()
                })
                .collect();
            app.findings.extend(docker_findings(&inspects));
            app.container_stats = runtime
                .block_on(container_stats(transport))
                .unwrap_or_default();
            app.container_inspects = inspects;
            app.containers = containers;
            app.docker_available = true;
        }
        Err(_) => {
            app.docker_available = false;
            app.containers.clear();
            app.container_inspects.clear();
            app.container_stats.clear();
        }
    }

    let crons = runtime.block_on(collect_cron_entries(transport));
    app.findings
        .extend(runtime.block_on(cron_findings(transport, &crons)));
    app.crons = crons;
    app.timers = runtime.block_on(collect_timers(transport));
    app.now = Local::now().naive_local();

    // Merged findings (security + docker + cron) re-sorted worst-first.
    app.findings
        .sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
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
}
