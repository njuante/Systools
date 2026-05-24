//! SysTUI terminal UI: the ratatui/crossterm event loop, global application
//! state, tab navigation, theme, key bindings and the shared UI states
//! (loading, empty, error, permission-denied, ...). The UI only requests
//! actions; it never executes them.

pub mod app;
pub mod data;
pub mod fleet;
pub mod input;
pub mod theme;
pub mod ui;

pub use app::{App, Tab, ViewState};
pub use fleet::{FleetExit, run_fleet};
pub use theme::Theme;

use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use systui_actions::ActionEngine;
use systui_core::{AuditContext, Config, CoreError, ExecutionMode, Result, Transport};
use systui_storage::AuditLog;
use tokio::runtime::Runtime;

/// How often the event loop wakes to poll for input and check the refresh timer.
const TICK: Duration = Duration::from_millis(250);

/// Launch the interactive TUI for a host and run until the user quits.
///
/// Sets up and tears down the terminal (alternate screen, raw mode) around a
/// synchronous render/event loop. Collectors run through the given transport on
/// a private current-thread runtime. Auto-refresh interval and thresholds come
/// from `config`. Restores the terminal even on error.
pub fn run(
    transport: Box<dyn Transport>,
    host_label: impl Into<String>,
    mode: ExecutionMode,
    config: &Config,
) -> Result<()> {
    let runtime = Runtime::new().map_err(CoreError::Io)?;
    let mut app = App::new(host_label, mode);
    app.thresholds = config.thresholds.clone();
    app.cert_warning_days = config.security.cert_expiry_warning_days;
    let refresh_interval = Duration::from_secs(config.general.default_refresh_seconds);

    // Probe what the connected user can do (once), then degrade the mode to match:
    // a non-privileged user can't run privileged actions. Works the same locally
    // and over SSH.
    let capabilities = runtime.block_on(systui_collectors::probe_capabilities(transport.as_ref()));
    app.mode = capabilities.effective_mode(app.mode);
    app.capabilities = Some(capabilities);

    data::refresh_blocking(&runtime, transport.as_ref(), &mut app);

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(
        &mut terminal,
        &mut app,
        &runtime,
        transport.as_ref(),
        refresh_interval,
    );
    let _ = ratatui::try_restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    runtime: &Runtime,
    transport: &dyn Transport,
    refresh_interval: Duration,
) -> Result<()> {
    let mut last_refresh = Instant::now();

    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    input::handle_key(app, key);
                }
            }
        }

        if app.action_plan_requested {
            app.action_plan_requested = false;
            plan_action(runtime, transport, app);
        } else if app.action_exec_requested {
            app.action_exec_requested = false;
            execute_action(runtime, transport, app);
        }

        let auto_due = !refresh_interval.is_zero() && last_refresh.elapsed() >= refresh_interval;
        if app.refresh_requested || auto_due {
            app.refresh_requested = false;
            data::refresh_blocking(runtime, transport, app);
            app.clamp_selections();
            last_refresh = Instant::now();
        } else if app.logs_reload_requested {
            // A log filter changed: re-collect only the logs.
            app.logs_reload_requested = false;
            data::reload_logs_blocking(runtime, transport, app);
        }
    }
    Ok(())
}

/// Plan the pending action through the engine and update the modal.
fn plan_action(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    let Some(pending) = &app.pending else {
        return;
    };
    let engine = ActionEngine::new(app.mode);
    match runtime.block_on(engine.plan(pending.as_action(), transport)) {
        Ok(decision) => app.set_decision(decision),
        Err(err) => app.set_decision(systui_actions::ActionDecision::Rejected(err.to_string())),
    }
}

/// Execute the pending action, persist the audit record and show the result.
fn execute_action(runtime: &Runtime, transport: &dyn Transport, app: &mut App) {
    let Some(pending) = app.pending.take() else {
        return;
    };
    let confirmation = app
        .action
        .as_ref()
        .map(|m| m.input.clone())
        .unwrap_or_default();
    let ctx = AuditContext {
        host: app.host_label.clone(),
        user: std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned()),
    };

    let engine = ActionEngine::new(app.mode);
    let (result, record) =
        runtime.block_on(engine.run(pending.as_action(), transport, Some(&confirmation), &ctx));
    if let Ok(log) = AuditLog::at_default_location() {
        let _ = log.append(&record);
    }
    let message = match result {
        Ok(outcome) => outcome.message,
        Err(err) => err.to_string(),
    };
    app.set_action_result(message);
    app.request_refresh();
}
