//! SysTUI terminal UI: the ratatui/crossterm event loop, global application
//! state, tab navigation, theme, key bindings and the shared UI states
//! (loading, empty, error, permission-denied, ...). The UI only requests
//! actions; it never executes them.

pub mod app;
pub mod input;
pub mod theme;
pub mod ui;

pub use app::{App, Tab, ViewState};
pub use theme::Theme;

use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use systui_core::{ExecutionMode, Result};

/// Launch the interactive TUI for a host and run until the user quits.
///
/// Sets up and tears down the terminal (alternate screen, raw mode) around a
/// synchronous render/event loop. Restores the terminal even on error.
pub fn run(host_label: impl Into<String>, mode: ExecutionMode) -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let app = App::new(host_label, mode);
    let result = event_loop(&mut terminal, app);
    let _ = ratatui::try_restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, mut app: App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    input::handle_key(&mut app, key);
                }
            }
        }
    }
    Ok(())
}
