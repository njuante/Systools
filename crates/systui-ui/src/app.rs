//! Global application state for the TUI.

use chrono::{Local, NaiveDateTime};
use systui_actions::{
    ActionDecision, CronAction, DockerAction, DockerOp, DockerPruneAction, ServiceAction,
    ServiceOp, Signal, SignalAction,
};
use systui_collectors::{
    ComposeProject, Container, ContainerStats, CronEntry, CronSource, DatabaseSnapshot,
    ExposureEntry, FirewallSnapshot, HealthReport, HostCapabilities, ImageHygiene, InspectSummary,
    LogEntry, LogQuery, NetworkSnapshot, PackageUpdates, Process, ServiceUnit, SystemSnapshot,
    SystemdTimer, parse_schedule,
};
use systui_core::{Action, ExecutionMode, Finding, ModuleId, Severity, Thresholds};
use systui_storage::PersistentState;

use crate::form::{Field, Form};
use crate::theme::Theme;

/// A concrete action queued for the engine. An enum (not a trait object) so
/// [`App`] stays `Debug`.
#[derive(Debug, Clone)]
pub enum PendingAction {
    Service(ServiceAction),
    Signal(SignalAction),
    Docker(DockerAction),
    DockerPrune(DockerPruneAction),
    Cron(CronAction),
}

impl PendingAction {
    pub fn as_action(&self) -> &dyn Action {
        match self {
            PendingAction::Service(a) => a,
            PendingAction::Signal(a) => a,
            PendingAction::Docker(a) => a,
            PendingAction::DockerPrune(a) => a,
            PendingAction::Cron(a) => a,
        }
    }
}

/// Whether the cron form creates a new job or edits an existing user-crontab job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronFormMode {
    Add,
    Edit {
        original_schedule: String,
        original_command: String,
    },
}

/// Cron form overlay state.
#[derive(Debug, Clone)]
pub struct CronFormState {
    pub mode: CronFormMode,
    pub form: Form,
}

/// Stage of the action confirmation modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStage {
    /// Typed confirmation required.
    Confirm,
    /// Low-risk; press Enter to run.
    Ready,
    /// Terminal: showing the outcome or rejection.
    Result,
}

/// The action confirmation/result overlay.
#[derive(Debug, Clone)]
pub struct ActionModal {
    pub stage: ActionStage,
    pub title: String,
    pub details: Vec<String>,
    pub phrase: String,
    pub input: String,
    pub message: String,
}

impl ActionModal {
    fn rejected(message: String) -> Self {
        Self {
            stage: ActionStage::Result,
            title: "Action blocked".to_owned(),
            details: Vec::new(),
            phrase: String::new(),
            input: String::new(),
            message,
        }
    }

    fn from_decision(decision: ActionDecision) -> Self {
        match decision {
            ActionDecision::Rejected(message) => Self::rejected(message),
            ActionDecision::NeedsConfirmation { preview, phrase } => Self {
                stage: ActionStage::Confirm,
                title: preview.summary,
                details: preview.details,
                phrase,
                input: String::new(),
                message: String::new(),
            },
            ActionDecision::Ready { preview } => Self {
                stage: ActionStage::Ready,
                title: preview.summary,
                details: preview.details,
                phrase: String::new(),
                input: String::new(),
                message: String::new(),
            },
        }
    }
}

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
    Crons,
    Databases,
    Security,
}

impl Tab {
    /// All tabs, in display order.
    pub const ALL: [Tab; 10] = [
        Tab::Dashboard,
        Tab::System,
        Tab::Processes,
        Tab::Services,
        Tab::Logs,
        Tab::Network,
        Tab::Docker,
        Tab::Crons,
        Tab::Databases,
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
            Tab::Crons => "Crons",
            Tab::Databases => "Databases",
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
            Tab::Crons => ModuleId::Crons,
            Tab::Databases => ModuleId::Databases,
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

/// Which subset of systemd units the Services screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceFilter {
    All,
    Failed,
    Running,
    Inactive,
    Enabled,
}

impl ServiceFilter {
    /// All filters, in display (cycle) order.
    pub const ALL: [ServiceFilter; 5] = [
        ServiceFilter::All,
        ServiceFilter::Failed,
        ServiceFilter::Running,
        ServiceFilter::Inactive,
        ServiceFilter::Enabled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ServiceFilter::All => "ALL",
            ServiceFilter::Failed => "FAILED",
            ServiceFilter::Running => "RUNNING",
            ServiceFilter::Inactive => "INACTIVE",
            ServiceFilter::Enabled => "ENABLED",
        }
    }
}

/// One reachability probe result shown in the Network → Connectivity panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectivityResult {
    /// The probed host/address.
    pub target: String,
    /// Where the target came from (`gateway`, `dns`).
    pub label: String,
    pub reachable: bool,
    /// Human summary, e.g. `0.4ms avg · 0% loss` or `no reply`.
    pub detail: String,
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
    /// What the connected user can do on the host (probed once at startup).
    pub capabilities: Option<HostCapabilities>,
    pub theme: Theme,
    pub active_tab: usize,
    pub view_state: ViewState,
    pub snapshot: Option<SystemSnapshot>,
    pub processes: Vec<Process>,
    pub process_sort: ProcessSort,
    pub failed_units: Vec<ServiceUnit>,
    /// Full service unit list (slow-tier), backing the Services screen filters.
    pub all_units: Vec<ServiceUnit>,
    /// Names of units enabled at boot, from `list-unit-files`.
    pub enabled_units: Vec<String>,
    pub service_filter: ServiceFilter,
    pub network: Option<NetworkSnapshot>,
    pub exposures: Vec<ExposureEntry>,
    /// On-demand reachability probes (Network tab, run on `c`).
    pub connectivity: Vec<ConnectivityResult>,
    /// A connectivity probe run was requested; the loop spawns it in background.
    pub connectivity_requested: bool,
    /// A background connectivity run is in flight (drives the indicator and
    /// coalesces requests).
    pub connectivity_running: bool,
    pub firewall: FirewallSnapshot,
    pub findings: Vec<Finding>,
    pub databases: DatabaseSnapshot,
    pub cert_warning_days: u32,
    pub containers: Vec<Container>,
    pub container_inspects: Vec<InspectSummary>,
    pub container_stats: Vec<ContainerStats>,
    pub docker_available: bool,
    pub compose_projects: Vec<ComposeProject>,
    pub image_hygiene: ImageHygiene,
    pub containers_selected: usize,
    pub crons: Vec<CronEntry>,
    pub timers: Vec<SystemdTimer>,
    pub packages: PackageUpdates,
    pub crons_selected: usize,
    pub databases_selected: usize,
    /// Reference time for cron next-run previews, refreshed each collection.
    pub now: NaiveDateTime,
    pub logs: Vec<LogEntry>,
    pub log_query: LogQuery,
    pub log_search: String,
    pub input_mode: InputMode,
    pub health: Option<HealthReport>,
    pub thresholds: Thresholds,
    pub services_selected: usize,
    pub processes_selected: usize,
    pub action: Option<ActionModal>,
    pub pending: Option<PendingAction>,
    pub cron_form: Option<CronFormState>,
    pub show_help: bool,
    pub should_quit: bool,
    /// A background gather is in flight; drives the refresh indicator and
    /// coalesces refresh requests so only one gather runs at a time.
    pub refreshing: bool,
    pub refresh_requested: bool,
    pub logs_reload_requested: bool,
    pub action_plan_requested: bool,
    pub action_exec_requested: bool,
    /// Recent CPU busy% samples for the dashboard sparkline (oldest first).
    pub cpu_history: Vec<u64>,
    /// Recent RAM used% samples for the dashboard sparkline (oldest first).
    pub mem_history: Vec<u64>,
    /// Persisted local state (health/finding snapshots, notes, saved searches).
    pub state: PersistentState,
    /// `true` when [`state`] changed and should be flushed to disk.
    pub state_dirty: bool,
    /// In-progress session note being typed (Dashboard); `None` when not entering.
    pub note_draft: Option<String>,
    /// Selected saved search on the Logs tab.
    pub saved_search_selected: usize,
}

/// How many samples the dashboard sparklines retain.
pub const HISTORY_LEN: usize = 48;

impl App {
    /// Create the initial application state for a host.
    pub fn new(host_label: impl Into<String>, mode: ExecutionMode) -> Self {
        Self {
            host_label: host_label.into(),
            mode,
            capabilities: None,
            theme: Theme::dark(),
            active_tab: 0,
            view_state: ViewState::Empty,
            snapshot: None,
            processes: Vec::new(),
            process_sort: ProcessSort::Cpu,
            failed_units: Vec::new(),
            all_units: Vec::new(),
            enabled_units: Vec::new(),
            service_filter: ServiceFilter::All,
            network: None,
            exposures: Vec::new(),
            connectivity: Vec::new(),
            connectivity_requested: false,
            connectivity_running: false,
            firewall: FirewallSnapshot::default(),
            findings: Vec::new(),
            databases: DatabaseSnapshot::default(),
            cert_warning_days: 30,
            containers: Vec::new(),
            container_inspects: Vec::new(),
            container_stats: Vec::new(),
            docker_available: false,
            compose_projects: Vec::new(),
            image_hygiene: ImageHygiene::default(),
            containers_selected: 0,
            crons: Vec::new(),
            timers: Vec::new(),
            packages: PackageUpdates::default(),
            crons_selected: 0,
            databases_selected: 0,
            now: Local::now().naive_local(),
            logs: Vec::new(),
            log_query: LogQuery::default(),
            log_search: String::new(),
            input_mode: InputMode::Normal,
            health: None,
            thresholds: Thresholds::default(),
            services_selected: 0,
            processes_selected: 0,
            action: None,
            pending: None,
            cron_form: None,
            show_help: false,
            should_quit: false,
            refreshing: false,
            refresh_requested: false,
            logs_reload_requested: false,
            action_plan_requested: false,
            action_exec_requested: false,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            state: PersistentState::default(),
            state_dirty: false,
            note_draft: None,
            saved_search_selected: 0,
        }
    }

    /// Append the latest CPU/RAM utilisation to the sparkline history rings.
    pub fn push_history(&mut self, cpu_percent: f64, mem_percent: f64) {
        for (ring, value) in [
            (&mut self.cpu_history, cpu_percent),
            (&mut self.mem_history, mem_percent),
        ] {
            ring.push(value.clamp(0.0, 100.0).round() as u64);
            if ring.len() > HISTORY_LEN {
                let overflow = ring.len() - HISTORY_LEN;
                ring.drain(0..overflow);
            }
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

    // --- Session notes (Dashboard) ----------------------------------------

    /// Begin typing a session note for the current host.
    pub fn open_note(&mut self) {
        self.note_draft = Some(String::new());
    }

    pub fn note_push_char(&mut self, c: char) {
        if let Some(draft) = &mut self.note_draft {
            draft.push(c);
        }
    }

    pub fn note_pop_char(&mut self) {
        if let Some(draft) = &mut self.note_draft {
            draft.pop();
        }
    }

    pub fn cancel_note(&mut self) {
        self.note_draft = None;
    }

    /// Save the in-progress note to the persistent store (no-op if empty).
    pub fn submit_note(&mut self) {
        let Some(draft) = self.note_draft.take() else {
            return;
        };
        let text = draft.trim();
        if text.is_empty() {
            return;
        }
        let at = self.now.format("%Y-%m-%d %H:%M").to_string();
        self.state.add_note(&self.host_label, &at, text);
        self.state_dirty = true;
    }

    // --- Saved log searches (Logs) ----------------------------------------

    /// Persist the current log search query as a saved search.
    pub fn save_current_search(&mut self) {
        if self.log_search.trim().is_empty() {
            return;
        }
        self.state.add_search(&self.log_search);
        self.state_dirty = true;
        self.saved_search_selected = 0;
    }

    /// Apply the selected saved search to the log filter and reload.
    pub fn apply_saved_search(&mut self) {
        if let Some(search) = self.state.saved_searches.get(self.saved_search_selected) {
            self.log_search = search.query.clone();
            self.logs_reload_requested = true;
        }
    }

    /// Move the saved-search selection (Logs tab).
    pub fn saved_search_down(&mut self) {
        let len = self.state.saved_searches.len();
        if len > 0 && self.saved_search_selected + 1 < len {
            self.saved_search_selected += 1;
        }
    }

    pub fn saved_search_up(&mut self) {
        self.saved_search_selected = self.saved_search_selected.saturating_sub(1);
    }

    /// Record a health/finding snapshot for today into the persistent store
    /// (deduped per day). Called after each refresh.
    pub fn record_health_snapshot(&mut self) {
        let Some(health) = &self.health else {
            return;
        };
        let [crit, high, med, ..] = self.finding_counts();
        let date = self.now.format("%Y-%m-%d").to_string();
        self.state
            .record_snapshot(&self.host_label, &date, health.score, crit, high, med);
        self.state_dirty = true;
    }

    /// Toggle the help overlay.
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Ask the event loop to re-run collectors on its next tick.
    pub fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Ask the loop to run connectivity probes in the background.
    pub fn request_connectivity(&mut self) {
        self.connectivity_requested = true;
    }

    /// Targets to probe: the default gateway and each configured DNS server,
    /// derived from the current network snapshot. De-duplicated, in a stable
    /// order. Empty when there is no network data yet. Deliberately limited to
    /// the host's own gateway/resolvers — never arbitrary external hosts.
    pub fn connectivity_targets(&self) -> Vec<(String, String)> {
        let mut targets: Vec<(String, String)> = Vec::new();
        let mut push = |addr: &str, label: &str| {
            if !addr.is_empty() && !targets.iter().any(|(t, _)| t == addr) {
                targets.push((addr.to_owned(), label.to_owned()));
            }
        };
        if let Some(net) = &self.network {
            for route in &net.routes {
                if route.dst == "default"
                    && let Some(gw) = &route.gateway
                {
                    push(gw, "gateway");
                }
            }
            for ns in &net.dns.nameservers {
                push(ns, "dns");
            }
        }
        targets
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

    /// Units shown on the Services screen under the active filter. Falls back to
    /// the live `failed_units` when the full list has not been gathered yet (so
    /// the screen is never blank on the first frames), and always for the FAILED
    /// filter (the freshest signal).
    pub fn visible_units(&self) -> Vec<&ServiceUnit> {
        if self.service_filter == ServiceFilter::Failed || self.all_units.is_empty() {
            return self.failed_units.iter().collect();
        }
        self.all_units
            .iter()
            .filter(|u| match self.service_filter {
                ServiceFilter::All => true,
                ServiceFilter::Failed => u.is_failed(),
                ServiceFilter::Running => u.sub == "running",
                ServiceFilter::Inactive => u.active == "inactive",
                ServiceFilter::Enabled => self.enabled_units.iter().any(|n| n == &u.name),
            })
            .collect()
    }

    /// The unit currently selected on the Services screen, under the active filter.
    pub fn selected_service(&self) -> Option<ServiceUnit> {
        self.visible_units()
            .get(self.services_selected)
            .map(|u| (*u).clone())
    }

    /// Cycle the Services screen filter and reset the selection to the top.
    pub fn cycle_service_filter(&mut self) {
        let current = ServiceFilter::ALL
            .iter()
            .position(|f| *f == self.service_filter)
            .unwrap_or(0);
        self.service_filter = ServiceFilter::ALL[(current + 1) % ServiceFilter::ALL.len()];
        self.services_selected = 0;
    }

    /// Processes in display order (sorted by the current key).
    pub fn visible_processes(&self) -> Vec<&Process> {
        let mut procs: Vec<&Process> = self.processes.iter().collect();
        let key = |p: &Process| match self.process_sort {
            ProcessSort::Cpu => p.cpu_percent,
            ProcessSort::Mem => p.mem_percent,
        };
        procs.sort_by(|a, b| {
            key(b)
                .partial_cmp(&key(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        procs
    }

    /// Count findings at each severity, as `[critical, high, medium, low, info]`.
    pub fn finding_counts(&self) -> [usize; 5] {
        let mut counts = [0usize; 5];
        for finding in &self.findings {
            let idx = match finding.severity {
                Severity::Critical => 0,
                Severity::High => 1,
                Severity::Medium => 2,
                Severity::Low => 3,
                Severity::Info => 4,
            };
            counts[idx] += 1;
        }
        counts
    }

    /// Number of externally reachable, sensitive exposures (High/Critical).
    pub fn risky_exposure_count(&self) -> usize {
        self.exposures
            .iter()
            .filter(|e| e.severity >= Severity::High)
            .count()
    }

    fn selection_len(&self) -> usize {
        match self.current_tab() {
            Tab::Services => self.visible_units().len(),
            Tab::Processes => self.processes.len(),
            Tab::Docker => self.containers.len(),
            Tab::Crons => self.crons.len(),
            Tab::Databases => self.databases.instances.len(),
            _ => 0,
        }
    }

    fn selected_mut(&mut self) -> Option<&mut usize> {
        match self.current_tab() {
            Tab::Services => Some(&mut self.services_selected),
            Tab::Processes => Some(&mut self.processes_selected),
            Tab::Docker => Some(&mut self.containers_selected),
            Tab::Crons => Some(&mut self.crons_selected),
            Tab::Databases => Some(&mut self.databases_selected),
            _ => None,
        }
    }

    /// The container currently selected on the Docker tab, if any.
    pub fn selected_container(&self) -> Option<&Container> {
        self.containers.get(self.containers_selected)
    }

    /// The inspect summary for the selected container, matched by id.
    pub fn selected_inspect(&self) -> Option<&InspectSummary> {
        let container = self.selected_container()?;
        self.container_inspects
            .iter()
            .find(|i| i.id == container.id)
    }

    /// The database instance currently selected on the Databases tab, if any.
    pub fn selected_database(&self) -> Option<&systui_collectors::DatabaseInstance> {
        self.databases.instances.get(self.databases_selected)
    }

    /// The cron entry currently selected on the Crons tab, if any.
    pub fn selected_cron(&self) -> Option<&CronEntry> {
        self.crons.get(self.crons_selected)
    }

    /// Move the selection down within the active list.
    pub fn select_down(&mut self) {
        let len = self.selection_len();
        if let Some(sel) = self.selected_mut() {
            if len > 0 && *sel + 1 < len {
                *sel += 1;
            }
        }
    }

    /// Move the selection up within the active list.
    pub fn select_up(&mut self) {
        if let Some(sel) = self.selected_mut() {
            *sel = sel.saturating_sub(1);
        }
    }

    /// Clamp selections after the underlying lists change.
    pub fn clamp_selections(&mut self) {
        self.services_selected = self
            .services_selected
            .min(self.visible_units().len().saturating_sub(1));
        self.processes_selected = self
            .processes_selected
            .min(self.processes.len().saturating_sub(1));
        self.containers_selected = self
            .containers_selected
            .min(self.containers.len().saturating_sub(1));
        self.crons_selected = self.crons_selected.min(self.crons.len().saturating_sub(1));
        self.databases_selected = self
            .databases_selected
            .min(self.databases.instances.len().saturating_sub(1));
    }

    /// Build the default action for the selected item and request planning.
    pub fn request_action(&mut self) {
        let pending = match self.current_tab() {
            Tab::Services => self
                .selected_service()
                .map(|u| PendingAction::Service(ServiceAction::new(ServiceOp::Restart, &u.name))),
            Tab::Processes => {
                let procs = self.visible_processes();
                procs.get(self.processes_selected).map(|p| {
                    PendingAction::Signal(SignalAction::new(Signal::Term, p.pid, &p.command))
                })
            }
            // Default container op is state-aware: restart a running container,
            // start a stopped one. Stop/remove remain available via the engine.
            Tab::Docker => self.selected_container().map(|c| {
                let op = if c.is_running() {
                    DockerOp::Restart
                } else {
                    DockerOp::Start
                };
                PendingAction::Docker(DockerAction::new(op, &c.name))
            }),
            _ => None,
        };
        if let Some(pending) = pending {
            self.pending = Some(pending);
            self.action_plan_requested = true;
        }
    }

    /// Queue a "prune dangling images" mutation for the engine (Docker tab).
    pub fn request_prune_images(&mut self) {
        if self.mode == ExecutionMode::ReadOnly {
            self.set_decision(ActionDecision::Rejected(
                "image prune is disabled in read-only mode".to_owned(),
            ));
            return;
        }
        self.pending = Some(PendingAction::DockerPrune(DockerPruneAction::new()));
        self.action_plan_requested = true;
    }

    /// Open a form for adding a user-crontab entry.
    pub fn open_add_cron_form(&mut self) {
        if self.mode == ExecutionMode::ReadOnly {
            self.set_decision(ActionDecision::Rejected(
                "cron management is disabled in read-only mode".to_owned(),
            ));
            return;
        }
        self.cron_form = Some(CronFormState {
            mode: CronFormMode::Add,
            form: cron_form("Add cron job", "", ""),
        });
    }

    /// Open a form for editing the selected user-crontab entry.
    pub fn open_edit_cron_form(&mut self) {
        if self.mode == ExecutionMode::ReadOnly {
            self.set_decision(ActionDecision::Rejected(
                "cron management is disabled in read-only mode".to_owned(),
            ));
            return;
        }
        let Some(entry) = self.selected_user_cron().cloned() else {
            self.set_decision(ActionDecision::Rejected(
                "select a user crontab entry to edit".to_owned(),
            ));
            return;
        };
        self.cron_form = Some(CronFormState {
            mode: CronFormMode::Edit {
                original_schedule: entry.schedule.clone(),
                original_command: entry.command.clone(),
            },
            form: cron_form("Edit cron job", &entry.schedule, &entry.command),
        });
    }

    /// Delete the selected user-crontab entry through the action engine.
    pub fn request_delete_cron(&mut self) {
        if let Some(action) = self
            .selected_user_cron()
            .map(|entry| CronAction::delete(entry.schedule.clone(), entry.command.clone()))
        {
            self.pending = Some(PendingAction::Cron(action));
            self.action_plan_requested = true;
        } else {
            self.set_decision(ActionDecision::Rejected(
                "select a user crontab entry to delete".to_owned(),
            ));
        }
    }

    /// Run the selected user-crontab entry's command immediately, via the engine.
    pub fn request_run_cron(&mut self) {
        if let Some(action) = self
            .selected_user_cron()
            .map(|entry| CronAction::run_now(entry.schedule.clone(), entry.command.clone()))
        {
            self.pending = Some(PendingAction::Cron(action));
            self.action_plan_requested = true;
        } else {
            self.set_decision(ActionDecision::Rejected(
                "select a user crontab entry to run".to_owned(),
            ));
        }
    }

    /// Toggle the selected user-crontab entry between active and commented out.
    pub fn request_toggle_cron(&mut self) {
        if let Some(action) = self.selected_user_cron().map(|entry| {
            if entry.enabled {
                CronAction::disable(entry.schedule.clone(), entry.command.clone())
            } else {
                CronAction::enable(entry.schedule.clone(), entry.command.clone())
            }
        }) {
            self.pending = Some(PendingAction::Cron(action));
            self.action_plan_requested = true;
        } else {
            self.set_decision(ActionDecision::Rejected(
                "select a user crontab entry to toggle".to_owned(),
            ));
        }
    }

    /// Submit the open cron form and route the resulting mutation through the
    /// action engine.
    pub fn submit_cron_form(&mut self) {
        let Some(mut state) = self.cron_form.take() else {
            return;
        };
        let schedule = state.form.value("Schedule").to_owned();
        let command = state.form.value("Command").to_owned();
        if schedule.is_empty() || command.is_empty() {
            state.form.error = Some("schedule and command are required".to_owned());
            self.cron_form = Some(state);
            return;
        }
        if let Err(err) = parse_schedule(&schedule) {
            state.form.error = Some(format!("invalid schedule: {err}"));
            self.cron_form = Some(state);
            return;
        }

        let action = match state.mode {
            CronFormMode::Add => CronAction::add(schedule, command),
            CronFormMode::Edit {
                original_schedule,
                original_command,
            } => CronAction::edit(original_schedule, original_command, schedule, command),
        };
        self.pending = Some(PendingAction::Cron(action));
        self.action_plan_requested = true;
    }

    pub fn close_cron_form(&mut self) {
        self.cron_form = None;
    }

    pub fn cron_form_focus_next(&mut self) {
        if let Some(state) = &mut self.cron_form {
            state.form.focus_next();
        }
    }

    pub fn cron_form_focus_prev(&mut self) {
        if let Some(state) = &mut self.cron_form {
            state.form.focus_prev();
        }
    }

    pub fn cron_form_push_char(&mut self, c: char) {
        if let Some(state) = &mut self.cron_form {
            state.form.error = None;
            state.form.push_char(c);
        }
    }

    pub fn cron_form_pop_char(&mut self) {
        if let Some(state) = &mut self.cron_form {
            state.form.error = None;
            state.form.pop_char();
        }
    }

    fn selected_user_cron(&self) -> Option<&CronEntry> {
        self.selected_cron()
            .filter(|entry| entry.source == CronSource::User)
    }

    /// Apply the engine's planning decision to the modal.
    pub fn set_decision(&mut self, decision: ActionDecision) {
        self.action = Some(ActionModal::from_decision(decision));
        if self.action.as_ref().map(|a| a.stage) == Some(ActionStage::Result) {
            self.pending = None;
        }
    }

    /// Show the action result/rejection message.
    pub fn set_action_result(&mut self, message: String) {
        if let Some(modal) = &mut self.action {
            modal.stage = ActionStage::Result;
            modal.message = message;
        }
        self.pending = None;
    }

    /// Confirm the current modal (Enter): request execution if appropriate.
    pub fn submit_action(&mut self) {
        if let Some(modal) = &self.action {
            match modal.stage {
                ActionStage::Confirm | ActionStage::Ready => {
                    self.action_exec_requested = true;
                }
                ActionStage::Result => self.close_action(),
            }
        }
    }

    /// Close the action overlay.
    pub fn close_action(&mut self) {
        self.action = None;
        self.pending = None;
    }

    /// Typed confirmation input (used in `Confirm` stage).
    pub fn push_action_char(&mut self, c: char) {
        if let Some(modal) = &mut self.action {
            modal.input.push(c);
        }
    }

    pub fn pop_action_char(&mut self) {
        if let Some(modal) = &mut self.action {
            modal.input.pop();
        }
    }

    /// Request application exit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

fn cron_form(title: &str, schedule: &str, command: &str) -> Form {
    Form::new(
        title,
        vec![
            Field::text("Schedule", schedule).with_hint("five fields or @daily"),
            Field::text("Command", command),
        ],
    )
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

    #[test]
    fn request_action_on_service_builds_pending_restart() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.select_tab(3); // Services
        app.failed_units = vec![ServiceUnit {
            name: "nginx.service".to_owned(),
            load: "loaded".to_owned(),
            active: "failed".to_owned(),
            sub: "failed".to_owned(),
            description: "web".to_owned(),
        }];

        app.request_action();
        assert!(app.action_plan_requested);
        match app.pending {
            Some(PendingAction::Service(ref a)) => {
                assert_eq!(a.unit, "nginx.service");
                assert_eq!(a.op, ServiceOp::Restart);
            }
            other => panic!("expected a service action, got {other:?}"),
        }
    }

    #[test]
    fn connectivity_targets_from_gateway_and_dns() {
        use systui_collectors::{DnsConfig, NetworkSnapshot, Route};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        assert!(app.connectivity_targets().is_empty()); // no network yet
        app.network = Some(NetworkSnapshot {
            interfaces: Vec::new(),
            routes: vec![Route {
                dst: "default".to_owned(),
                gateway: Some("10.0.0.1".to_owned()),
                dev: "eth0".to_owned(),
                prefsrc: None,
            }],
            dns: DnsConfig {
                nameservers: vec!["1.1.1.1".to_owned(), "10.0.0.1".to_owned()],
                search: Vec::new(),
            },
            listeners: Vec::new(),
            connections: Vec::new(),
        });
        let targets = app.connectivity_targets();
        // gateway first, then DNS; the duplicate 10.0.0.1 is de-duplicated.
        assert_eq!(
            targets,
            vec![
                ("10.0.0.1".to_owned(), "gateway".to_owned()),
                ("1.1.1.1".to_owned(), "dns".to_owned()),
            ]
        );
    }

    #[test]
    fn request_action_on_docker_is_state_aware() {
        use systui_collectors::Container;
        let container = |name: &str, state: &str| Container {
            id: name.to_owned(),
            name: name.to_owned(),
            image: "img".to_owned(),
            state: state.to_owned(),
            status: String::new(),
            health: None,
            ports: String::new(),
            created: String::new(),
        };
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.select_tab(6); // Docker
        app.containers = vec![container("web", "running"), container("db", "exited")];

        app.request_action();
        match app.pending {
            Some(PendingAction::Docker(ref a)) => {
                assert_eq!(a.container, "web");
                assert_eq!(a.op, DockerOp::Restart); // running → restart
            }
            other => panic!("expected a docker action, got {other:?}"),
        }

        app.containers_selected = 1;
        app.request_action();
        match app.pending {
            Some(PendingAction::Docker(ref a)) => {
                assert_eq!(a.container, "db");
                assert_eq!(a.op, DockerOp::Start); // stopped → start
            }
            other => panic!("expected a docker action, got {other:?}"),
        }
    }

    #[test]
    fn confirm_decision_opens_modal_and_typing_works() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.set_decision(ActionDecision::NeedsConfirmation {
            preview: systui_core::ActionPreview {
                summary: "Restart nginx.service".to_owned(),
                details: vec!["Restarts the unit.".to_owned()],
                command: None,
                reversible: false,
                creates_backup: false,
            },
            phrase: "Restart nginx.service".to_owned(),
        });

        let modal = app.action.as_ref().unwrap();
        assert_eq!(modal.stage, ActionStage::Confirm);

        app.push_action_char('r');
        app.push_action_char('x');
        app.pop_action_char();
        assert_eq!(app.action.as_ref().unwrap().input, "r");

        app.submit_action();
        assert!(app.action_exec_requested);
    }

    #[test]
    fn rejected_decision_is_terminal() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.pending = None;
        app.set_decision(ActionDecision::Rejected("not allowed".to_owned()));
        assert_eq!(app.action.as_ref().unwrap().stage, ActionStage::Result);
    }
}
