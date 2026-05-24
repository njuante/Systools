//! Headless report gathering: run every collector and the security scans over a
//! [`Transport`] and assemble a [`Report`]. This is the non-TUI equivalent of the
//! dashboard refresh, used by the `report` CLI for local and remote hosts.

use std::time::Instant;

use systui_collectors::{LogQuery, probe_capabilities, timing};
use systui_core::{Config, ExecutionMode, Result, Transport};

use crate::collect::{
    gather_crons, gather_databases, gather_docker, gather_network, gather_timers,
    host_report_within_timeout, merge_findings,
};
use crate::model::{Report, ReportMeta};

/// Gather a full [`Report`] for a host over `transport`.
///
/// The system snapshot is required (its failure fails the report); every other
/// part is best-effort and degrades to empty, exactly like the dashboard. `mode`
/// is recorded after capability-based degradation. `generated_at` is injected by
/// the caller for deterministic output, and `notes` carries any review notes.
pub async fn gather_report(
    transport: &dyn Transport,
    config: &Config,
    host_label: impl Into<String>,
    mode: ExecutionMode,
    generated_at: impl Into<String>,
    notes: Vec<String>,
) -> Result<Report> {
    let capabilities = probe_capabilities(transport).await;
    let effective_mode = capabilities.effective_mode(mode);

    let gather_start = Instant::now();

    // Independent collector groups run concurrently, each bounded by the
    // per-collector timeout. Identical scheduling to the dashboard refresh.
    // One-shot gather: no session cache, so the slow-changing tiers are always
    // read fresh (`None`).
    let log_query = LogQuery::default();
    let (host, net, dbs, docker, crons_group, timers) = tokio::join!(
        host_report_within_timeout(transport, &config.thresholds, &log_query, None),
        gather_network(transport, config.security.cert_expiry_warning_days, None),
        gather_databases(transport),
        gather_docker(transport),
        gather_crons(transport),
        gather_timers(transport),
    );

    let host = host?;
    let (network, exposures, security_findings) = net;
    let (databases, database_findings_v) = dbs;
    let (containers, container_inspects, container_stats_data, docker_available, docker_findings_v) =
        docker;
    let (crons, cron_findings_v) = crons_group;

    let findings = merge_findings(
        security_findings,
        database_findings_v,
        docker_findings_v,
        cron_findings_v,
    );
    tracing::info!(
        target: timing::PERF_TARGET,
        collector = "gather_total",
        elapsed_ms = gather_start.elapsed().as_secs_f64() * 1000.0,
    );

    Ok(Report {
        meta: ReportMeta {
            host_label: host_label.into(),
            generated_at: generated_at.into(),
            mode: effective_mode,
            capabilities: Some(capabilities),
            docker_available,
        },
        host,
        network,
        exposures,
        findings,
        containers,
        container_inspects,
        container_stats: container_stats_data,
        databases,
        crons,
        timers,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    /// A transport with just enough for the required system snapshot to succeed;
    /// everything else degrades to empty.
    fn minimal_host() -> MockTransport {
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
    async fn gathers_a_report_and_degrades_missing_parts() {
        let report = gather_report(
            &minimal_host(),
            &Config::default(),
            "prod-01",
            ExecutionMode::Privileged,
            "2026-05-24 10:00:00",
            vec!["reviewed by ops".to_owned()],
        )
        .await
        .unwrap();

        assert_eq!(report.meta.host_label, "prod-01");
        assert_eq!(report.meta.generated_at, "2026-05-24 10:00:00");
        assert_eq!(report.host.snapshot.hostname, "prod-01");
        // No docker daemon configured on the mock → unavailable, not "no containers".
        assert!(!report.meta.docker_available);
        assert!(report.containers.is_empty());
        // Notes are carried through.
        assert_eq!(report.notes, ["reviewed by ops"]);
        // Capabilities were probed (id/sudo unconfigured → unknown, non-privileged),
        // so Privileged degrades to SafeActions.
        assert_eq!(report.meta.mode, ExecutionMode::SafeActions);
    }

    #[tokio::test]
    async fn fails_when_system_snapshot_fails() {
        let report = gather_report(
            &MockTransport::new(),
            &Config::default(),
            "x",
            ExecutionMode::ReadOnly,
            "t",
            Vec::new(),
        )
        .await;
        assert!(report.is_err());
    }
}
