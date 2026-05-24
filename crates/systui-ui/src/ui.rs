//! Rendering of the application frame. This is a pure function of [`App`], which
//! makes it testable headlessly with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use regex::RegexBuilder;
use systui_collectors::{
    BindScope, Container, CronEntry, CronSource, DatabaseInstance, Disk, ExposureEntry, LogEntry,
    NetworkSnapshot, SystemSnapshot, parse_schedule,
};
use systui_core::{Finding, ModuleId, Severity};

use crate::app::{ActionStage, App, InputMode, Tab, ViewState};
use crate::form::render_form;

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(1), // tabs
        Constraint::Min(0),    // content
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    render_title(frame, app, rows[0]);
    render_tabs(frame, app, rows[1]);
    render_content(frame, app, rows[2]);
    render_footer(frame, app, rows[3]);

    if app.show_help {
        render_help(frame, app);
    }
    if app.action.is_some() {
        render_action_modal(frame, app);
    }
    if let Some(state) = &app.cron_form {
        render_form(frame, &state.form, &app.theme);
    }
}

fn render_action_modal(frame: &mut Frame, app: &App) {
    let Some(modal) = &app.action else {
        return;
    };
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let (border, footer) = match modal.stage {
        ActionStage::Confirm => (app.theme.warn, "Enter confirm · Esc cancel"),
        ActionStage::Ready => (app.theme.accent, "Enter run · Esc cancel"),
        ActionStage::Result => (app.theme.border, "press any key to close"),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(border))
        .title(format!(" {} ", modal.title));

    let mut lines = vec![Line::from("")];
    for detail in &modal.details {
        lines.push(Line::from(Span::styled(
            format!("  {detail}"),
            Style::new().fg(app.theme.dim),
        )));
    }
    match modal.stage {
        ActionStage::Confirm => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Type to confirm: ", Style::new().fg(app.theme.text)),
                Span::styled(
                    modal.phrase.clone(),
                    Style::new().fg(app.theme.warn).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  > {}_", modal.input),
                Style::new().fg(app.theme.accent),
            )));
        }
        ActionStage::Ready => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Ready to run.",
                Style::new().fg(app.theme.text),
            )));
        }
        ActionStage::Result => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", modal.message),
                Style::new().fg(app.theme.text),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  [{footer}]"),
        Style::new().fg(app.theme.dim),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_title(frame: &mut Frame, app: &App, area: Rect) {
    let title = Line::from(vec![
        Span::styled(
            "SysTUI",
            Style::new()
                .fg(app.theme.title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" — "),
        Span::styled(app.host_label.clone(), Style::new().fg(app.theme.accent)),
    ]);
    frame.render_widget(Paragraph::new(title), area);

    let status = match &app.capabilities {
        Some(caps) => format!("{} · mode: {} ", caps.label(), app.mode),
        None => format!("mode: {} ", app.mode),
    };
    let status = Line::from(status).alignment(Alignment::Right);
    frame.render_widget(
        Paragraph::new(status).style(Style::new().fg(app.theme.dim)),
        area,
    );
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
    let tabs = Tabs::new(titles)
        .select(app.active_tab)
        .style(Style::new().fg(app.theme.dim))
        .highlight_style(
            Style::new()
                .fg(app.theme.selected_fg)
                .bg(app.theme.selected_bg)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" ");
    frame.render_widget(tabs, area);
}

fn render_content(frame: &mut Frame, app: &App, area: Rect) {
    let tab = app.current_tab();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(app.theme.border))
        .title(format!(" {} ", tab.title()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match (&app.view_state, &app.snapshot, tab) {
        (ViewState::Ready, Some(snap), Tab::Dashboard) => {
            frame.render_widget(Paragraph::new(dashboard_text(app, snap)), inner);
        }
        (ViewState::Ready, Some(snap), Tab::System) => {
            frame.render_widget(Paragraph::new(system_text(app, snap)), inner);
        }
        (ViewState::Ready, _, Tab::Processes) => render_processes(frame, app, inner),
        (ViewState::Ready, _, Tab::Services) => render_services(frame, app, inner),
        (ViewState::Ready, _, Tab::Logs) => render_logs(frame, app, inner),
        (ViewState::Ready, _, Tab::Network) => render_network(frame, app, inner),
        (ViewState::Ready, _, Tab::Docker) => render_docker(frame, app, inner),
        (ViewState::Ready, _, Tab::Crons) => render_crons(frame, app, inner),
        (ViewState::Ready, _, Tab::Databases) => render_databases(frame, app, inner),
        (ViewState::Ready, _, Tab::Security) => render_security(frame, app, inner),
        _ => render_message(frame, app, tab, inner),
    }
}

fn render_logs(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    render_log_filter_bar(frame, app, rows[0]);

    // Case-insensitive regex search; invalid patterns fall back to substring.
    let regex = if app.log_search.is_empty() {
        None
    } else {
        RegexBuilder::new(&app.log_search)
            .case_insensitive(true)
            .build()
            .ok()
    };
    let filtered: Vec<&LogEntry> = app
        .logs
        .iter()
        .filter(|e| log_matches(e, &app.log_search, regex.as_ref()))
        .collect();

    if filtered.is_empty() {
        let msg = if app.logs.is_empty() {
            "No logs for this filter."
        } else {
            "No matches."
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(app.theme.dim)))
                .alignment(Alignment::Center),
            rows[1],
        );
        return;
    }

    let header = Row::new(["TIME", "PRIO", "SOURCE", "MESSAGE"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = filtered.iter().map(|e| {
        Row::new([
            Cell::from(e.time.clone()),
            Cell::from(Span::styled(
                e.priority_label().to_owned(),
                Style::new().fg(log_priority_color(app, e)),
            )),
            Cell::from(e.identifier.clone()),
            Cell::from(e.message.clone()),
        ])
    });
    let widths = [
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(18),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, rows[1]);
}

fn render_log_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let dim = Style::new().fg(app.theme.dim);
    let accent = Style::new().fg(app.theme.accent);
    let mut spans = vec![
        Span::styled(
            format!("level {} ", app.log_level_label()),
            Style::new().fg(app.theme.text),
        ),
        Span::styled(format!("· window {} ", app.log_window_label()), dim),
    ];
    if app.input_mode == InputMode::Search {
        spans.push(Span::styled(
            format!("· search: {}_", app.log_search),
            accent,
        ));
    } else if !app.log_search.is_empty() {
        spans.push(Span::styled(format!("· /{}", app.log_search), accent));
    } else {
        spans.push(Span::styled("· l level · t window · / search", dim));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn selected_style(app: &App) -> Style {
    Style::new()
        .fg(app.theme.selected_fg)
        .bg(app.theme.selected_bg)
        .add_modifier(Modifier::BOLD)
}

fn log_priority_color(app: &App, entry: &LogEntry) -> ratatui::style::Color {
    if entry.priority <= 3 {
        app.theme.danger
    } else if entry.priority == 4 {
        app.theme.warn
    } else {
        app.theme.dim
    }
}

fn log_matches(entry: &LogEntry, query: &str, regex: Option<&regex::Regex>) -> bool {
    if query.is_empty() {
        return true;
    }
    match regex {
        Some(re) => re.is_match(&entry.identifier) || re.is_match(&entry.message),
        None => {
            let q = query.to_lowercase();
            entry.identifier.to_lowercase().contains(&q)
                || entry.message.to_lowercase().contains(&q)
        }
    }
}

fn render_services(frame: &mut Frame, app: &App, area: Rect) {
    if app.failed_units.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No failed units — all good.",
                Style::new().fg(app.theme.ok),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let header = Row::new(["UNIT", "ACTIVE", "SUB", "DESCRIPTION"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.failed_units.iter().enumerate().map(|(i, u)| {
        let row = Row::new([
            Cell::from(Span::styled(
                u.name.clone(),
                Style::new().fg(app.theme.danger),
            )),
            Cell::from(u.active.clone()),
            Cell::from(u.sub.clone()),
            Cell::from(u.description.clone()),
        ]);
        if i == app.services_selected {
            row.style(selected_style(app))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(28),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, area);
}

fn render_processes(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    let hint = Line::from(vec![
        Span::styled(
            format!("sorted by {} ", app.process_sort.label()),
            Style::new().fg(app.theme.text),
        ),
        Span::styled("(s to toggle)", Style::new().fg(app.theme.dim)),
    ]);
    frame.render_widget(Paragraph::new(hint), rows[0]);
    let area = rows[1];

    if app.processes.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No process data.",
                Style::new().fg(app.theme.dim),
            )),
            area,
        );
        return;
    }

    let procs = app.visible_processes();
    let header = Row::new(["PID", "USER", "%CPU", "%MEM", "COMMAND"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = procs.iter().take(20).enumerate().map(|(i, p)| {
        let row = Row::new([
            p.pid.to_string(),
            p.user.clone(),
            format!("{:.1}", p.cpu_percent),
            format!("{:.1}", p.mem_percent),
            p.command.clone(),
        ]);
        if i == app.processes_selected {
            row.style(selected_style(app))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(7),
        Constraint::Length(12),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, rows[1]);
}

/// Color for a finding/exposure severity.
fn severity_color(app: &App, severity: Severity) -> ratatui::style::Color {
    match severity {
        Severity::Critical | Severity::High => app.theme.danger,
        Severity::Medium => app.theme.warn,
        Severity::Low => app.theme.accent,
        Severity::Info => app.theme.dim,
    }
}

/// A fixed-width severity badge, e.g. `[CRITICAL]`.
fn severity_badge(severity: Severity) -> String {
    format!("[{}]", severity.to_string().to_uppercase())
}

fn render_network(frame: &mut Frame, app: &App, area: Rect) {
    let rows = [area, area];
    let Some(net) = &app.network else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No network data — `ip`/`ss` unavailable.",
                Style::new().fg(app.theme.dim),
            )),
            rows[1],
        );
        return;
    };
    frame.render_widget(
        Paragraph::new(network_text(app, net)).wrap(Wrap { trim: false }),
        area,
    );
}

/// Interfaces, routing, DNS, connection states and the ranked exposure map.
fn network_text(app: &App, net: &NetworkSnapshot) -> Text<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let text_s = Style::new().fg(app.theme.text);
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);

    let mut lines = vec![Line::from(Span::styled("Interfaces", accent))];
    if net.interfaces.is_empty() {
        lines.push(Line::from(Span::styled("  none", dim)));
    }
    for iface in &net.interfaces {
        let addrs = iface
            .addrs
            .iter()
            .map(|a| format!("{}/{}", a.ip, a.prefix_len))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10} ", iface.name), text_s),
            Span::styled(format!("{:<8} ", iface.state), dim),
            Span::styled(addrs, text_s),
        ]));
    }

    let gateways: Vec<String> = net
        .routes
        .iter()
        .filter(|r| r.dst == "default")
        .filter_map(|r| r.gateway.clone().map(|g| format!("{g} via {}", r.dev)))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Gateway  ", accent),
        Span::styled(
            if gateways.is_empty() {
                "none".to_owned()
            } else {
                gateways.join(", ")
            },
            text_s,
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("DNS      ", accent),
        Span::styled(
            if net.dns.nameservers.is_empty() {
                "none".to_owned()
            } else {
                net.dns.nameservers.join(", ")
            },
            text_s,
        ),
    ]));

    let counts = net.connection_state_counts();
    if !counts.is_empty() {
        let summary = counts
            .iter()
            .map(|(state, n)| format!("{state} {n}"))
            .collect::<Vec<_>>()
            .join(" · ");
        lines.push(Line::from(vec![
            Span::styled("Conns    ", accent),
            Span::styled(summary, dim),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Exposure map ", accent),
        Span::styled("(worst first)", dim),
    ]));
    if app.exposures.is_empty() {
        lines.push(Line::from(Span::styled("  no listening sockets", dim)));
    }
    for entry in app.exposures.iter().take(12) {
        lines.push(exposure_line(app, entry));
    }

    Text::from(lines)
}

fn exposure_line(app: &App, entry: &ExposureEntry) -> Line<'static> {
    let proto = format!("{:?}", entry.listener.protocol).to_lowercase();
    let owner = match (&entry.listener.process, &entry.listener.unit) {
        (Some(p), Some(unit)) => format!("{} ({unit})", p.name),
        (Some(p), None) => p.name.clone(),
        (None, _) => "—".to_owned(),
    };
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", severity_badge(entry.severity)),
            Style::new().fg(severity_color(app, entry.severity)),
        ),
        Span::styled(
            format!(
                "{proto} {}:{}",
                entry.listener.local_ip, entry.listener.port
            ),
            Style::new().fg(app.theme.text),
        ),
        Span::styled(format!("  {owner}"), Style::new().fg(app.theme.dim)),
    ])
}

fn render_docker(frame: &mut Frame, app: &App, area: Rect) {
    if !app.docker_available {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "Docker unavailable — `docker` is not installed or the socket is not accessible.",
                Style::new().fg(app.theme.dim),
            ))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    if app.containers.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No containers.",
                Style::new().fg(app.theme.dim),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let rows =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    render_container_table(frame, app, rows[0]);
    render_container_detail(frame, app, rows[1]);
}

fn render_container_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(["NAME", "IMAGE", "STATE", "HEALTH", "PORTS"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.containers.iter().enumerate().map(|(i, c)| {
        let state_color = if c.is_running() {
            app.theme.ok
        } else if c.state == "restarting" {
            app.theme.warn
        } else {
            app.theme.dim
        };
        let (health, health_color) = container_health(app, c);
        let row = Row::new([
            Cell::from(c.name.clone()),
            Cell::from(c.image.clone()),
            Cell::from(Span::styled(c.state.clone(), Style::new().fg(state_color))),
            Cell::from(Span::styled(health, Style::new().fg(health_color))),
            Cell::from(c.ports.clone()),
        ]);
        if i == app.containers_selected {
            row.style(selected_style(app))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(18),
        Constraint::Length(22),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, area);
}

fn container_health(app: &App, c: &Container) -> (String, ratatui::style::Color) {
    use systui_collectors::ContainerHealth;
    match c.health {
        Some(ContainerHealth::Healthy) => ("healthy".to_owned(), app.theme.ok),
        Some(ContainerHealth::Unhealthy) => ("unhealthy".to_owned(), app.theme.danger),
        Some(ContainerHealth::Starting) => ("starting".to_owned(), app.theme.warn),
        None => ("-".to_owned(), app.theme.dim),
    }
}

fn render_container_detail(frame: &mut Frame, app: &App, area: Rect) {
    let dim = Style::new().fg(app.theme.dim);
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);
    let text_s = Style::new().fg(app.theme.text);

    let Some(inspect) = app.selected_inspect() else {
        let name = app
            .selected_container()
            .map(|c| c.name.as_str())
            .unwrap_or("container");
        frame.render_widget(
            Paragraph::new(Span::styled(format!("No inspect data for {name}."), dim)),
            area,
        );
        return;
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", inspect.name), accent),
        Span::styled(inspect.image.clone(), dim),
    ])];

    let mem = if inspect.memory_limit_bytes == 0 {
        "unlimited".to_owned()
    } else {
        human_kb(inspect.memory_limit_bytes / 1024)
    };
    lines.push(Line::from(Span::styled(
        format!(
            "  privileged {} · restart {} ({}) · mem {} · net {}",
            inspect.privileged,
            inspect.restart_policy,
            inspect.restart_count,
            mem,
            inspect.networks.join(",")
        ),
        text_s,
    )));

    if let Some(stats) = app
        .container_stats
        .iter()
        .find(|s| s.id == inspect.id || s.name == inspect.name)
    {
        lines.push(Line::from(Span::styled(
            format!(
                "  cpu {:.1}% · mem {:.1}% ({})",
                stats.cpu_percent, stats.mem_percent, stats.mem_usage
            ),
            text_s,
        )));
    }

    if !inspect.mounts.is_empty() {
        lines.push(Line::from(Span::styled("  mounts", dim)));
        for m in &inspect.mounts {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {} -> {} ({})",
                    m.source,
                    m.destination,
                    if m.rw { "rw" } else { "ro" }
                ),
                dim,
            )));
        }
    }

    let risks = systui_security::check_container(inspect);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Risks ({})", risks.len()),
        accent,
    )));
    if risks.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::new().fg(app.theme.ok),
        )));
    }
    for finding in risks.iter().take(6) {
        lines.push(finding_header(app, finding));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_crons(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Percentage(45),
        Constraint::Percentage(30),
        Constraint::Percentage(25),
    ])
    .split(area);
    render_cron_jobs(frame, app, rows[0]);
    render_cron_timers(frame, app, rows[1]);
    render_cron_warnings(frame, app, rows[2]);
}

fn render_cron_jobs(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let user_jobs = app
        .crons
        .iter()
        .filter(|entry| entry.source == CronSource::User)
        .count();
    let hint = if app.mode == systui_core::ExecutionMode::ReadOnly {
        "read-only mode"
    } else {
        "a add  e edit  d delete  x enable/disable user crontab"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("Jobs {}  user {}", app.crons.len(), user_jobs),
                Style::new().fg(app.theme.text),
            ),
            Span::styled(format!("  |  {hint}"), Style::new().fg(app.theme.dim)),
        ])),
        rows[0],
    );
    if app.crons.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No cron jobs found.",
                Style::new().fg(app.theme.dim),
            )),
            rows[1],
        );
        return;
    }
    let header = Row::new(["STATE", "SCHEDULE", "NEXT RUN", "USER", "COMMAND"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.crons.iter().enumerate().map(|(i, e)| {
        let (schedule, next) = cron_schedule_cells(app, e);
        let state = if e.enabled { "enabled" } else { "disabled" };
        let state_color = if e.enabled {
            app.theme.ok
        } else {
            app.theme.dim
        };
        let row = Row::new([
            Cell::from(Span::styled(state, Style::new().fg(state_color))),
            Cell::from(schedule),
            Cell::from(next),
            Cell::from(e.user.clone().unwrap_or_else(|| "—".to_owned())),
            Cell::from(e.command.clone()),
        ]);
        if i == app.crons_selected {
            row.style(selected_style(app))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(9),
        Constraint::Length(20),
        Constraint::Length(17),
        Constraint::Length(10),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, rows[1]);
}

/// The natural-language schedule and next-run cells for a cron entry. An invalid
/// expression renders as `invalid` with no next run.
fn cron_schedule_cells(app: &App, entry: &CronEntry) -> (String, String) {
    match parse_schedule(&entry.schedule) {
        Ok(schedule) => {
            let next = schedule
                .next_after(app.now)
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".to_owned());
            (schedule.describe(), next)
        }
        Err(_) => ("invalid".to_owned(), "—".to_owned()),
    }
}

fn render_cron_timers(frame: &mut Frame, app: &App, area: Rect) {
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);
    if app.timers.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("Systemd timers", accent)),
                Line::from(Span::styled("  none", Style::new().fg(app.theme.dim))),
            ]),
            area,
        );
        return;
    }
    let header = Row::new(["TIMER", "NEXT", "ACTIVATES"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.timers.iter().map(|t| {
        Row::new([
            Cell::from(t.unit.clone()),
            Cell::from(t.next.clone()),
            Cell::from(t.activates.clone()),
        ])
    });
    let widths = [
        Constraint::Length(26),
        Constraint::Length(26),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, area);
}

fn render_cron_warnings(frame: &mut Frame, app: &App, area: Rect) {
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(app.theme.dim);
    let warnings: Vec<&Finding> = app
        .findings
        .iter()
        .filter(|f| f.module == ModuleId::Crons)
        .collect();

    let mut lines = vec![Line::from(Span::styled(
        format!("Warnings ({})", warnings.len()),
        accent,
    ))];
    if warnings.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::new().fg(app.theme.ok),
        )));
    }
    for finding in warnings.iter().take(6) {
        lines.push(finding_header(app, finding));
        if let Some(evidence) = finding.evidence.first() {
            lines.push(Line::from(Span::styled(format!("      {evidence}"), dim)));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_databases(frame: &mut Frame, app: &App, area: Rect) {
    if app.databases.instances.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No database services found.",
                Style::new().fg(app.theme.dim),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let rows =
        Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    render_database_table(frame, app, rows[0]);
    render_database_detail(frame, app, rows[1]);
}

fn render_database_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(["ENGINE", "SERVICE", "ENDPOINT", "EXPOSURE", "VERSION"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.databases.instances.iter().enumerate().map(|(i, db)| {
        let exposure = database_exposure_label(db);
        let exposure_color = match db.exposure {
            Some(BindScope::External) => app.theme.danger,
            Some(BindScope::Loopback) => app.theme.ok,
            None => app.theme.dim,
        };
        let row = Row::new([
            Cell::from(db.engine.label()),
            Cell::from(
                db.service
                    .as_ref()
                    .map(|s| s.unit.clone())
                    .unwrap_or_else(|| "-".to_owned()),
            ),
            Cell::from(db.endpoint().unwrap_or_else(|| "-".to_owned())),
            Cell::from(Span::styled(exposure, Style::new().fg(exposure_color))),
            Cell::from(db.version.clone().unwrap_or_else(|| "-".to_owned())),
        ]);
        if i == app.databases_selected {
            row.style(selected_style(app))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(15),
        Constraint::Length(28),
        Constraint::Length(22),
        Constraint::Length(10),
        Constraint::Min(10),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .style(Style::new().fg(app.theme.text));
    frame.render_widget(table, area);
}

fn render_database_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(db) = app.selected_database() else {
        return;
    };
    let dim = Style::new().fg(app.theme.dim);
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);
    let text_s = Style::new().fg(app.theme.text);

    let mut lines = vec![Line::from(vec![
        Span::styled(db.engine.label(), accent),
        Span::styled(
            format!(
                "  {}",
                db.endpoint().unwrap_or_else(|| "no listener".to_owned())
            ),
            dim,
        ),
    ])];

    if let Some(service) = &db.service {
        lines.push(Line::from(Span::styled(
            format!(
                "  service {} active={} sub={}",
                service.unit, service.active, service.sub
            ),
            text_s,
        )));
    }
    if let Some(process) = db.process() {
        lines.push(Line::from(Span::styled(
            format!("  process {} pid {}", process.name, process.pid),
            text_s,
        )));
    }
    if !db.credential_sources.is_empty() {
        let labels = db
            .credential_sources
            .iter()
            .map(|source| source.label.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(Span::styled(
            format!("  credentials {labels}"),
            dim,
        )));
    }

    let op = &db.operational;
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Operational signals", accent)));
    push_operational_line(
        &mut lines,
        "connections",
        op.connection_summary.as_deref(),
        dim,
        text_s,
    );
    push_operational_line(&mut lines, "size", op.size_summary.as_deref(), dim, text_s);
    push_operational_line(
        &mut lines,
        "replication",
        op.replication_summary.as_deref(),
        dim,
        text_s,
    );
    push_operational_line(&mut lines, "locks", op.lock_summary.as_deref(), dim, text_s);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Recent errors ({})", op.recent_errors.len()),
        accent,
    )));
    if op.recent_errors.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::new().fg(app.theme.ok),
        )));
    }
    for entry in op.recent_errors.iter().take(4) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", entry.priority_label()),
                Style::new().fg(app.theme.danger),
            ),
            Span::styled(format!("{} ", entry.time), dim),
            Span::styled(entry.message.clone(), text_s),
        ]));
    }

    let db_findings: Vec<&Finding> = app
        .findings
        .iter()
        .filter(|f| f.module == ModuleId::Databases)
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Database findings ({})", db_findings.len()),
        accent,
    )));
    if db_findings.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::new().fg(app.theme.ok),
        )));
    }
    for finding in db_findings.iter().take(4) {
        lines.push(finding_header(app, finding));
    }
    for note in op.notes.iter().take(3) {
        lines.push(Line::from(Span::styled(format!("  note: {note}"), dim)));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn database_exposure_label(db: &DatabaseInstance) -> String {
    match db.exposure {
        Some(BindScope::External) => "external".to_owned(),
        Some(BindScope::Loopback) => "loopback".to_owned(),
        None => "unknown".to_owned(),
    }
}

fn push_operational_line(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: Option<&str>,
    dim: Style,
    text_s: Style,
) {
    lines.push(Line::from(vec![
        Span::styled(format!("  {label:<12}"), dim),
        Span::styled(value.unwrap_or("unavailable").to_owned(), text_s),
    ]));
}

fn render_security(frame: &mut Frame, app: &App, area: Rect) {
    if app.findings.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No findings — nothing flagged.",
                Style::new().fg(app.theme.ok),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(security_text(app)).wrap(Wrap { trim: false }),
        area,
    );
}

/// The prioritized, evidence-based findings list (worst first).
fn security_text(app: &App) -> Text<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let text_s = Style::new().fg(app.theme.text);

    let [crit, high, med, low, info] = app.finding_counts();
    let mut lines = vec![
        Line::from(vec![
            Span::styled("critical ", dim),
            Span::styled(crit.to_string(), Style::new().fg(app.theme.danger)),
            Span::styled("  high ", dim),
            Span::styled(high.to_string(), Style::new().fg(app.theme.danger)),
            Span::styled("  medium ", dim),
            Span::styled(med.to_string(), Style::new().fg(app.theme.warn)),
            Span::styled("  low ", dim),
            Span::styled(low.to_string(), Style::new().fg(app.theme.accent)),
            Span::styled("  info ", dim),
            Span::styled(info.to_string(), text_s),
        ]),
        Line::from(""),
    ];

    for finding in app.findings.iter().take(14) {
        lines.push(finding_header(app, finding));
        if let Some(evidence) = finding.evidence.first() {
            lines.push(Line::from(Span::styled(format!("      {evidence}"), dim)));
        }
        if !finding.recommendation.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("      → {}", finding.recommendation),
                Style::new().fg(app.theme.ok),
            )));
        }
    }

    Text::from(lines)
}

fn finding_header(app: &App, finding: &Finding) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<10}", severity_badge(finding.severity)),
            Style::new()
                .fg(severity_color(app, finding.severity))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            finding.title.clone(),
            Style::new().fg(app.theme.text).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn render_message(frame: &mut Frame, app: &App, tab: Tab, area: Rect) {
    let (heading, body) = content_message(app, tab);
    let text = Text::from(vec![
        Line::from(""),
        Line::from(Span::styled(
            heading,
            Style::new().fg(app.theme.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(body, Style::new().fg(app.theme.dim))),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn content_message(app: &App, tab: Tab) -> (String, String) {
    match &app.view_state {
        ViewState::Loading => ("Loading…".to_owned(), "Collecting data.".to_owned()),
        ViewState::Empty => (
            tab.title().to_owned(),
            "No data yet — press r to refresh.".to_owned(),
        ),
        ViewState::Ready => (
            tab.title().to_owned(),
            "No data for this module yet — arrives in a later phase.".to_owned(),
        ),
        ViewState::PartialData(msg) => ("Partial data".to_owned(), msg.clone()),
        ViewState::PermissionDenied(msg) => ("Permission denied".to_owned(), msg.clone()),
        ViewState::Error(msg) => ("Error".to_owned(), msg.clone()),
    }
}

/// The prioritized overview: header, CPU/RAM/swap bars and disk usage.
fn dashboard_text(app: &App, snap: &SystemSnapshot) -> Text<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let text_s = Style::new().fg(app.theme.text);
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(snap.hostname.clone(), accent),
            Span::styled("  ·  ", dim),
            Span::styled(
                snap.os.clone().unwrap_or_else(|| "unknown OS".to_owned()),
                text_s,
            ),
        ]),
        Line::from(vec![Span::styled(
            format!(
                "up {} · load {:.2} {:.2} {:.2} · {} cores",
                human_uptime(snap.uptime_secs),
                snap.load.one,
                snap.load.five,
                snap.load.fifteen,
                snap.cpu.cores,
            ),
            dim,
        )]),
        Line::from(""),
        metric_line(app, "CPU ", snap.cpu.busy_percent, None),
        metric_line(
            app,
            "RAM ",
            snap.memory.used_percent(),
            Some(format!(
                "{} / {}",
                human_kb(snap.memory.used_kb()),
                human_kb(snap.memory.total_kb)
            )),
        ),
    ];

    let failed = app.failed_units.len();
    let errors = app.logs.iter().filter(|e| e.is_error()).count();
    let count_style = |n: usize| {
        if n > 0 {
            Style::new()
                .fg(app.theme.danger)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(app.theme.ok)
        }
    };
    lines.insert(
        2,
        Line::from(vec![
            Span::styled("failed units: ", dim),
            Span::styled(failed.to_string(), count_style(failed)),
            Span::styled("   recent errors: ", dim),
            Span::styled(errors.to_string(), count_style(errors)),
        ]),
    );

    if snap.swap.total_kb > 0 {
        lines.push(metric_line(
            app,
            "Swap",
            snap.swap.used_percent(),
            Some(format!(
                "{} / {}",
                human_kb(snap.swap.used_kb()),
                human_kb(snap.swap.total_kb)
            )),
        ));
    } else {
        lines.push(Line::from(vec![
            Span::styled("Swap  ", text_s),
            Span::styled("none", dim),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Disks", accent)));
    for disk in &snap.disks {
        lines.push(disk_line(app, disk));
    }

    if let Some(health) = &app.health {
        // Health score line at the very top, colored by severity band.
        let score_color = if health.score >= 80 {
            app.theme.ok
        } else if health.score >= 50 {
            app.theme.warn
        } else {
            app.theme.danger
        };
        lines.insert(
            0,
            Line::from(vec![
                Span::styled("Health ", accent),
                Span::styled(
                    format!("{}/100", health.score),
                    Style::new().fg(score_color).add_modifier(Modifier::BOLD),
                ),
            ]),
        );

        // Prioritized findings at the bottom.
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Findings", accent)));
        if health.checks.is_empty() {
            lines.push(Line::from(Span::styled("  none — healthy", dim)));
        } else {
            for check in health.checks.iter().take(8) {
                lines.push(finding_line(app, check));
            }
        }
    }

    // Security posture: network exposure and the worst security findings.
    let [crit, high, med, ..] = app.finding_counts();
    let risky = app.risky_exposure_count();
    let count_span = |n: usize, color| {
        let style = if n > 0 {
            Style::new().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(app.theme.ok)
        };
        Span::styled(n.to_string(), style)
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Security", accent)));
    lines.push(Line::from(vec![
        Span::styled("  exposed ports: ", dim),
        count_span(risky, app.theme.danger),
        Span::styled("   critical: ", dim),
        count_span(crit, app.theme.danger),
        Span::styled("   high: ", dim),
        count_span(high, app.theme.danger),
        Span::styled("   medium: ", dim),
        count_span(med, app.theme.warn),
    ]));

    // Docker & crons: running/total containers and job/timer counts, each with
    // the number of risk findings their module produced.
    let module_findings = |module| app.findings.iter().filter(|f| f.module == module).count();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Docker", accent)));
    if app.docker_available {
        let running = app.containers.iter().filter(|c| c.is_running()).count();
        lines.push(Line::from(vec![
            Span::styled("  running: ", dim),
            Span::styled(
                format!("{running}/{}", app.containers.len()),
                Style::new().fg(app.theme.text),
            ),
            Span::styled("   issues: ", dim),
            count_span(module_findings(ModuleId::Docker), app.theme.warn),
        ]));
    } else {
        lines.push(Line::from(Span::styled("  unavailable", dim)));
    }
    lines.push(Line::from(Span::styled("Crons", accent)));
    lines.push(Line::from(vec![
        Span::styled("  jobs: ", dim),
        Span::styled(app.crons.len().to_string(), Style::new().fg(app.theme.text)),
        Span::styled("   timers: ", dim),
        Span::styled(
            app.timers.len().to_string(),
            Style::new().fg(app.theme.text),
        ),
        Span::styled("   warnings: ", dim),
        count_span(module_findings(ModuleId::Crons), app.theme.warn),
    ]));

    Text::from(lines)
}

fn finding_line(app: &App, check: &systui_collectors::Check) -> Line<'static> {
    use systui_core::Severity;
    let color = match check.severity {
        Severity::Critical | Severity::High => app.theme.danger,
        Severity::Medium => app.theme.warn,
        _ => app.theme.dim,
    };
    let label = format!("[{:?}]", check.severity).to_uppercase();
    Line::from(vec![
        Span::styled(
            format!("  -{:<3}", check.points),
            Style::new().fg(app.theme.dim),
        ),
        Span::styled(format!("{label:<10}"), Style::new().fg(color)),
        Span::styled(check.message.clone(), Style::new().fg(app.theme.text)),
    ])
}

/// The full system detail view.
fn system_text(app: &App, snap: &SystemSnapshot) -> Text<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let accent = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);

    let rows = [
        ("Hostname", snap.hostname.clone()),
        (
            "OS",
            snap.os.clone().unwrap_or_else(|| "unknown".to_owned()),
        ),
        ("Kernel", snap.kernel.clone()),
        ("Uptime", human_uptime(snap.uptime_secs)),
        (
            "CPU",
            format!(
                "{:.0}% busy · {} cores",
                snap.cpu.busy_percent, snap.cpu.cores
            ),
        ),
        (
            "Load",
            format!(
                "{:.2}  {:.2}  {:.2}",
                snap.load.one, snap.load.five, snap.load.fifteen
            ),
        ),
        (
            "Memory",
            format!(
                "{} / {} ({:.0}%)",
                human_kb(snap.memory.used_kb()),
                human_kb(snap.memory.total_kb),
                snap.memory.used_percent()
            ),
        ),
        (
            "Swap",
            if snap.swap.total_kb > 0 {
                format!(
                    "{} / {} ({:.0}%)",
                    human_kb(snap.swap.used_kb()),
                    human_kb(snap.swap.total_kb),
                    snap.swap.used_percent()
                )
            } else {
                "none".to_owned()
            },
        ),
    ];

    let mut lines = vec![Line::from("")];
    for (key, value) in rows {
        lines.push(label_value(app, key, &value));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Disks", accent)));
    for disk in &snap.disks {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<10} {:>3}%  {} / {}  ({})",
                disk.mount,
                disk.use_percent,
                human_kb(disk.used_kb),
                human_kb(disk.size_kb),
                disk.filesystem
            ),
            dim,
        )]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Users", accent)));
    if snap.users.is_empty() {
        lines.push(Line::from(Span::styled("  none", dim)));
    } else {
        for user in &snap.users {
            let from = user
                .from
                .as_deref()
                .map(|f| format!(" ({f})"))
                .unwrap_or_default();
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  {:<10} {:<10} {}{}",
                    user.name, user.tty, user.login_time, from
                ),
                dim,
            )]));
        }
    }

    Text::from(lines)
}

fn metric_line(app: &App, label: &str, percent: f64, detail: Option<String>) -> Line<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let text_s = Style::new().fg(app.theme.text);
    let mut spans = vec![
        Span::styled(format!("{label} "), text_s),
        Span::styled(
            format!("[{}]", bar(percent, 20)),
            Style::new().fg(app.theme.accent),
        ),
        Span::styled(format!(" {percent:>3.0}%"), text_s),
    ];
    if let Some(detail) = detail {
        spans.push(Span::styled(format!("  {detail}"), dim));
    }
    Line::from(spans)
}

fn disk_line(app: &App, disk: &Disk) -> Line<'static> {
    let dim = Style::new().fg(app.theme.dim);
    let text_s = Style::new().fg(app.theme.text);
    Line::from(vec![
        Span::styled(format!("  {:<10} ", disk.mount), text_s),
        Span::styled(
            format!("[{}]", bar(disk.use_percent as f64, 16)),
            Style::new().fg(app.theme.accent),
        ),
        Span::styled(format!(" {:>3}%", disk.use_percent), text_s),
        Span::styled(
            format!("  {} / {}", human_kb(disk.used_kb), human_kb(disk.size_kb)),
            dim,
        ),
    ])
}

fn label_value(app: &App, key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<10}"), Style::new().fg(app.theme.dim)),
        Span::styled(
            value.to_owned(),
            Style::new().fg(app.theme.text).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// A textual progress bar of `width` cells.
fn bar(percent: f64, width: usize) -> String {
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width);
    s.extend(std::iter::repeat_n('█', filled));
    s.extend(std::iter::repeat_n('░', width - filled));
    s
}

/// Format a kB amount (1024-based) as KiB/MiB/GiB/TiB.
fn human_kb(kb: u64) -> String {
    const MIB: f64 = 1024.0;
    const GIB: f64 = 1024.0 * 1024.0;
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let kb_f = kb as f64;
    if kb_f >= TIB {
        format!("{:.1} TiB", kb_f / TIB)
    } else if kb_f >= GIB {
        format!("{:.1} GiB", kb_f / GIB)
    } else if kb_f >= MIB {
        format!("{:.1} MiB", kb_f / MIB)
    } else {
        format!("{kb} KiB")
    }
}

/// Format seconds as `Xd Yh Zm`.
fn human_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    format!("{days}d {hours}h {mins}m")
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hint = if app.current_tab() == Tab::Crons {
        " r refresh | a add | e edit | d delete | x toggle | ? help | q quit "
    } else {
        " r refresh | / search | a actions | ? help | q quit "
    };
    frame.render_widget(
        Paragraph::new(Line::from(hint)).style(Style::new().fg(app.theme.dim)),
        area,
    );
}

fn render_help(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(app.theme.title))
        .title(" Help ");

    let keys = [
        ("Tab / →", "next tab"),
        ("Shift+Tab / ←", "previous tab"),
        ("1–9", "jump to tab"),
        ("↑ / ↓", "move selection (services/processes/docker/crons)"),
        ("a", "act on selection; add cron job on Crons"),
        ("e / d / x", "edit, delete or toggle a user crontab entry"),
        ("r", "refresh"),
        ("s", "sort processes by CPU/memory"),
        ("/", "search logs (Esc to clear)"),
        ("l / t", "cycle log level / time window"),
        ("?", "toggle this help"),
        ("q / Ctrl+C", "quit"),
        ("Esc", "close overlay / back"),
    ];
    let mut lines = vec![Line::from("")];
    for (key, desc) in keys {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<14}"),
                Style::new()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc, Style::new().fg(app.theme.text)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use systui_core::ExecutionMode;

    fn render_to_string(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_chrome_and_empty_state() {
        let app = App::new("prod-01", ExecutionMode::ReadOnly);
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("SysTUI"));
        assert!(out.contains("prod-01"));
        assert!(out.contains("read-only"));
        assert!(out.contains("Dashboard"));
        assert!(out.contains("Security"));
        assert!(out.contains("q quit"));
        assert!(out.contains("No data yet"));
    }

    #[test]
    fn title_shows_user_capabilities() {
        use systui_collectors::HostCapabilities;
        let mut app = App::new("prod-01", ExecutionMode::Privileged);
        app.capabilities = Some(HostCapabilities {
            user: "admin".to_owned(),
            uid: Some(1000),
            can_sudo: true,
        });
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("admin (sudo)"));
        assert!(out.contains("privileged"));
    }

    #[test]
    fn error_state_is_shown() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.view_state = ViewState::Error("disk collector failed".to_owned());
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("Error"));
        assert!(out.contains("disk collector failed"));
    }

    #[test]
    fn help_overlay_renders_key_bindings() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.toggle_help();
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("Help"));
        assert!(out.contains("next tab"));
        assert!(out.contains("quit"));
    }

    fn sample_snapshot() -> SystemSnapshot {
        use systui_collectors::{CpuUsage, LoadAverage, LoggedUser, Memory, Swap};
        SystemSnapshot {
            hostname: "prod-01".to_owned(),
            os: Some("Debian GNU/Linux 12".to_owned()),
            kernel: "6.1.0-18-amd64".to_owned(),
            uptime_secs: 123_456,
            load: LoadAverage {
                one: 0.52,
                five: 0.58,
                fifteen: 0.59,
            },
            cpu: CpuUsage {
                busy_percent: 28.0,
                cores: 4,
            },
            memory: Memory {
                total_kb: 16_000_000,
                available_kb: 6_400_000,
            },
            swap: Swap {
                total_kb: 4_000_000,
                free_kb: 3_000_000,
            },
            disks: vec![
                Disk {
                    filesystem: "/dev/sda1".to_owned(),
                    size_kb: 102_687_672,
                    used_kb: 86_012_345,
                    avail_kb: 11_425_327,
                    use_percent: 89,
                    mount: "/".to_owned(),
                },
                Disk {
                    filesystem: "/dev/sda2".to_owned(),
                    size_kb: 515_928_320,
                    used_kb: 120_000_000,
                    avail_kb: 369_000_000,
                    use_percent: 25,
                    mount: "/home".to_owned(),
                },
            ],
            users: vec![LoggedUser {
                name: "admin".to_owned(),
                tty: "pts/0".to_owned(),
                from: Some("10.0.0.5".to_owned()),
                login_time: "2026-05-24 09:12".to_owned(),
            }],
        }
    }

    #[test]
    fn renders_dashboard_when_ready() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("prod-01"));
        assert!(out.contains("CPU"));
        assert!(out.contains("RAM"));
        assert!(out.contains("Disks"));
        assert!(out.contains("/home"));
        assert!(out.contains("89%"));
    }

    #[test]
    fn renders_system_detail_when_ready() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(1); // System

        let out = render_to_string(&app, 100, 30);
        assert!(out.contains("Hostname"));
        assert!(out.contains("Kernel"));
        assert!(out.contains("6.1.0-18-amd64"));
        assert!(out.contains("Users"));
        assert!(out.contains("admin"));
    }

    #[test]
    fn renders_top_processes_table() {
        use systui_collectors::Process;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.processes = vec![
            Process {
                pid: 1,
                ppid: 0,
                user: "root".to_owned(),
                cpu_percent: 0.1,
                mem_percent: 0.2,
                command: "systemd".to_owned(),
            },
            Process {
                pid: 3300,
                ppid: 1,
                user: "admin".to_owned(),
                cpu_percent: 12.4,
                mem_percent: 0.8,
                command: "node".to_owned(),
            },
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(2); // Processes

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("sorted by CPU"));
        assert!(out.contains("COMMAND"));
        assert!(out.contains("node"));
        assert!(out.contains("3300"));
        // node (12.4% CPU) must appear before systemd (0.1%) in CPU sort order
        let node_at = out.find("node").unwrap();
        let systemd_at = out.find("systemd").unwrap();
        assert!(node_at < systemd_at);
    }

    #[test]
    fn dashboard_shows_failed_unit_count() {
        use systui_collectors::ServiceUnit;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.failed_units = vec![ServiceUnit {
            name: "nginx.service".to_owned(),
            load: "loaded".to_owned(),
            active: "failed".to_owned(),
            sub: "failed".to_owned(),
            description: "web server".to_owned(),
        }];
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("failed units: 1"));
    }

    #[test]
    fn services_tab_lists_failed_units() {
        use systui_collectors::ServiceUnit;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.failed_units = vec![ServiceUnit {
            name: "docker.service".to_owned(),
            load: "loaded".to_owned(),
            active: "failed".to_owned(),
            sub: "failed".to_owned(),
            description: "Docker Application Container Engine".to_owned(),
        }];
        app.view_state = ViewState::Ready;
        app.select_tab(3); // Services

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("UNIT"));
        assert!(out.contains("docker.service"));
    }

    #[test]
    fn services_tab_reports_no_failures() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(3); // Services

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("No failed units"));
    }

    #[test]
    fn dashboard_shows_health_score_and_findings() {
        use systui_collectors::{Check, HealthReport};
        use systui_core::Severity;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.health = Some(HealthReport {
            score: 72,
            checks: vec![Check {
                severity: Severity::Critical,
                message: "/ at 95% (>= 90% critical)".to_owned(),
                points: 15,
            }],
        });
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 100, 30);
        assert!(out.contains("Health"));
        assert!(out.contains("72/100"));
        assert!(out.contains("Findings"));
        assert!(out.contains("CRITICAL"));
        assert!(out.contains("95%"));
    }

    fn sample_network() -> systui_collectors::NetworkSnapshot {
        use systui_collectors::{
            AddrFamily, Connection, DnsConfig, InterfaceAddr, Listener, NetInterface,
            NetworkSnapshot, ProcessRef, Protocol, Route,
        };
        NetworkSnapshot {
            interfaces: vec![NetInterface {
                name: "eth0".to_owned(),
                state: "UP".to_owned(),
                addrs: vec![InterfaceAddr {
                    ip: "192.168.1.10".to_owned(),
                    prefix_len: 24,
                    family: AddrFamily::V4,
                }],
            }],
            routes: vec![Route {
                dst: "default".to_owned(),
                gateway: Some("192.168.1.1".to_owned()),
                dev: "eth0".to_owned(),
                prefsrc: None,
            }],
            dns: DnsConfig {
                nameservers: vec!["1.1.1.1".to_owned()],
                search: Vec::new(),
            },
            listeners: vec![Listener {
                protocol: Protocol::Tcp,
                local_ip: "0.0.0.0".to_owned(),
                port: 6379,
                process: Some(ProcessRef {
                    pid: 1500,
                    name: "redis-server".to_owned(),
                }),
                unit: Some("redis.service".to_owned()),
            }],
            connections: vec![Connection {
                state: "ESTAB".to_owned(),
                local_ip: "192.168.1.10".to_owned(),
                local_port: 22,
                peer_ip: "10.0.0.2".to_owned(),
                peer_port: 51000,
            }],
        }
    }

    #[test]
    fn network_tab_shows_interfaces_and_exposure() {
        use systui_collectors::exposure_map;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        let net = sample_network();
        app.exposures = exposure_map(&net.listeners);
        app.network = Some(net);
        app.view_state = ViewState::Ready;
        app.select_tab(5); // Network

        let out = render_to_string(&app, 100, 30);
        assert!(out.contains("Interfaces"));
        assert!(out.contains("eth0"));
        assert!(out.contains("192.168.1.1")); // gateway
        assert!(out.contains("Exposure map"));
        assert!(out.contains("CRITICAL")); // redis on 0.0.0.0:6379
        assert!(out.contains("6379"));
        assert!(out.contains("redis.service"));
    }

    #[test]
    fn security_tab_lists_findings_worst_first() {
        use systui_core::{Finding, ModuleId, Severity};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.findings = vec![
            Finding::new(
                "ssh.root-login",
                Severity::High,
                ModuleId::Security,
                "SSH permits direct root login",
            )
            .with_evidence("/etc/ssh/sshd_config: PermitRootLogin yes")
            .recommendation("Set PermitRootLogin to no."),
            Finding::new(
                "firewall.absent",
                Severity::Medium,
                ModuleId::Firewall,
                "No active firewall detected",
            ),
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(9); // Security

        let out = render_to_string(&app, 100, 30);
        assert!(out.contains("SSH permits direct root login"));
        assert!(out.contains("PermitRootLogin yes"));
        assert!(out.contains("No active firewall detected"));
        // High appears before Medium.
        let high_at = out.find("HIGH").unwrap();
        let med_at = out.find("MEDIUM").unwrap();
        assert!(high_at < med_at);
    }

    #[test]
    fn security_tab_reports_clean() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(9);
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("No findings"));
    }

    #[test]
    fn dashboard_shows_security_summary() {
        use systui_core::{Finding, ModuleId, Severity};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.findings = vec![Finding::new(
            "net.sensitive-port.6379",
            Severity::Critical,
            ModuleId::Network,
            "Sensitive service exposed",
        )];
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 100, 40);
        assert!(out.contains("Security"));
        assert!(out.contains("exposed ports:"));
        assert!(out.contains("critical:"));
    }

    #[test]
    fn logs_tab_lists_recent_errors() {
        use systui_collectors::LogEntry;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.logs = vec![LogEntry {
            time: "09:12:00".to_owned(),
            priority: 3,
            identifier: "nginx".to_owned(),
            message: "upstream timed out".to_owned(),
        }];
        app.view_state = ViewState::Ready;
        app.select_tab(4); // Logs

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("PRIO"));
        assert!(out.contains("ERR"));
        assert!(out.contains("nginx"));
        assert!(out.contains("upstream timed out"));
    }

    fn log(identifier: &str, message: &str) -> LogEntry {
        LogEntry {
            time: "09:00:00".to_owned(),
            priority: 3,
            identifier: identifier.to_owned(),
            message: message.to_owned(),
        }
    }

    #[test]
    fn logs_tab_filters_by_search() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.logs = vec![
            log("nginx", "upstream timed out"),
            log("sshd", "failed password"),
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(4); // Logs
        app.log_search = "sshd".to_owned();

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("failed password"));
        assert!(!out.contains("upstream timed out"));
    }

    #[test]
    fn action_modal_renders_confirmation() {
        use crate::app::{ActionModal, ActionStage};
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.set_decision(systui_actions::ActionDecision::NeedsConfirmation {
            preview: systui_core::ActionPreview {
                summary: "Restart nginx.service".to_owned(),
                details: vec!["Restarts the unit; it will be briefly unavailable.".to_owned()],
                command: None,
                reversible: false,
                creates_backup: false,
            },
            phrase: "Restart nginx.service".to_owned(),
        });

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("Restart nginx.service"));
        assert!(out.contains("Type to confirm"));

        // sanity: the modal type is what we expect
        let modal: &ActionModal = app.action.as_ref().unwrap();
        assert_eq!(modal.stage, ActionStage::Confirm);
    }

    fn sample_container() -> systui_collectors::Container {
        systui_collectors::Container {
            id: "abc123".to_owned(),
            name: "redis".to_owned(),
            image: "redis:latest".to_owned(),
            state: "running".to_owned(),
            status: "Up 2 days (unhealthy)".to_owned(),
            health: Some(systui_collectors::ContainerHealth::Unhealthy),
            ports: "0.0.0.0:6379->6379/tcp".to_owned(),
            created: "2026-05-22".to_owned(),
        }
    }

    fn sample_inspect() -> systui_collectors::InspectSummary {
        use systui_collectors::{ContainerHealth, Mount, PublishedPort};
        systui_collectors::InspectSummary {
            id: "abc123".to_owned(),
            name: "redis".to_owned(),
            image: "redis:latest".to_owned(),
            privileged: true,
            restart_policy: "always".to_owned(),
            max_retry_count: 0,
            restart_count: 7,
            memory_limit_bytes: 0,
            networks: vec!["bridge".to_owned()],
            mounts: vec![Mount {
                source: "/var/run/docker.sock".to_owned(),
                destination: "/var/run/docker.sock".to_owned(),
                rw: true,
            }],
            published_ports: vec![PublishedPort {
                host_ip: "0.0.0.0".to_owned(),
                host_port: 6379,
                container_port: 6379,
                protocol: "tcp".to_owned(),
            }],
            health: Some(ContainerHealth::Unhealthy),
        }
    }

    #[test]
    fn docker_tab_lists_containers_and_shows_risks() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.docker_available = true;
        app.containers = vec![sample_container()];
        app.container_inspects = vec![sample_inspect()];
        app.view_state = ViewState::Ready;
        app.select_tab(6); // Docker

        let out = render_to_string(&app, 110, 30);
        assert!(out.contains("redis"));
        assert!(out.contains("redis:latest"));
        assert!(out.contains("unhealthy"));
        assert!(out.contains("Risks"));
        // The privileged + docker.sock risks surface in the detail panel.
        assert!(out.contains("privileged") || out.contains("Docker socket"));
    }

    #[test]
    fn docker_tab_reports_unavailable() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.docker_available = false;
        app.view_state = ViewState::Ready;
        app.select_tab(6);

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("Docker unavailable"));
    }

    #[test]
    fn crons_tab_lists_jobs_timers_and_warnings() {
        use systui_collectors::{CronEntry, CronSource, SystemdTimer};
        use systui_core::{Finding, ModuleId, Severity};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.now = chrono::NaiveDate::from_ymd_opt(2026, 5, 24)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        app.crons = vec![CronEntry {
            schedule: "0 2 * * *".to_owned(),
            user: Some("root".to_owned()),
            command: "/opt/backup.sh".to_owned(),
            source: CronSource::System,
            origin: "/etc/crontab".to_owned(),
            enabled: true,
        }];
        app.timers = vec![SystemdTimer {
            unit: "logrotate.timer".to_owned(),
            activates: "logrotate.service".to_owned(),
            next: "Wed 2026-05-27 00:00:00 UTC".to_owned(),
        }];
        app.findings = vec![Finding::new(
            "cron.no-logging./opt/backup.sh",
            Severity::Info,
            ModuleId::Crons,
            "Cron job output is not captured",
        )];
        app.view_state = ViewState::Ready;
        app.select_tab(7); // Crons

        let out = render_to_string(&app, 110, 36);
        assert!(out.contains("Every day at 02:00"));
        assert!(out.contains("2026-05-25 02:00")); // next run
        assert!(out.contains("/opt/backup.sh"));
        assert!(out.contains("logrotate.timer"));
        assert!(out.contains("Warnings"));
        assert!(out.contains("not captured"));
    }

    #[test]
    fn databases_tab_lists_instances_and_operational_signals() {
        use systui_collectors::{
            BindScope, DatabaseEngine, DatabaseInstance, DatabaseOperational, DatabaseService,
            DatabaseSnapshot, Listener, LogEntry, ProcessRef, Protocol,
        };
        use systui_core::{Finding, ModuleId, Severity};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.databases = DatabaseSnapshot {
            instances: vec![DatabaseInstance {
                engine: DatabaseEngine::Redis,
                service: Some(DatabaseService {
                    unit: "redis-server.service".to_owned(),
                    active: "active".to_owned(),
                    sub: "running".to_owned(),
                    description: "Redis".to_owned(),
                }),
                listener: Some(Listener {
                    protocol: Protocol::Tcp,
                    local_ip: "0.0.0.0".to_owned(),
                    port: 6379,
                    process: Some(ProcessRef {
                        pid: 1500,
                        name: "redis-server".to_owned(),
                    }),
                    unit: Some("redis-server.service".to_owned()),
                }),
                version: Some("Redis server v=7.0.15".to_owned()),
                exposure: Some(BindScope::External),
                credential_sources: Vec::new(),
                operational: DatabaseOperational {
                    connection_summary: Some("12 connected clients".to_owned()),
                    size_summary: Some("10.40M memory, 1200 keys".to_owned()),
                    replication_summary: Some("master with 2 replicas".to_owned()),
                    lock_summary: Some("1 blocked clients".to_owned()),
                    recent_errors: vec![LogEntry {
                        time: "09:00:00".to_owned(),
                        priority: 3,
                        identifier: "redis-server".to_owned(),
                        message: "background save failed".to_owned(),
                    }],
                    notes: Vec::new(),
                },
                detected_by: vec!["default port 6379".to_owned()],
            }],
        };
        app.findings = vec![Finding::new(
            "db.exposed.redis.6379",
            Severity::Critical,
            ModuleId::Databases,
            "Redis is reachable on a non-loopback address",
        )];
        app.view_state = ViewState::Ready;
        app.select_tab(8); // Databases

        let out = render_to_string(&app, 120, 34);
        assert!(out.contains("Redis"));
        assert!(out.contains("0.0.0.0:6379"));
        assert!(out.contains("external"));
        assert!(out.contains("12 connected clients"));
        assert!(out.contains("background save failed"));
        assert!(out.contains("Database findings"));
    }

    #[test]
    fn logs_tab_shows_filter_bar_and_search_input() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.logs = vec![log("nginx", "boom")];
        app.view_state = ViewState::Ready;
        app.select_tab(4);
        app.enter_search();
        app.push_search_char('n');

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("level err+"));
        assert!(out.contains("search: n"));
    }
}
