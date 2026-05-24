//! Aggregates all v0.1 collectors into a single host snapshot, shared by the
//! dashboard (UI) and the report generator.

use serde::{Deserialize, Serialize};
use systui_core::{Collector, Result, Thresholds, Transport};

use crate::{
    FailedUnitsCollector, HealthReport, LogEntry, LogQuery, LogsCollector, Process,
    ProcessCollector, ServiceUnit, SystemCollector, SystemSnapshot, evaluate_health,
};

/// A complete collected view of a host at one point in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostReport {
    pub snapshot: SystemSnapshot,
    pub health: HealthReport,
    pub processes: Vec<Process>,
    pub failed_units: Vec<ServiceUnit>,
    pub logs: Vec<LogEntry>,
}

/// Run every v0.1 collector through `transport` and assemble a [`HostReport`].
///
/// The system snapshot is required; processes, failed units and logs are
/// best-effort and degrade to empty. Health is computed from the result.
pub async fn collect_host_report(
    transport: &dyn Transport,
    thresholds: &Thresholds,
    log_query: &LogQuery,
) -> Result<HostReport> {
    let snapshot = SystemCollector::new().collect(transport).await?;
    let processes = ProcessCollector::new()
        .collect(transport)
        .await
        .unwrap_or_default();
    let failed_units = FailedUnitsCollector::new()
        .collect(transport)
        .await
        .unwrap_or_default();
    let logs = LogsCollector::with_query(log_query.clone())
        .collect(transport)
        .await
        .unwrap_or_default();

    let recent_errors = logs.iter().filter(|e| e.is_error()).count();
    let health = evaluate_health(&snapshot, failed_units.len(), recent_errors, thresholds);

    Ok(HostReport {
        snapshot,
        health,
        processes,
        failed_units,
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    fn full_transport() -> MockTransport {
        MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0\n")
            .with_file("/proc/uptime", b"100.0 0\n".to_vec())
            .with_file("/proc/loadavg", b"0.1 0.2 0.3 1/1 1\n".to_vec())
            .with_file(
                "/proc/meminfo",
                b"MemTotal: 100 kB\nMemAvailable: 50 kB\n".to_vec(),
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

    #[tokio::test]
    async fn collects_all_parts() {
        let report = collect_host_report(
            &full_transport(),
            &Thresholds::default(),
            &LogQuery::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.snapshot.hostname, "prod-01");
        assert_eq!(report.processes.len(), 1);
        // df/who/systemctl/journalctl unconfigured -> empty, but no failure
        assert!(report.failed_units.is_empty());
        assert!(report.logs.is_empty());
        assert_eq!(report.health.score, 100);
    }

    #[tokio::test]
    async fn fails_when_system_snapshot_fails() {
        let transport = MockTransport::new(); // nothing configured
        assert!(
            collect_host_report(&transport, &Thresholds::default(), &LogQuery::default())
                .await
                .is_err()
        );
    }
}
