//! SysTUI terminal UI: the ratatui/crossterm event loop, global application
//! state, tab navigation, theme, key bindings and the shared UI states
//! (loading, empty, error, permission-denied, ...). The UI only requests
//! actions; it never executes them.

pub mod app;
pub mod data;
pub mod input;
pub mod theme;
pub mod ui;

pub use app::{App, Tab, ViewState};
pub use theme::Theme;

use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use systui_core::{CoreError, ExecutionMode, Result, Transport};
use tokio::runtime::Runtime;

/// Launch the interactive TUI for a host and run until the user quits.
///
/// Sets up and tears down the terminal (alternate screen, raw mode) around a
/// synchronous render/event loop. Collectors run through the given transport on
/// a private current-thread runtime. Restores the terminal even on error.
pub fn run(
    transport: Box<dyn Transport>,
    host_label: impl Into<String>,
    mode: ExecutionMode,
) -> Result<()> {
    let runtime = Runtime::new().map_err(CoreError::Io)?;
    let mut app = App::new(host_label, mode);
    data::refresh_blocking(&runtime, transport.as_ref(), &mut app);

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &runtime, transport.as_ref());
    let _ = ratatui::try_restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    runtime: &Runtime,
    transport: &dyn Transport,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    input::handle_key(app, key);
                }
            }
        }

        if app.refresh_requested {
            app.refresh_requested = false;
            data::refresh_blocking(runtime, transport, app);
        }
    }
    Ok(())
}
