//! Global application state for the TUI.

use systui_collectors::SystemSnapshot;
use systui_core::{ExecutionMode, ModuleId};

use crate::theme::Theme;

/// The top navigation tabs (`Product.md` §5 layout). More modules are added as
/// later phases land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    System,
    Services,
    Logs,
    Network,
    Docker,
    Security,
}

impl Tab {
    /// All tabs, in display order.
    pub const ALL: [Tab; 7] = [
        Tab::Dashboard,
        Tab::System,
        Tab::Services,
        Tab::Logs,
        Tab::Network,
        Tab::Docker,
        Tab::Security,
    ];

    /// Title shown in the tab bar.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::System => "System",
            Tab::Services => "Services",
            Tab::Logs => "Logs",
            Tab::Network => "Network",
            Tab::Docker => "Docker",
            Tab::Security => "Security",
        }
    }

    /// The module this tab maps to.
    pub fn module(self) -> ModuleId {
        match self {
            Tab::Dashboard => ModuleId::Dashboard,
            Tab::System => ModuleId::System,
            Tab::Services => ModuleId::Services,
            Tab::Logs => ModuleId::Logs,
            Tab::Network => ModuleId::Network,
            Tab::Docker => ModuleId::Docker,
            Tab::Security => ModuleId::Security,
        }
    }
}

/// The render state of the active view (`Product.md` §5). A module that fails
/// renders one of these instead of crashing the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewState {
    Loading,
    Empty,
    Ready,
    PartialData(String),
    PermissionDenied(String),
    Error(String),
}

/// Everything the renderer needs to draw a frame.
#[derive(Debug)]
pub struct App {
    pub host_label: String,
    pub mode: ExecutionMode,
    pub theme: Theme,
    pub active_tab: usize,
    pub view_state: ViewState,
    pub snapshot: Option<SystemSnapshot>,
    pub show_help: bool,
    pub should_quit: bool,
    pub refresh_requested: bool,
}

impl App {
    /// Create the initial application state for a host.
    pub fn new(host_label: impl Into<String>, mode: ExecutionMode) -> Self {
        Self {
            host_label: host_label.into(),
            mode,
            theme: Theme::dark(),
            active_tab: 0,
            view_state: ViewState::Empty,
            snapshot: None,
            show_help: false,
            should_quit: false,
            refresh_requested: false,
        }
    }

    /// The currently selected tab.
    pub fn current_tab(&self) -> Tab {
        Tab::ALL[self.active_tab]
    }

    /// Move to the next tab, wrapping around.
    pub fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % Tab::ALL.len();
    }

    /// Move to the previous tab, wrapping around.
    pub fn prev_tab(&mut self) {
        self.active_tab = (self.active_tab + Tab::ALL.len() - 1) % Tab::ALL.len();
    }

    /// Select a tab by zero-based index, ignoring out-of-range values.
    pub fn select_tab(&mut self, index: usize) {
        if index < Tab::ALL.len() {
            self.active_tab = index;
        }
    }

    /// Toggle the help overlay.
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Ask the event loop to re-run collectors on its next tick.
    pub fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Request application exit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_dashboard_empty() {
        let app = App::new("local", ExecutionMode::ReadOnly);
        assert_eq!(app.current_tab(), Tab::Dashboard);
        assert_eq!(app.view_state, ViewState::Empty);
    }

    #[test]
    fn tab_navigation_wraps_both_ways() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        for _ in 0..Tab::ALL.len() - 1 {
            app.next_tab();
        }
        assert_eq!(app.current_tab(), Tab::Security);
        app.next_tab();
        assert_eq!(app.current_tab(), Tab::Dashboard);
        app.prev_tab();
        assert_eq!(app.current_tab(), Tab::Security);
    }

    #[test]
    fn select_tab_ignores_out_of_range() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.select_tab(2);
        assert_eq!(app.current_tab(), Tab::Services);
        app.select_tab(99);
        assert_eq!(app.current_tab(), Tab::Services);
    }
}
