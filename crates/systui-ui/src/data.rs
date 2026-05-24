//! Bridges the synchronous UI loop to the async collectors.
//!
//! Collectors are `async` (they run through a [`Transport`]), but the render
//! loop is synchronous, so we drive them with `Runtime::block_on`. This is the
//! foundation's single-collector wiring; phase 1 generalises it into a proper
//! controller with background refresh.

use systui_collectors::{FailedUnitsCollector, LogsCollector, ProcessCollector, SystemCollector};
use systui_core::{Collector, CoreError, Transport};
use tokio::runtime::Runtime;

use crate::app::{App, ViewState};

/// Re-run the collectors and fold the result into the app state.
///
/// The system snapshot is the core view: if it fails, the whole refresh fails.
/// Processes are best-effort and degrade to an empty list.
pub fn refresh_blocking(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    app.view_state = ViewState::Loading;
    match runtime.block_on(SystemCollector::new().collect(transport)) {
        Ok(snapshot) => {
            app.snapshot = Some(snapshot);
            app.processes = runtime
                .block_on(ProcessCollector::new().collect(transport))
                .unwrap_or_default();
            app.failed_units = runtime
                .block_on(FailedUnitsCollector::new().collect(transport))
                .unwrap_or_default();
            app.logs = runtime
                .block_on(LogsCollector::new().collect(transport))
                .unwrap_or_default();
            app.view_state = ViewState::Ready;
        }
        Err(err) => apply_error(app, err),
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
                "ps -eo pid,user,pcpu,pmem,comm",
                "  PID USER %CPU %MEM COMMAND\n  1 root 0.0 0.1 systemd\n",
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
