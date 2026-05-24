//! The fleet overview TUI: a selectable, worst-first table of inventory hosts
//! (`Product.md` §4.16). It lets the operator **manage the inventory** (add / edit
//! / delete hosts, persisted to `config.toml`) and **drill into** a host. It never
//! executes remote actions; gathering and the per-host drill-in are driven by the
//! caller, which owns the transports and the async runtime.

use std::path::Path;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table};
use ratatui::{DefaultTerminal, Frame};
use systui_core::Result;
use systui_core::config::{Config, Host};
use systui_report::{FleetOutcome, FleetOverview, findings_summary};

use crate::Theme;
use crate::form::{Field, Form, render_form};

/// How often the event loop wakes to poll for input.
const TICK: Duration = Duration::from_millis(250);

/// What the operator chose from the fleet overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetExit {
    /// Leave fleet mode.
    Quit,
    /// Drill into the host with this inventory id.
    Enter(String),
}

/// What an open form will produce on submit.
enum FormTarget {
    /// Add a brand-new inventory host.
    Add,
    /// Edit the host with this id (id is fixed).
    Edit(String),
}

/// A modal overlaying the host table.
enum Overlay {
    None,
    Form { form: Form, target: FormTarget },
    Confirm { id: String },
    Notice(String),
}

/// Mutable state of the fleet view.
struct FleetView {
    overview: FleetOverview,
    selected: usize,
    overlay: Overlay,
    read_only: bool,
}

/// Run the fleet overview TUI.
///
/// `gather` re-collects the overview from the (possibly edited) config — the caller
/// supplies it because it owns the transports and runtime. Inventory edits are
/// persisted to `config_path` and mirrored into `config`; in read-only mode the
/// management keys are disabled. Returns [`FleetExit::Enter`] for drill-in (the
/// caller launches the per-host TUI, then re-enters) or [`FleetExit::Quit`].
pub fn run_fleet(
    config: &mut Config,
    config_path: &Path,
    read_only: bool,
    gather: impl FnMut(&Config) -> FleetOverview,
) -> Result<FleetExit> {
    let mut terminal = ratatui::try_init()?;
    let result = fleet_loop(&mut terminal, config, config_path, read_only, gather);
    let _ = ratatui::try_restore();
    result
}

fn fleet_loop(
    terminal: &mut DefaultTerminal,
    config: &mut Config,
    config_path: &Path,
    read_only: bool,
    mut gather: impl FnMut(&Config) -> FleetOverview,
) -> Result<FleetExit> {
    let theme = Theme::dark();
    let mut view = FleetView {
        overview: gather(config),
        selected: 0,
        overlay: Overlay::None,
        read_only,
    };

    loop {
        terminal.draw(|frame| render(frame, &view, &theme))?;

        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if matches!(view.overlay, Overlay::None) {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(FleetExit::Quit);
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(FleetExit::Quit),
                KeyCode::Enter => {
                    if let Some(host) = view.overview.hosts.get(view.selected) {
                        return Ok(FleetExit::Enter(host.id.clone()));
                    }
                }
                KeyCode::Char('r') => {
                    view.overview = gather(config);
                    clamp(&mut view);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    view.selected = next_index(view.selected, view.overview.hosts.len());
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    view.selected = prev_index(view.selected, view.overview.hosts.len());
                }
                KeyCode::Char('a') => open_add(&mut view),
                KeyCode::Char('e') => open_edit(&mut view, config),
                KeyCode::Char('d') => open_delete(&mut view),
                _ => {}
            }
        } else {
            handle_overlay(&mut view, key, config, config_path, &mut gather);
        }
    }
}

/// Keep the selection within the (possibly shrunk) host list.
fn clamp(view: &mut FleetView) {
    let len = view.overview.hosts.len();
    if len == 0 {
        view.selected = 0;
    } else if view.selected >= len {
        view.selected = len - 1;
    }
}

fn open_add(view: &mut FleetView) {
    if view.read_only {
        view.overlay = read_only_notice();
        return;
    }
    view.overlay = Overlay::Form {
        form: add_host_form(),
        target: FormTarget::Add,
    };
}

fn open_edit(view: &mut FleetView, config: &Config) {
    if view.read_only {
        view.overlay = read_only_notice();
        return;
    }
    let Some(summary) = view.overview.hosts.get(view.selected) else {
        return;
    };
    let Some(host) = config.hosts.get(&summary.id) else {
        view.overlay = Overlay::Notice(format!("{} is not an editable inventory host", summary.id));
        return;
    };
    view.overlay = Overlay::Form {
        form: edit_host_form(host),
        target: FormTarget::Edit(summary.id.clone()),
    };
}

fn open_delete(view: &mut FleetView) {
    if view.read_only {
        view.overlay = read_only_notice();
        return;
    }
    if let Some(summary) = view.overview.hosts.get(view.selected) {
        view.overlay = Overlay::Confirm {
            id: summary.id.clone(),
        };
    }
}

fn read_only_notice() -> Overlay {
    Overlay::Notice("Read-only mode: inventory editing is disabled.".to_owned())
}

/// Handle a key while a modal is open. Takes ownership of the overlay so the
/// borrow checker is happy mutating `view`, `config` and calling `gather`.
fn handle_overlay(
    view: &mut FleetView,
    key: KeyEvent,
    config: &mut Config,
    config_path: &Path,
    gather: &mut impl FnMut(&Config) -> FleetOverview,
) {
    let overlay = std::mem::replace(&mut view.overlay, Overlay::None);
    match overlay {
        Overlay::None => {}
        Overlay::Notice(_) => {} // any key dismisses (overlay already cleared)
        Overlay::Confirm { id } => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                if let Err(err) = systui_storage::remove_host_from(config_path, &id) {
                    view.overlay = Overlay::Notice(format!("Could not save: {err}"));
                } else {
                    config.remove_host(&id);
                    view.overview = gather(config);
                    clamp(view);
                }
            }
            _ => {} // n / Esc / anything else cancels
        },
        Overlay::Form { mut form, target } => {
            match key.code {
                KeyCode::Esc => {} // cancel (overlay cleared)
                KeyCode::Enter => match build_host(&form, &target, config) {
                    Ok((id, host)) => match systui_storage::save_host_to(config_path, &id, &host) {
                        Ok(()) => {
                            config.upsert_host(id, host);
                            view.overview = gather(config);
                            clamp(view);
                        }
                        Err(err) => {
                            form.error = Some(format!("Could not save: {err}"));
                            view.overlay = Overlay::Form { form, target };
                        }
                    },
                    Err(err) => {
                        form.error = Some(err);
                        view.overlay = Overlay::Form { form, target };
                    }
                },
                KeyCode::Tab | KeyCode::Down => {
                    form.focus_next();
                    view.overlay = Overlay::Form { form, target };
                }
                KeyCode::Up | KeyCode::BackTab => {
                    form.focus_prev();
                    view.overlay = Overlay::Form { form, target };
                }
                KeyCode::Backspace => {
                    form.pop_char();
                    view.overlay = Overlay::Form { form, target };
                }
                KeyCode::Char(' ') if form.fields.get(form.focused).is_some_and(is_bool_field) => {
                    form.toggle();
                    view.overlay = Overlay::Form { form, target };
                }
                KeyCode::Char(c) => {
                    form.push_char(c);
                    view.overlay = Overlay::Form { form, target };
                }
                _ => {
                    view.overlay = Overlay::Form { form, target };
                }
            }
        }
    }
}

fn is_bool_field(field: &Field) -> bool {
    field.kind == crate::form::FieldKind::Bool
}

fn add_host_form() -> Form {
    Form::new(
        "Add host",
        vec![
            Field::text("id", "").with_hint("inventory key"),
            Field::text("host", "").with_hint("hostname or IP"),
            Field::text("user", ""),
            Field::text("port", "22"),
            Field::text("tags", "").with_hint("comma-separated"),
            Field::boolean("read_only", false),
            Field::boolean("favorite", false),
        ],
    )
}

fn edit_host_form(host: &Host) -> Form {
    Form::new(
        "Edit host",
        vec![
            Field::text("host", host.host.clone()).with_hint("hostname or IP"),
            Field::text("user", host.user.clone().unwrap_or_default()),
            Field::text("port", host.port.to_string()),
            Field::text("tags", host.tags.join(", ")).with_hint("comma-separated"),
            Field::boolean("read_only", host.read_only),
            Field::boolean("favorite", host.favorite),
        ],
    )
}

/// Validate a form into an inventory id and [`Host`]. On edit, the id is fixed and
/// the existing host's policy is preserved (the form does not edit policies).
fn build_host(
    form: &Form,
    target: &FormTarget,
    config: &Config,
) -> std::result::Result<(String, Host), String> {
    let id = match target {
        FormTarget::Add => {
            let id = form.value("id");
            if id.is_empty() {
                return Err("id is required".to_owned());
            }
            if id.contains(char::is_whitespace) {
                return Err("id cannot contain spaces".to_owned());
            }
            if config.hosts.contains_key(id) {
                return Err(format!("host `{id}` already exists"));
            }
            id.to_owned()
        }
        FormTarget::Edit(id) => id.clone(),
    };

    let host_addr = form.value("host");
    if host_addr.is_empty() {
        return Err("host is required".to_owned());
    }

    let port = match form.value("port") {
        "" => 22,
        p => p
            .parse::<u16>()
            .map_err(|_| format!("invalid port `{p}`"))?,
    };

    let user = match form.value("user") {
        "" => None,
        u => Some(u.to_owned()),
    };

    let tags = form
        .value("tags")
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();

    let policy = match target {
        FormTarget::Edit(id) => config.hosts.get(id).and_then(|h| h.policy.clone()),
        FormTarget::Add => None,
    };

    Ok((
        id,
        Host {
            host: host_addr.to_owned(),
            user,
            port,
            tags,
            read_only: form.flag("read_only"),
            favorite: form.flag("favorite"),
            policy,
        },
    ))
}

/// Next selection index, wrapping around. Stays at 0 for an empty list.
fn next_index(current: usize, len: usize) -> usize {
    if len == 0 { 0 } else { (current + 1) % len }
}

/// Previous selection index, wrapping around. Stays at 0 for an empty list.
fn prev_index(current: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (current + len - 1) % len
    }
}

/// Draw the fleet view: the base table plus any open modal.
fn render(frame: &mut Frame, view: &FleetView, theme: &Theme) {
    render_fleet(frame, &view.overview, theme, view.selected);
    match &view.overlay {
        Overlay::None => {}
        Overlay::Form { form, .. } => render_form(frame, form, theme),
        Overlay::Confirm { id } => render_message(
            frame,
            theme,
            theme.warn,
            "Delete host",
            &format!("Delete `{id}` from the inventory?  [y] yes  ·  [n] no"),
        ),
        Overlay::Notice(msg) => render_message(
            frame,
            theme,
            theme.border,
            "Notice",
            &format!("{msg}   (press any key)"),
        ),
    }
}

/// Render the fleet overview frame (base layer). A pure function of the overview
/// and the selected index, so it is testable headlessly with `TestBackend`.
pub fn render_fleet(frame: &mut Frame, overview: &FleetOverview, theme: &Theme, selected: usize) {
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(0),    // hosts table
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    render_title(frame, overview, theme, rows[0]);
    render_table(frame, overview, theme, selected, rows[1]);

    let footer = Paragraph::new(Line::from(Span::styled(
        " \u{2191}/\u{2193} select \u{00b7} Enter open \u{00b7} a add \u{00b7} e edit \u{00b7} d delete \u{00b7} r refresh \u{00b7} q quit ",
        Style::new().fg(theme.dim),
    )));
    frame.render_widget(footer, rows[2]);
}

fn render_title(frame: &mut Frame, overview: &FleetOverview, theme: &Theme, area: Rect) {
    let title = Line::from(vec![
        Span::styled(
            "SysTUI",
            Style::new().fg(theme.title).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" \u{2014} fleet"),
    ]);
    frame.render_widget(Paragraph::new(title), area);

    let status = Line::from(format!(
        "{} reviewed \u{00b7} {} unreachable \u{00b7} {} ",
        overview.reviewed_count(),
        overview.failed_count(),
        overview.generated_at,
    ))
    .alignment(Alignment::Right);
    frame.render_widget(
        Paragraph::new(status).style(Style::new().fg(theme.dim)),
        area,
    );
}

fn render_table(
    frame: &mut Frame,
    overview: &FleetOverview,
    theme: &Theme,
    selected: usize,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.border))
        .title(" Hosts ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if overview.hosts.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No hosts in the inventory. Press `a` to add one.",
                Style::new().fg(theme.dim),
            )),
            inner,
        );
        return;
    }

    let header = Row::new(["", "HOST", "TAGS", "HEALTH", "FINDINGS / STATUS"])
        .style(Style::new().fg(theme.dim).add_modifier(Modifier::BOLD));
    let body = overview.hosts.iter().enumerate().map(|(i, host)| {
        let marker = if host.favorite { "*" } else { "" };
        let tags = if host.tags.is_empty() {
            "-".to_owned()
        } else {
            host.tags.join(",")
        };
        let (health, status, color) = match &host.outcome {
            FleetOutcome::Reviewed {
                health,
                finding_counts,
                ..
            } => (
                format!("{health}/100"),
                findings_summary(finding_counts),
                health_color(theme, *health),
            ),
            FleetOutcome::Failed { error } => {
                ("\u{2014}".to_owned(), truncate(error, 48), theme.danger)
            }
        };
        let row = Row::new([marker.to_owned(), host.id.clone(), tags, health, status]);
        if i == selected {
            row.style(
                Style::new()
                    .fg(theme.selected_fg)
                    .bg(theme.selected_bg)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            row.style(Style::new().fg(color))
        }
    });
    let widths = [
        Constraint::Length(2),
        Constraint::Length(18),
        Constraint::Length(20),
        Constraint::Length(9),
        Constraint::Min(20),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(theme.text));
    frame.render_widget(table, inner);
}

/// A small centered message box (used for delete-confirm and notices).
fn render_message(frame: &mut Frame, theme: &Theme, border: Color, title: &str, body: &str) {
    let area = frame.area();
    let width = area.width.saturating_mul(60) / 100;
    let height = 3u16.min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border))
        .title(format!(" {title} "));
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {body}"),
            Style::new().fg(theme.text),
        ))
        .block(block),
        rect,
    );
}

/// Color a health score: green when healthy, yellow when degraded, red when poor.
fn health_color(theme: &Theme, health: u8) -> Color {
    if health >= 85 {
        theme.ok
    } else if health >= 60 {
        theme.warn
    } else {
        theme.danger
    }
}

/// Truncate a string to `max` chars, appending an ellipsis when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let kept: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}\u{2026}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use systui_report::FleetHostSummary;

    fn overview() -> FleetOverview {
        FleetOverview::build(
            "2026-05-24 10:00:00",
            vec![
                FleetHostSummary::failed("db-01", vec!["db".to_owned()], false, "timed out"),
                {
                    let mut s =
                        FleetHostSummary::failed("prod-01", vec!["web".to_owned()], true, "x");
                    s.outcome = FleetOutcome::Reviewed {
                        health: 72,
                        finding_counts: [0, 2, 0, 0, 0],
                        mode: systui_core::ExecutionMode::ReadOnly,
                        docker_available: true,
                    };
                    s
                },
            ],
        )
    }

    fn config_with_host() -> Config {
        let mut cfg = Config::default();
        cfg.upsert_host(
            "prod-01",
            Host {
                host: "10.0.0.1".to_owned(),
                user: Some("admin".to_owned()),
                port: 2222,
                tags: vec!["web".to_owned()],
                read_only: true,
                favorite: false,
                policy: Some("prod".to_owned()),
            },
        );
        cfg
    }

    #[test]
    fn index_navigation_wraps() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 0);
        assert_eq!(prev_index(0, 3), 2);
        assert_eq!(prev_index(1, 3), 0);
        assert_eq!(next_index(0, 0), 0);
        assert_eq!(prev_index(0, 0), 0);
    }

    #[test]
    fn truncate_caps_long_text() {
        assert_eq!(truncate("short", 48), "short");
        assert_eq!(truncate("abcdef", 4).chars().count(), 4);
    }

    #[test]
    fn build_host_validates_required_fields() {
        let cfg = Config::default();
        // Missing id.
        let mut form = add_host_form();
        assert!(build_host(&form, &FormTarget::Add, &cfg).is_err());
        // id with a space.
        form.fields[0].value = "bad id".to_owned();
        form.fields[1].value = "10.0.0.1".to_owned();
        assert!(build_host(&form, &FormTarget::Add, &cfg).is_err());
        // Bad port.
        form.fields[0].value = "ok".to_owned();
        form.fields[3].value = "99999".to_owned();
        assert!(build_host(&form, &FormTarget::Add, &cfg).is_err());
    }

    #[test]
    fn build_host_parses_a_valid_form() {
        let cfg = Config::default();
        let mut form = add_host_form();
        form.fields[0].value = "web-01".to_owned();
        form.fields[1].value = "192.168.1.5".to_owned();
        form.fields[2].value = "deploy".to_owned();
        form.fields[3].value = "2200".to_owned();
        form.fields[4].value = "web, prod".to_owned();
        form.fields[5].value = "true".to_owned(); // read_only
        let (id, host) = build_host(&form, &FormTarget::Add, &cfg).unwrap();
        assert_eq!(id, "web-01");
        assert_eq!(host.host, "192.168.1.5");
        assert_eq!(host.user.as_deref(), Some("deploy"));
        assert_eq!(host.port, 2200);
        assert_eq!(host.tags, ["web", "prod"]);
        assert!(host.read_only);
    }

    #[test]
    fn add_rejects_duplicate_id() {
        let cfg = config_with_host();
        let mut form = add_host_form();
        form.fields[0].value = "prod-01".to_owned();
        form.fields[1].value = "1.2.3.4".to_owned();
        assert!(build_host(&form, &FormTarget::Add, &cfg).is_err());
    }

    #[test]
    fn edit_preserves_policy_and_keeps_id() {
        let cfg = config_with_host();
        let host = cfg.hosts.get("prod-01").unwrap();
        let mut form = edit_host_form(host);
        // Change the address only.
        form.fields[0].value = "10.0.0.99".to_owned();
        let (id, built) = build_host(&form, &FormTarget::Edit("prod-01".to_owned()), &cfg).unwrap();
        assert_eq!(id, "prod-01");
        assert_eq!(built.host, "10.0.0.99");
        // Policy is preserved even though the form never showed it.
        assert_eq!(built.policy.as_deref(), Some("prod"));
    }

    #[test]
    fn renders_hosts_worst_first() {
        let backend = TestBackend::new(90, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let overview = overview();
        terminal
            .draw(|f| render_fleet(f, &overview, &Theme::dark(), 0))
            .unwrap();
        let rendered =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut acc, cell| {
                    acc.push_str(cell.symbol());
                    acc
                });
        assert!(rendered.contains("fleet"));
        assert!(rendered.contains("db-01"));
        assert!(rendered.contains("prod-01"));
        assert!(rendered.contains("72/100"));
        let db = rendered.find("db-01").unwrap();
        let prod = rendered.find("prod-01").unwrap();
        assert!(db < prod);
    }
}
