//! Bridges the synchronous UI loop to the async collectors.
//!
//! Collectors are `async` (they run through a [`Transport`]), but the render
//! loop is synchronous, so we drive them with `Runtime::block_on`. This is the
//! foundation's single-collector wiring; phase 1 generalises it into a proper
//! controller with background refresh.

use systui_collectors::HostInfoCollector;
use systui_core::{Collector, CoreError, Transport};
use tokio::runtime::Runtime;

use crate::app::{App, ViewState};

/// Re-run the collectors and fold the result into the app state.
pub fn refresh_blocking(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    app.view_state = ViewState::Loading;
    match runtime.block_on(HostInfoCollector.collect(transport)) {
        Ok(info) => {
            app.host_info = Some(info);
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

    #[test]
    fn successful_refresh_populates_host_info() {
        let transport = MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0\n");
        let mut app = App::new("local", ExecutionMode::ReadOnly);

        refresh_blocking(&runtime(), &transport, &mut app);

        assert_eq!(app.view_state, ViewState::Ready);
        let info = app.host_info.expect("host info");
        assert_eq!(info.hostname, "prod-01");
        assert_eq!(info.kernel, "6.1.0");
    }

    #[test]
    fn failed_refresh_sets_error_state() {
        let transport = MockTransport::new(); // no responses configured
        let mut app = App::new("local", ExecutionMode::ReadOnly);

        refresh_blocking(&runtime(), &transport, &mut app);

        assert!(matches!(app.view_state, ViewState::Error(_)));
        assert!(app.host_info.is_none());
    }
}
