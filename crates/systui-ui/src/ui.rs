//! Rendering of the application frame. This is a pure function of [`App`], which
//! makes it testable headlessly with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap};
use systui_collectors::{Disk, Process, SystemSnapshot};

use crate::app::{App, ProcessSort, Tab, ViewState};

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

    let mode = Line::from(format!("mode: {} ", app.mode)).alignment(Alignment::Right);
    frame.render_widget(
        Paragraph::new(mode).style(Style::new().fg(app.theme.dim)),
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
        _ => render_message(frame, app, tab, inner),
    }
}

fn render_logs(frame: &mut Frame, app: &App, area: Rect) {
    if app.logs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No recent error logs.",
                Style::new().fg(app.theme.ok),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let priority_color = |entry: &systui_collectors::LogEntry| {
        if entry.priority <= 3 {
            app.theme.danger
        } else if entry.priority == 4 {
            app.theme.warn
        } else {
            app.theme.dim
        }
    };

    let header = Row::new(["TIME", "PRIO", "SOURCE", "MESSAGE"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = app.logs.iter().map(|e| {
        Row::new([
            Cell::from(e.time.clone()),
            Cell::from(Span::styled(
                e.priority_label().to_owned(),
                Style::new().fg(priority_color(e)),
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
    frame.render_widget(table, area);
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
    let body = app.failed_units.iter().map(|u| {
        Row::new([
            Cell::from(Span::styled(
                u.unit.clone(),
                Style::new().fg(app.theme.danger),
            )),
            Cell::from(u.active.clone()),
            Cell::from(u.sub.clone()),
            Cell::from(u.description.clone()),
        ])
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

    if app.processes.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No process data.",
                Style::new().fg(app.theme.dim),
            )),
            rows[1],
        );
        return;
    }

    let mut procs: Vec<&Process> = app.processes.iter().collect();
    let key = |p: &Process| match app.process_sort {
        ProcessSort::Cpu => p.cpu_percent,
        ProcessSort::Mem => p.mem_percent,
    };
    procs.sort_by(|a, b| {
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let header = Row::new(["PID", "USER", "%CPU", "%MEM", "COMMAND"])
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD));
    let body = procs.iter().take(20).map(|p| {
        Row::new([
            p.pid.to_string(),
            p.user.clone(),
            format!("{:.1}", p.cpu_percent),
            format!("{:.1}", p.mem_percent),
            p.command.clone(),
        ])
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

    Text::from(lines)
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
    let hint = " r refresh | / search | a actions | ? help | q quit ";
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
        ("1–8", "jump to tab"),
        ("r", "refresh"),
        ("s", "sort processes by CPU/memory"),
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
                user: "root".to_owned(),
                cpu_percent: 0.1,
                mem_percent: 0.2,
                command: "systemd".to_owned(),
            },
            Process {
                pid: 3300,
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
        use systui_collectors::FailedUnit;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.failed_units = vec![FailedUnit {
            unit: "nginx.service".to_owned(),
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
        use systui_collectors::FailedUnit;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.failed_units = vec![FailedUnit {
            unit: "docker.service".to_owned(),
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
}
