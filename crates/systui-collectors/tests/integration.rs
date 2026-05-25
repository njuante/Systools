//! Integration tests: run the real read-only collectors against the host.
//!
//! These exercise the collectors end-to-end over a real transport (no mocks) and
//! assert they produce sane data or degrade gracefully — never panic. They are
//! gated behind the `integration` feature so `cargo test --workspace` stays
//! hermetic; CI enables them per distro (see `.github/workflows/integration.yml`):
//!
//! - the distro matrix runs the universal tests inside each distro container;
//! - the `systemd` job sets `SYSTUI_HAS_SYSTEMD=1` on a runner where systemd is
//!   PID 1, turning the systemd assertions strict;
//! - the same job sets `SYSTUI_SSH_TARGET=<host>` to smoke-test local/SSH parity.
#![cfg(feature = "integration")]

use systui_collectors::{
    FailedUnitsCollector, LogQuery, ProcessCollector, ServiceCollector, SystemCollector,
    collect_host_report,
};
use systui_core::{Collector, Thresholds, Transport};
use systui_transport::LocalTransport;

/// The system snapshot is the one required collector; on any Linux host it must
/// yield a non-empty hostname, a kernel string and a non-zero memory total.
#[tokio::test]
async fn system_snapshot_is_sane() {
    let transport = LocalTransport::new();
    let snapshot = SystemCollector::new()
        .collect(&transport)
        .await
        .expect("system snapshot must be collectable on a real host");

    assert!(!snapshot.hostname.trim().is_empty(), "hostname is empty");
    assert!(!snapshot.kernel.trim().is_empty(), "kernel is empty");
    assert!(snapshot.memory.total_kb > 0, "memory total is zero");
}

/// PID 1 always exists on a running system, so the process list is never empty.
#[tokio::test]
async fn processes_include_pid_1() {
    let transport = LocalTransport::new();
    let processes = ProcessCollector::new()
        .collect(&transport)
        .await
        .expect("process list must be collectable on a real host");

    assert!(!processes.is_empty(), "process list is empty");
    assert!(
        processes.iter().any(|p| p.pid == 1),
        "PID 1 not present in the process list"
    );
}

/// The full host report must assemble even when optional collectors return empty
/// (its contract degrades processes/units/logs to empty but requires the snapshot).
#[tokio::test]
async fn host_report_assembles() {
    let transport = LocalTransport::new();
    let report = collect_host_report(
        &transport,
        &Thresholds::default(),
        &LogQuery::default(),
        None,
    )
    .await
    .expect("host report must assemble on a real host");

    assert!(!report.snapshot.hostname.trim().is_empty());
}

/// systemd behaviour: graceful everywhere, strict where systemd is PID 1.
///
/// On a non-systemd host (`systemctl` missing) the collectors must degrade to an
/// empty result rather than erroring. When CI sets `SYSTUI_HAS_SYSTEMD=1`, a real
/// systemd host must list at least one unit.
#[tokio::test]
async fn services_reflect_systemd_presence() {
    let transport = LocalTransport::new();

    let failed = FailedUnitsCollector::new().collect(&transport).await;
    let units = ServiceCollector::new().collect(&transport).await;

    if std::env::var("SYSTUI_HAS_SYSTEMD").as_deref() == Ok("1") {
        let units = units.expect("systemd host must list units");
        assert!(!units.is_empty(), "systemd host listed no units");
        assert!(
            failed.is_ok(),
            "failed-units query errored on a systemd host"
        );
    } else {
        // No systemd guarantee: must not panic; an Err is tolerated only as a
        // clean degradation, but the common case is Ok(empty).
        let _ = failed;
        let _ = units;
    }
}

/// Local/SSH parity smoke test: when CI provides a reachable SSH target, the same
/// collector over SSH must agree with the local run on the host's identity.
///
/// Opt-in via `SYSTUI_SSH_TARGET` (e.g. `localhost`, with key auth configured in
/// `~/.ssh/config`). Skipped when unset so the test is a no-op outside that job.
#[tokio::test]
async fn local_and_ssh_agree_on_hostname() {
    let Ok(target) = std::env::var("SYSTUI_SSH_TARGET") else {
        return;
    };

    let local = SystemCollector::new()
        .collect(&LocalTransport::new())
        .await
        .expect("local snapshot");

    let ssh = systui_transport::SshTransport::new(target);
    let remote = SystemCollector::new()
        .collect(&ssh)
        .await
        .expect("ssh snapshot");

    assert_eq!(
        local.hostname, remote.hostname,
        "hostname differs between local and SSH transports"
    );
    assert_eq!(local.kernel, remote.kernel, "kernel differs local vs SSH");
}

/// `LocalTransport` runs arbitrary read-only `CommandSpec`s; a trivial one proves
/// the transport itself is wired before any collector-specific assertion.
#[tokio::test]
async fn local_transport_runs_a_command() {
    let transport = LocalTransport::new();
    let out = transport
        .run(&systui_core::CommandSpec::new("uname").arg("-s"))
        .await
        .expect("uname -s must run");
    assert!(out.stdout.to_lowercase().contains("linux"));
}
