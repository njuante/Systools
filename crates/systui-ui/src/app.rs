//! Global application state for the TUI.

use systui_collectors::{HealthReport, LogEntry, LogQuery, Process, ServiceUnit, SystemSnapshot};
use systui_core::{ExecutionMode, ModuleId, Thresholds};

use crate::theme::Theme;

/// The top navigation tabs (`Product.md` §5 layout). More modules are added as
/// later phases land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    System,
    Processes,
    Services,
    Logs,
    Network,
    Docker,
    Security,
}

impl Tab {
    /// All tabs, in display order.
    pub const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::System,
        Tab::Processes,
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
            Tab::Processes => "Processes",
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
            Tab::Processes => ModuleId::Processes,
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

/// How the process list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSort {
    Cpu,
    Mem,
}

impl ProcessSort {
    pub fn label(self) -> &'static str {
        match self {
            ProcessSort::Cpu => "CPU",
            ProcessSort::Mem => "memory",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            ProcessSort::Cpu => ProcessSort::Mem,
            ProcessSort::Mem => ProcessSort::Cpu,
        }
    }
}

/// Whether keystrokes drive navigation or the log search box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
}

/// Preset time windows for the log view (`--since` value, label).
const LOG_WINDOWS: &[(Option<&str>, &str)] = &[
    (None, "all"),
    (Some("15 min ago"), "15m"),
    (Some("1 hour ago"), "1h"),
    (Some("1 day ago"), "24h"),
];

/// Min-priority levels cycled in the log view (priority number, label).
const LOG_LEVELS: &[(u8, &str)] = &[(3, "err+"), (4, "warning+"), (6, "info+"), (7, "all")];

/// Everything the renderer needs to draw a frame.
#[derive(Debug)]
pub struct App {
    pub host_label: String,
    pub mode: ExecutionMode,
    pub theme: Theme,
    pub active_tab: usize,
    pub view_state: ViewState,
    pub snapshot: Option<SystemSnapshot>,
    pub processes: Vec<Process>,
    pub process_sort: ProcessSort,
    pub failed_units: Vec<ServiceUnit>,
    pub logs: Vec<LogEntry>,
    pub log_query: LogQuery,
    pub log_search: String,
    pub input_mode: InputMode,
    pub health: Option<HealthReport>,
    pub thresholds: Thresholds,
    pub show_help: bool,
    pub should_quit: bool,
    pub refresh_requested: bool,
    pub logs_reload_requested: bool,
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
            processes: Vec::new(),
            process_sort: ProcessSort::Cpu,
            failed_units: Vec::new(),
            logs: Vec::new(),
            log_query: LogQuery::default(),
            log_search: String::new(),
            input_mode: InputMode::Normal,
            health: None,
            thresholds: Thresholds::default(),
            show_help: false,
            should_quit: false,
            refresh_requested: false,
            logs_reload_requested: false,
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

    /// Switch the process list ordering between CPU and memory.
    pub fn toggle_process_sort(&mut self) {
        self.process_sort = self.process_sort.toggled();
    }

    /// Enter incremental log-search mode.
    pub fn enter_search(&mut self) {
        self.input_mode = InputMode::Search;
    }

    /// Leave search mode, clearing the query.
    pub fn exit_search(&mut self) {
        self.input_mode = InputMode::Normal;
        self.log_search.clear();
    }

    /// Append a character to the log search query.
    pub fn push_search_char(&mut self, c: char) {
        self.log_search.push(c);
    }

    /// Remove the last character of the log search query.
    pub fn pop_search_char(&mut self) {
        self.log_search.pop();
    }

    /// Label of the current log min-priority level.
    pub fn log_level_label(&self) -> &'static str {
        LOG_LEVELS
            .iter()
            .find(|(p, _)| *p == self.log_query.min_priority)
            .map_or("custom", |(_, label)| label)
    }

    /// Label of the current log time window.
    pub fn log_window_label(&self) -> &'static str {
        LOG_WINDOWS
            .iter()
            .find(|(since, _)| since.map(str::to_owned) == self.log_query.since)
            .map_or("custom", |(_, label)| label)
    }

    /// Cycle the log min-priority level and request a logs reload.
    pub fn cycle_log_level(&mut self) {
        let current = LOG_LEVELS
            .iter()
            .position(|(p, _)| *p == self.log_query.min_priority)
            .unwrap_or(0);
        let (priority, _) = LOG_LEVELS[(current + 1) % LOG_LEVELS.len()];
        self.log_query.min_priority = priority;
        self.logs_reload_requested = true;
    }

    /// Cycle the log time window and request a logs reload.
    pub fn cycle_log_window(&mut self) {
        let current = LOG_WINDOWS
            .iter()
            .position(|(since, _)| since.map(str::to_owned) == self.log_query.since)
            .unwrap_or(0);
        let (since, _) = LOG_WINDOWS[(current + 1) % LOG_WINDOWS.len()];
        self.log_query.since = since.map(str::to_owned);
        self.logs_reload_requested = true;
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
        assert_eq!(app.current_tab(), Tab::Processes);
        app.select_tab(99);
        assert_eq!(app.current_tab(), Tab::Processes);
    }
}
