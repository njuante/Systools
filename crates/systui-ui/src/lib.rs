//! SysTUI terminal UI: the ratatui/crossterm event loop, global application
//! state, tab navigation, theme, key bindings and the shared UI states
//! (loading, empty, error, permission-denied, ...). The UI only requests
//! actions; it never executes them.

pub mod app;
pub mod cron_builder;
pub mod data;
pub mod fleet;
pub mod form;
pub mod input;
pub mod theme;
pub mod ui;
pub mod visual_style;
pub mod widgets;

pub use app::{App, Tab, ViewState};
pub use fleet::{FleetExit, run_fleet};
pub use theme::Theme;
pub use visual_style::VisualStyle;

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use systui_actions::ActionEngine;
use systui_core::{AuditContext, Config, CoreError, ExecutionMode, Result, Transport};
use systui_storage::{AuditLog, StateStore};
use tokio::runtime::Runtime;

use crate::app::ConnectivityResult;
use crate::data::RefreshOutcome;

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
    // Shared, `Send + Sync` transport so the background gather task can own a
    // clone while the main thread keeps one for synchronous action calls.
    let transport: Arc<dyn Transport> = Arc::from(transport);
    let mut app = App::new(host_label, mode);
    app.set_theme_kind(theme::ThemeKind::from_config_name(&config.general.theme));
    app.set_visual_style(visual_style::VisualStyle::from_config_name(
        &config.general.visual_style,
    ));
    app.thresholds = config.thresholds.clone();
    app.cert_warning_days = config.security.cert_expiry_warning_days;
    app.policy_selection = systui_security::PolicySelection::for_host(config, &app.host_label);
    // Load persisted local state (trends, notes, saved searches); best-effort.
    let state_store = StateStore::at_default_location().ok();
    if let Some(store) = &state_store {
        app.state = store.load();
    }
    let refresh_interval = Duration::from_secs(config.general.default_refresh_seconds);

    // Probe what the connected user can do (once), then degrade the mode to match:
    // a non-privileged user can't run privileged actions. Works the same locally
    // and over SSH.
    let capabilities = runtime.block_on(systui_collectors::probe_capabilities(transport.as_ref()));
    app.mode = capabilities.effective_mode(app.mode);
    app.capabilities = Some(capabilities);
    app.view_state = ViewState::Loading;

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(
        &mut terminal,
        &mut app,
        &runtime,
        &transport,
        refresh_interval,
    );
    let _ = ratatui::try_restore();
    // Flush accumulated local state (snapshots/notes/searches) on exit.
    if let Some(store) = &state_store
        && app.state_dirty
    {
        let _ = store.save(&app.state);
    }
    result
}

/// Spawn a background gather on the shared runtime, posting its result to `tx`.
/// No-op if a gather is already in flight, so refresh requests coalesce and at
/// most one gather runs at a time. Returns whether a gather was started.
fn spawn_refresh(
    runtime: &Runtime,
    transport: &Arc<dyn Transport>,
    app: &mut App,
    tx: &Sender<RefreshOutcome>,
) -> bool {
    if app.refreshing {
        return false;
    }
    app.refreshing = true;
    let transport = transport.clone();
    let tx = tx.clone();
    let thresholds = app.thresholds.clone();
    let log_query = app.log_query.clone();
    let cert_warning_days = app.cert_warning_days;
    let policy_selection = app.policy_selection.clone();
    // Reuse the slow-changing tiers already on screen so this gather skips
    // re-reading them (tiered refresh); `None` on the first gather reads fresh.
    let (host_statics, net_statics) = data::cached_statics(app);
    runtime.spawn(async move {
        let outcome = data::gather(
            transport.as_ref(),
            &thresholds,
            &log_query,
            cert_warning_days,
            &policy_selection,
            host_statics,
            net_statics,
        )
        .await;
        // The receiver is dropped only when the loop exits; ignore send errors.
        let _ = tx.send(outcome);
    });
    true
}

/// Spawn the on-demand connectivity probes in the background, posting the
/// results to `tx`. No-op if a run is already in flight or there are no targets.
/// Probes never touch host state, so they bypass the action engine.
fn spawn_connectivity(
    runtime: &Runtime,
    transport: &Arc<dyn Transport>,
    app: &mut App,
    tx: &Sender<Vec<ConnectivityResult>>,
) {
    if app.connectivity_running {
        return;
    }
    let targets = app.connectivity_targets();
    if targets.is_empty() {
        return;
    }
    app.connectivity_running = true;
    let transport = transport.clone();
    let tx = tx.clone();
    runtime.spawn(async move {
        let results = data::run_connectivity(transport.as_ref(), targets).await;
        let _ = tx.send(results);
    });
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    runtime: &Runtime,
    transport: &Arc<dyn Transport>,
    refresh_interval: Duration,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<RefreshOutcome>();
    let (conn_tx, conn_rx) = mpsc::channel::<Vec<ConnectivityResult>>();
    // Kick off the first gather in the background; the loop draws a "refreshing"
    // frame until it lands rather than freezing on startup.
    spawn_refresh(runtime, transport, app, &tx);
    let mut last_refresh = Instant::now();

    while !app.should_quit {
        // Fold in any finished background gathers (non-blocking).
        while let Ok(outcome) = rx.try_recv() {
            data::apply_refresh(app, outcome);
            app.clamp_selections();
        }
        // Fold in finished connectivity probe runs (non-blocking).
        while let Ok(results) = conn_rx.try_recv() {
            app.connectivity = results;
            app.connectivity_running = false;
        }

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
            plan_action(runtime, transport.as_ref(), app);
        } else if app.action_exec_requested {
            app.action_exec_requested = false;
            execute_action(runtime, transport.as_ref(), app);
        }

        let auto_due = !refresh_interval.is_zero() && last_refresh.elapsed() >= refresh_interval;
        if app.refresh_requested || auto_due {
            app.refresh_requested = false;
            last_refresh = Instant::now();
            spawn_refresh(runtime, transport, app, &tx);
        } else if app.logs_reload_requested {
            // A log filter changed: re-collect only the logs.
            app.logs_reload_requested = false;
            data::reload_logs_blocking(runtime, transport.as_ref(), app);
        } else if app.service_detail_requested {
            // The Services selection changed: fetch that unit's detail.
            app.service_detail_requested = false;
            data::reload_service_detail_blocking(runtime, transport.as_ref(), app);
        }

        if app.connectivity_requested {
            app.connectivity_requested = false;
            spawn_connectivity(runtime, transport, app, &conn_tx);
        }

        if app.theme_persist_requested {
            app.theme_persist_requested = false;
            // Best-effort: a failed write just means the choice isn't remembered.
            let _ = systui_storage::save_general_theme(app.theme_kind.config_name());
        }

        if app.visual_style_persist_requested {
            app.visual_style_persist_requested = false;
            // Best-effort: a failed write just means the choice isn't remembered.
            let _ = systui_storage::save_general_visual_style(app.visual_style.config_name());
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
