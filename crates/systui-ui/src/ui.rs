//! Rendering of the application frame. This is a pure function of [`App`], which
//! makes it testable headlessly with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Gauge, Paragraph, Row, Sparkline, Table, Wrap,
};
use regex::RegexBuilder;
use systui_collectors::{
    BindScope, Connection, Container, CronEntry, CronSource, DatabaseInstance, LogEntry,
    NetworkSnapshot, Protocol, ServiceUnit, SystemSnapshot, parse_schedule,
};
use systui_core::{Finding, ModuleId, Severity};

use crate::app::{ActionStage, App, InputMode, ProcessView, ServiceFilter, Tab, ViewState};
use crate::theme::Theme;
use crate::widgets::{labeled_gauge, severity_bars};

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, app: &App) {
    // Fill the whole frame with the theme background so the truecolor palette
    // covers gutters between panels.
    frame.render_widget(
        Block::default().style(Style::new().bg(app.theme.bg).fg(app.theme.fg)),
        frame.area(),
    );

    let rows = Layout::vertical([
        Constraint::Length(6), // top bar (ASCII wordmark + meta + status)
        Constraint::Length(1), // tabs
        Constraint::Min(0),    // body
        Constraint::Length(1), // status bar
    ])
    .split(frame.area());

    render_top_bar(frame, app, rows[0]);
    render_tabs(frame, app, rows[1]);
    render_content(frame, app, rows[2]);
    render_status_bar(frame, app, rows[3]);

    if app.show_help {
        render_help(frame, app);
    }
    if app.action.is_some() {
        render_action_modal(frame, app);
    }
    if let Some(builder) = &app.cron_builder {
        crate::cron_builder::render_cron_builder(frame, builder, &app.theme, app.now);
    }
    if let Some(draft) = &app.note_draft {
        render_note_input(frame, app, draft);
    }
}

/// Single-line input overlay for a new session note.
fn render_note_input(frame: &mut Frame, app: &App, draft: &str) {
    let t = app.theme;
    let area = centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(t.accent))
        .title(" New session note ");
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {draft}_"),
            Style::new().fg(t.fg_strong),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter save · Esc cancel",
            Style::new().fg(t.fg_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
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

/// ASCII block wordmark for "SYSTOOLS" — five rows, ~31 cols.
const WORDMARK: [&str; 5] = [
    "███ █ █ ███ ███ ███ ███ █   ███",
    "█   █ █ █    █  █ █ █ █ █   █  ",
    "███ ███ ███  █  █ █ █ █ █   ███",
    "  █  █    █  █  █ █ █ █ █     █",
    "███  █  ███  █  ███ ███ ███ ███",
];

/// Format an uptime in seconds as `47d 12h 09m`.
fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("{d}d {h:02}h {m:02}m")
    } else {
        format!("{h}h {m:02}m")
    }
}

/// The top bar: ASCII SYSTOOLS wordmark on the left, a host/os/kernel/uptime/load
/// meta grid in the middle, and a health/checks/caps/clock status block on the
/// right, divided by a bottom rule (spec §13; design `app.jsx` topbar).
fn render_top_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    // Bottom rule under the whole bar.
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::new().fg(t.border))
        .style(Style::new().bg(t.bg_elev));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::horizontal([
        Constraint::Length(34), // wordmark
        Constraint::Min(28),    // meta grid
        Constraint::Length(40), // status block
    ])
    .split(inner);

    // ── Wordmark ───────────────────────────────────────────────
    let logo: Vec<Line> = WORDMARK
        .iter()
        .map(|row| {
            Line::from(Span::styled(
                format!(" {row}"),
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(logo), cols[0]);

    // ── Meta grid ──────────────────────────────────────────────
    let snap = app.snapshot.as_ref();
    let os = snap
        .and_then(|s| s.os.clone())
        .unwrap_or_else(|| "—".to_owned());
    let kernel = snap.map(|s| s.kernel.clone()).unwrap_or_default();
    let cores = snap.map(|s| s.cpu.cores).unwrap_or(0);
    let uptime = snap.map(|s| fmt_uptime(s.uptime_secs)).unwrap_or_default();
    let load = snap
        .map(|s| {
            format!(
                "{:.2} · {:.2} · {:.2}",
                s.load.one, s.load.five, s.load.fifteen
            )
        })
        .unwrap_or_default();
    let host = snap
        .map(|s| s.hostname.clone())
        .unwrap_or_else(|| app.host_label.clone());

    let k = |s: &str| Span::styled(format!("{s:<7}"), Style::new().fg(t.fg_dim));
    let v = |s: String| Span::styled(s, Style::new().fg(t.fg));
    let vhi = |s: String| {
        Span::styled(
            s,
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
        )
    };
    let meta = vec![
        Line::from(vec![k("host"), vhi(host)]),
        Line::from(vec![k("os"), v(os)]),
        Line::from(vec![k("kernel"), v(kernel)]),
        Line::from(vec![k("uptime"), v(uptime)]),
        Line::from(vec![
            k("load"),
            v(load),
            Span::styled(format!("   {cores}c"), Style::new().fg(t.fg_muted)),
        ]),
    ];
    frame.render_widget(Paragraph::new(meta), cols[1]);

    // ── Status block ───────────────────────────────────────────
    let pill = |s: String, fg, bg| {
        Span::styled(
            format!(" {s} "),
            Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        )
    };
    let mut status: Vec<Line> = Vec::new();

    if let Some(h) = &app.health {
        let color = score_color(app, h.score);
        let issues = h.checks.len();
        let checks_span = if issues == 0 {
            Span::styled("✓ all clear", Style::new().fg(t.accent))
        } else {
            Span::styled(format!("{issues} ✗"), Style::new().fg(t.critical))
        };
        status.push(Line::from(vec![
            Span::styled("health ", Style::new().fg(t.fg_dim)),
            pill(format!("{}/100", h.score), t.bg, color),
            Span::styled("  checks ", Style::new().fg(t.fg_dim)),
            checks_span,
        ]));
    }

    status.push(Line::from(Span::styled(
        app.now.format("%Y-%m-%d %H:%M:%S").to_string(),
        Style::new().fg(t.fg_muted),
    )));

    let (badge_text, badge_color) = mode_badge(t, app.mode);
    let caps = app
        .capabilities
        .as_ref()
        .map(|c| c.label())
        .unwrap_or_else(|| "—".to_owned());
    let mut line3 = vec![
        Span::styled("caps ", Style::new().fg(t.fg_dim)),
        Span::styled(format!("{caps}  "), Style::new().fg(t.low)),
        pill(badge_text.to_owned(), t.bg, badge_color),
    ];
    if app.refreshing {
        line3.push(Span::styled(" ⟳", Style::new().fg(t.accent)));
    }
    status.push(Line::from(line3));
    frame.render_widget(Paragraph::new(status).alignment(Alignment::Left), cols[2]);
}

/// Badge label and color for an execution mode.
fn mode_badge(
    theme: &Theme,
    mode: systui_core::ExecutionMode,
) -> (&'static str, ratatui::style::Color) {
    use systui_core::ExecutionMode;
    match mode {
        ExecutionMode::ReadOnly => ("READ-ONLY", theme.accent),
        ExecutionMode::SafeActions => ("SAFE", theme.high),
        ExecutionMode::Privileged => ("PRIVILEGED", theme.critical),
    }
}

/// Color for a 0–100 score: green/amber/red by band.
fn score_color(app: &App, score: u8) -> ratatui::style::Color {
    if score >= 80 {
        app.theme.accent
    } else if score >= 50 {
        app.theme.high
    } else {
        app.theme.critical
    }
}

/// Numbered tab bar with key chips and per-tab count badges (design `app.jsx`).
fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    frame.render_widget(Block::default().style(Style::new().bg(t.bg_elev)), area);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let active = i == app.active_tab;
        let key = if i < 9 { (b'1' + i as u8) as char } else { '0' };

        // Key chip: a small bordered-looking cell.
        let (key_fg, key_bg) = if active {
            (t.bg, t.accent)
        } else {
            (t.fg_dim, t.bg_hover)
        };
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new().fg(key_fg).bg(key_bg).add_modifier(Modifier::BOLD),
        ));

        let name_style = if active {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.fg_muted)
        };
        spans.push(Span::styled(tab.title().to_lowercase(), name_style));

        if let Some((count, color)) = tab_badge(app, *tab) {
            spans.push(Span::styled(
                format!(" {count} "),
                Style::new().fg(t.bg).bg(color).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(" ", Style::new().fg(t.fg_dim)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The "needs attention" count badge for a tab, with its color. `None` hides it.
fn tab_badge(app: &App, tab: Tab) -> Option<(usize, ratatui::style::Color)> {
    let module_findings = |module| app.findings.iter().filter(|f| f.module == module).count();
    match tab {
        Tab::Dashboard => app
            .health
            .as_ref()
            .map(|h| h.checks.len())
            .filter(|n| *n > 0)
            .map(|n| (n, app.theme.high)),
        Tab::Services => {
            (!app.failed_units.is_empty()).then_some((app.failed_units.len(), app.theme.critical))
        }
        Tab::Logs => {
            let errors = app.logs.iter().filter(|e| e.is_error()).count();
            (errors > 0).then_some((errors, app.theme.critical))
        }
        Tab::Network => {
            let risky = app.risky_exposure_count();
            (risky > 0).then_some((risky, app.theme.critical))
        }
        Tab::Docker => {
            let n = module_findings(ModuleId::Docker);
            (n > 0).then_some((n, app.theme.high))
        }
        Tab::Crons => {
            let n = module_findings(ModuleId::Crons);
            (n > 0).then_some((n, app.theme.high))
        }
        Tab::Security => {
            let [crit, high, ..] = app.finding_counts();
            let total: usize = app.finding_counts().iter().sum();
            let color = if crit > 0 || high > 0 {
                app.theme.critical
            } else {
                app.theme.high
            };
            (total > 0).then_some((total, color))
        }
        _ => None,
    }
}

fn render_content(frame: &mut Frame, app: &App, area: Rect) {
    let tab = app.current_tab();
    // No outer titled border anymore — the active tab is shown in the tab bar
    // and each screen draws its own panels. Pad the body for breathing room.
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });

    match (&app.view_state, &app.snapshot, tab) {
        (ViewState::Ready, Some(snap), Tab::Dashboard) => {
            render_dashboard(frame, app, snap, inner);
        }
        (ViewState::Ready, Some(snap), Tab::System) => render_system(frame, app, snap, inner),
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

    // A bar chart of events by level sits above the tail on every view.
    let bands = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    render_log_level_chart(frame, app, &filtered, bands[0]);
    let area = bands[1];

    // Clean by default: the live tail full-width. Dense adds the analysis rail
    // (error fingerprints, sources, saved searches).
    if !app.dense {
        render_log_tail(frame, app, &filtered, area);
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)]).split(area);
    render_log_tail(frame, app, &filtered, cols[0]);
    let right = Layout::vertical([
        Constraint::Percentage(46),
        Constraint::Percentage(30),
        Constraint::Percentage(24),
    ])
    .split(cols[1]);
    render_log_fingerprints(frame, app, &filtered, right[0]);
    render_log_sources(frame, app, &filtered, right[1]);
    render_saved_searches(frame, app, right[2]);
}

/// A bar chart of the visible log lines grouped by severity level.
fn render_log_level_chart(frame: &mut Frame, app: &App, filtered: &[&LogEntry], area: Rect) {
    let t = app.theme;
    // priority: 0–3 error, 4 warning, 5 notice, 6 info, 7 debug.
    let mut buckets = [0u64; 5]; // ERR, WARN, NOTICE, INFO, DEBUG
    for e in filtered {
        let idx = match e.priority {
            0..=3 => 0,
            4 => 1,
            5 => 2,
            6 => 3,
            _ => 4,
        };
        buckets[idx] += 1;
    }
    let labels = ["ERR", "WARN", "NOTICE", "INFO", "DEBUG"];
    let colors = [t.critical, t.high, t.medium, t.accent, t.fg_muted];
    let items: Vec<(String, u64, ratatui::style::Color)> = labels
        .iter()
        .zip(buckets.iter().zip(colors.iter()))
        .map(|(label, (count, color))| ((*label).to_owned(), *count, *color))
        .collect();
    crate::widgets::bar_chart(frame, &t, area, "Lines by level", 7, &items);
}

/// Persisted log searches. `S` saves the current query; ↑/↓ select and Enter
/// applies one.
fn render_saved_searches(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Saved searches", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let searches = &app.state.saved_searches;
    if searches.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "none — S saves the current search",
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = searches
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let selected = i == app.saved_search_selected;
            let marker = if selected { "→ " } else { "  " };
            let style = if selected {
                Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(t.fg_muted)
            };
            Line::from(Span::styled(format!("{marker}{}", s.query), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_log_tail(frame: &mut Frame, app: &App, filtered: &[&LogEntry], area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "journalctl · live", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    render_log_filter_bar(frame, app, rows[0]);

    if filtered.is_empty() {
        let msg = if app.logs.is_empty() {
            "no logs for this filter"
        } else {
            "no matches"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(t.fg_dim)))
                .alignment(Alignment::Center),
            rows[1],
        );
        return;
    }

    let header = Row::new(["TIME", "LVL", "SOURCE", "MESSAGE"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = filtered.iter().map(|e| {
        let color = log_priority_color(app, e);
        Row::new([
            Cell::from(Span::styled(e.time.clone(), Style::new().fg(t.fg_dim))),
            Cell::from(Span::styled(
                e.priority_label().to_owned(),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            )),
            Cell::from(Span::styled(
                e.identifier.clone(),
                Style::new().fg(t.fg_muted),
            )),
            Cell::from(Span::styled(e.message.clone(), Style::new().fg(t.fg))),
        ])
    });
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(16),
        Constraint::Min(10),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        rows[1],
    );
}

fn render_log_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let dim = Style::new().fg(t.fg_dim);
    let accent = Style::new().fg(t.accent);
    let mut spans = vec![
        Span::styled("level ", Style::new().fg(t.fg_muted)),
        Span::styled(
            format!(" {} ", app.log_level_label()),
            Style::new()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  window {}  ", app.log_window_label()),
            Style::new().fg(t.fg_muted),
        ),
    ];
    if app.input_mode == InputMode::Search {
        spans.push(Span::styled(format!("search: {}_", app.log_search), accent));
    } else if !app.log_search.is_empty() {
        spans.push(Span::styled(format!("/{}", app.log_search), accent));
    } else {
        spans.push(Span::styled("l level · t window · / search", dim));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// One grouped error/warning pattern aggregated from the visible log buffer.
struct LogFingerprint {
    sample: String,
    count: usize,
    first: String,
    last: String,
    worst: u8,
}

/// Collapse a message into a fingerprint key: digit runs become `#`, lowercased,
/// whitespace normalised — so lines differing only in numbers group together.
fn normalize_log(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut prev_digit = false;
    for ch in msg.chars() {
        if ch.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            prev_digit = false;
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Group error/warning lines from the buffer into fingerprints, worst-count first.
fn log_fingerprints(entries: &[&LogEntry]) -> Vec<LogFingerprint> {
    use std::collections::HashMap;
    let mut map: HashMap<String, LogFingerprint> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| e.priority <= 4) {
        let key = format!("{}|{}", e.identifier, normalize_log(&e.message));
        let fp = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            LogFingerprint {
                sample: format!("{}: {}", e.identifier, e.message),
                count: 0,
                first: e.time.clone(),
                last: e.time.clone(),
                worst: e.priority,
            }
        });
        fp.count += 1;
        fp.last = e.time.clone();
        fp.worst = fp.worst.min(e.priority);
    }
    let mut v: Vec<LogFingerprint> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    v.sort_by_key(|fp| std::cmp::Reverse(fp.count));
    v
}

fn render_log_fingerprints(frame: &mut Frame, app: &App, filtered: &[&LogEntry], area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Error fingerprints", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let fps = log_fingerprints(filtered);
    if fps.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no errors", Style::new().fg(t.accent))),
            inner,
        );
        return;
    }

    let max = (inner.height as usize / 2).max(1);
    let mut lines: Vec<Line> = Vec::new();
    for fp in fps.iter().take(max) {
        let color = if fp.worst <= 3 { t.critical } else { t.high };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}x ", fp.count),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(fp.sample.clone(), Style::new().fg(t.fg_strong)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("   first {} · last {}", fp.first, fp.last),
            Style::new().fg(t.fg_dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_log_sources(frame: &mut Frame, app: &App, filtered: &[&LogEntry], area: Rect) {
    use std::collections::HashMap;
    let t = app.theme;
    let block = panel_block(&t, "Sources", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for e in filtered {
        let key = e.identifier.as_str();
        if !counts.contains_key(key) {
            order.push(key);
        }
        *counts.entry(key).or_insert(0) += 1;
    }
    if order.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no sources", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    order.sort_by_key(|s| std::cmp::Reverse(counts[*s]));

    let max = (inner.height as usize).max(1);
    let lines: Vec<Line> = order
        .iter()
        .take(max)
        .map(|src| {
            Line::from(vec![
                Span::styled(format!("{:>4} ", counts[src]), Style::new().fg(t.fg_muted)),
                Span::styled((*src).to_owned(), Style::new().fg(t.fg)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
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
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    render_service_filter_bar(frame, app, rows[0]);

    // Clean by default: the unit table full-width. Dense adds the detail pane.
    if app.dense {
        let cols = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(rows[1]);
        render_service_list(frame, app, cols[0]);
        render_service_detail(frame, app, cols[1]);
    } else {
        render_service_list(frame, app, rows[1]);
    }
}

fn render_service_filter_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let mut spans = vec![Span::styled("filter ", Style::new().fg(t.fg_muted))];
    for filter in ServiceFilter::ALL {
        let style = if filter == app.service_filter {
            Style::new()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.fg_dim)
        };
        spans.push(Span::styled(format!(" {} ", filter.label()), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("· f cycle", Style::new().fg(t.fg_dim)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Dot colour for a unit's state: failed=critical, running=accent, inactive=dim,
/// anything else (activating/reloading/…) = amber.
fn unit_dot_color(t: &Theme, u: &ServiceUnit) -> ratatui::style::Color {
    if u.is_failed() {
        t.critical
    } else if u.sub == "running" || u.active == "active" {
        t.accent
    } else if u.active == "inactive" {
        t.fg_dim
    } else {
        t.high
    }
}

fn render_service_list(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let units = app.visible_units();
    let block = panel_block(
        &t,
        &format!("systemd · {} {}", units.len(), app.service_filter.label()),
        app.domain_color(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if units.is_empty() {
        let showing_failed =
            app.service_filter == ServiceFilter::Failed || app.all_units.is_empty();
        let (msg, color) = if showing_failed {
            ("✓ no failed units — all healthy", t.accent)
        } else {
            ("no units match this filter", t.fg_dim)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(color))),
            inner,
        );
        return;
    }

    let header = Row::new(["", "UNIT", "ACTIVE", "SUB", "EN"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = units.iter().enumerate().map(|(i, u)| {
        let active_color = if u.is_failed() { t.critical } else { t.fg };
        let enabled = app.enabled_units.iter().any(|n| n == &u.name);
        let row = Row::new([
            Cell::from(Span::styled("●", Style::new().fg(unit_dot_color(&t, u)))),
            Cell::from(Span::styled(u.name.clone(), Style::new().fg(t.fg_strong))),
            Cell::from(Span::styled(
                u.active.clone(),
                Style::new().fg(active_color),
            )),
            Cell::from(Span::styled(u.sub.clone(), Style::new().fg(t.fg_muted))),
            Cell::from(Span::styled(
                if enabled { "on" } else { "" },
                Style::new().fg(t.accent),
            )),
        ]);
        if i == app.services_selected {
            row.style(Style::new().bg(t.bg_sel).add_modifier(Modifier::BOLD))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(1),
        Constraint::Min(16),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(3),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        inner,
    );
}

fn render_service_detail(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let units = app.visible_units();
    let Some(u) = units.get(app.services_selected).map(|u| (*u).clone()) else {
        frame.render_widget(
            Paragraph::new(Span::styled("no unit selected", Style::new().fg(t.fg_dim)))
                .block(panel_block(&t, "Unit", app.domain_color())),
            area,
        );
        return;
    };

    let block = panel_block(&t, &u.name, app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // The lazily-fetched detail is only trustworthy once it matches the current
    // selection; until then fall back to the list-level fields.
    let detail = app
        .selected_unit_detail
        .as_ref()
        .filter(|d| d.name == u.name);

    let rows = Layout::vertical([
        Constraint::Length(8), // identity fields
        Constraint::Length(5), // dependencies
        Constraint::Min(0),    // recent logs
    ])
    .split(inner);

    let field = |key: &str, value: String, color| {
        Line::from(vec![
            Span::styled(format!("{key:<9} "), Style::new().fg(t.fg_dim)),
            Span::styled(value, Style::new().fg(color)),
        ])
    };
    let enabled = detail
        .map(|d| d.unit_file_state.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if app.enabled_units.iter().any(|n| n == &u.name) {
                "enabled".to_owned()
            } else {
                "disabled".to_owned()
            }
        });
    let mut lines = vec![
        Line::from(Span::styled(
            u.description.clone(),
            Style::new().fg(t.fg_muted),
        )),
        field(
            "state",
            format!("{} ({})", u.active, u.sub),
            unit_dot_color(&t, &u),
        ),
        field("enabled", enabled, t.fg),
    ];
    match detail.and_then(|d| d.main_pid) {
        Some(pid) => lines.push(field(
            "main PID",
            format!("{pid}  (see Processes)"),
            t.fg_strong,
        )),
        None => lines.push(field("main PID", "—".to_owned(), t.fg_dim)),
    }
    if let Some(d) = detail {
        if !d.fragment_path.is_empty() {
            lines.push(field("file", d.fragment_path.clone(), t.fg_muted));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "fetching detail…",
            Style::new().fg(t.fg_dim),
        )));
    }
    let hint = if app.mode == systui_core::ExecutionMode::ReadOnly {
        "read-only — actions disabled"
    } else {
        "press a to act (restart / stop / …)"
    };
    lines.push(Line::from(Span::styled(hint, Style::new().fg(t.fg_dim))));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[0]);

    render_unit_dependencies(frame, app, rows[1]);
    render_unit_logs(frame, app, rows[2]);
}

/// Dependencies of the selected unit (`systemctl list-dependencies`).
fn render_unit_dependencies(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let header = Line::from(Span::styled(
        format!("Dependencies ({})", app.selected_unit_deps.len()),
        Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD),
    ));
    let mut lines = vec![header];
    if app.selected_unit_deps.is_empty() {
        lines.push(Line::from(Span::styled("none", Style::new().fg(t.fg_dim))));
    } else {
        let max = (area.height as usize).saturating_sub(1).max(1);
        for dep in app.selected_unit_deps.iter().take(max) {
            lines.push(Line::from(Span::styled(
                format!("  {dep}"),
                Style::new().fg(t.fg_muted),
            )));
        }
        let extra = app.selected_unit_deps.len().saturating_sub(max);
        if extra > 0 {
            lines.push(Line::from(Span::styled(
                format!("  +{extra} more"),
                Style::new().fg(t.fg_dim),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Recent journal lines for the selected unit (`journalctl -u`).
fn render_unit_logs(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let header = Line::from(Span::styled(
        "Recent logs",
        Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD),
    ));
    let mut lines = vec![header];
    if app.selected_unit_logs.is_empty() {
        lines.push(Line::from(Span::styled(
            "no recent journal entries",
            Style::new().fg(t.fg_dim),
        )));
    } else {
        let max = (area.height as usize).saturating_sub(1).max(1);
        for e in app.selected_unit_logs.iter().rev().take(max) {
            let color = log_priority_color(app, e);
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", e.time), Style::new().fg(t.fg_dim)),
                Span::styled(
                    format!("{:<5} ", e.priority_label()),
                    Style::new().fg(color),
                ),
                Span::styled(e.message.clone(), Style::new().fg(t.fg)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_processes(frame: &mut Frame, app: &App, area: Rect) {
    // A CPU bar chart of the top consumers sits above the table on every view.
    let rows = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    render_process_cpu_chart(frame, app, rows[0]);

    // Clean by default: the process table full-width. Dense adds the detail pane.
    if !app.dense {
        render_process_list(frame, app, rows[1]);
        return;
    }
    let cols =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(rows[1]);
    render_process_list(frame, app, cols[0]);
    let right = Layout::vertical([Constraint::Percentage(58), Constraint::Percentage(42)]).split(cols[1]);
    render_process_detail(frame, app, right[0]);
    render_process_treemap(frame, app, right[1]);
}

/// PROCESSES · RSS TREEMAP: the heaviest processes by resident memory as a
/// squarified treemap. The selected process is accented.
fn render_process_treemap(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "top by RSS · treemap", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut procs: Vec<_> = app.processes.iter().collect();
    procs.sort_by_key(|p| std::cmp::Reverse(p.rss_kb));
    // Domain hues cycled across tiles so adjacent processes are distinguishable.
    let palette = [t.teal, t.cyan, t.blue, t.indigo, t.violet, t.magenta, t.rose, t.high];
    let items: Vec<(String, String, u64, ratatui::style::Color)> = procs
        .iter()
        .filter(|p| p.rss_kb > 0)
        .take(12)
        .enumerate()
        .map(|(rank, p)| {
            (
                short_cmd(&p.command),
                human_kb(p.rss_kb),
                p.rss_kb,
                palette[rank % palette.len()],
            )
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no RSS data", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    crate::widgets::treemap(frame, &t, inner, &items);
}

/// A bar chart of the top processes by CPU — the heaviest consumers at a glance.
fn render_process_cpu_chart(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let mut procs: Vec<&systui_collectors::Process> = app.processes.iter().collect();
    procs.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    let items: Vec<(String, u64, ratatui::style::Color)> = procs
        .iter()
        .take(10)
        .map(|p| {
            let name: String = p.command.split_whitespace().next().unwrap_or(&p.command)
                .rsplit('/')
                .next()
                .unwrap_or("?")
                .chars()
                .take(8)
                .collect();
            (name, p.cpu_percent.round() as u64, usage_color(&t, p.cpu_percent))
        })
        .collect();
    if items.is_empty() {
        let block = panel_block(&t, "Top CPU", app.domain_color());
        frame.render_widget(block, area);
        return;
    }
    crate::widgets::bar_chart(frame, &t, area, "Top CPU %", 8, &items);
}

/// Left column: a one-line hint plus the process table (flat list or tree),
/// scrolled so the selected row stays visible.
fn render_process_list(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    let view = if app.process_view == ProcessView::Tree {
        "tree"
    } else {
        "list"
    };
    let hint = Line::from(vec![
        Span::styled(
            format!("sorted by {} · {view} ", app.process_sort.label()),
            Style::new().fg(t.text),
        ),
        Span::styled("(s sort · t list/tree)", Style::new().fg(t.dim)),
    ]);
    frame.render_widget(Paragraph::new(hint), rows[0]);
    let body_area = rows[1];

    if app.processes.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("No process data.", Style::new().fg(t.dim))),
            body_area,
        );
        return;
    }

    let procs = app.process_rows();
    let tree = app.process_view == ProcessView::Tree;
    let depths: Vec<u16> = procs.iter().map(|(d, _)| *d).collect();
    // Scroll a window of rows so the selection is always on screen.
    let capacity = (body_area.height as usize).saturating_sub(1).max(1); // minus header
    let start = app
        .processes_selected
        .saturating_sub(capacity.saturating_sub(1));
    let header = Row::new(["PID", "USER", "%CPU", "%MEM", "COMMAND"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = procs
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
        .map(|(i, (_, p))| {
            let command = if tree {
                format!("{}{}", tree_prefix(&depths, i), p.command)
            } else {
                p.command.clone()
            };
            let row = Row::new([
                p.pid.to_string(),
                p.user.clone(),
                format!("{:.1}", p.cpu_percent),
                format!("{:.1}", p.mem_percent),
                command,
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
        .column_spacing(1)
        .style(Style::new().fg(t.fg));
    frame.render_widget(table, body_area);
}

/// Tree connectors for row `i` of a pre-order, depth-annotated process list:
/// `│  ` for ancestor levels that continue, `├─ `/`└─ ` for the node itself.
fn tree_prefix(depths: &[u16], i: usize) -> String {
    let d = depths[i];
    if d == 0 {
        return String::new();
    }
    // At a given level, the relevant ancestor/self has a following sibling iff,
    // scanning forward, we meet that exact depth before rising above it.
    let has_following = |level: u16| {
        depths[i + 1..]
            .iter()
            .take_while(|&&dj| dj >= level)
            .any(|&dj| dj == level)
    };
    let mut prefix = String::with_capacity(d as usize * 3);
    for level in 1..=d {
        let following = has_following(level);
        if level == d {
            prefix.push_str(if following { "├─ " } else { "└─ " });
        } else {
            prefix.push_str(if following { "│  " } else { "   " });
        }
    }
    prefix
}

/// Right column: detail for the selected process, derived from the gathered
/// process list (PID/PPID/parent, owner, CPU/RAM, command) — real data only.
fn render_process_detail(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Process", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(p) = app.selected_process() else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no process selected",
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    };

    let parent = app
        .processes
        .iter()
        .find(|c| c.pid == p.ppid)
        .map(|c| format!("{} ({})", c.command, c.pid))
        .unwrap_or_else(|| p.ppid.to_string());

    let mut lines = vec![
        Line::from(Span::styled(
            p.command.clone(),
            Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        label_value(app, "PID", &p.pid.to_string()),
        label_value(app, "Parent", &parent),
        label_value(app, "User", &p.user),
        label_value(app, "CPU", &format!("{:.1}%", p.cpu_percent)),
        label_value(app, "Memory", &format!("{:.1}%", p.mem_percent)),
    ];

    let children = app.processes.iter().filter(|c| c.ppid == p.pid).count();
    if children > 0 {
        lines.push(label_value(app, "Children", &children.to_string()));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "a to signal (SIGTERM) · ↑↓ to select",
        Style::new().fg(t.fg_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// A bar chart of listening exposures grouped by severity.
fn render_exposure_severity_chart(frame: &mut Frame, app: &App, area: Rect) {
    let mut counts = [0u64; 5];
    for e in &app.exposures {
        let idx = match e.severity {
            Severity::Critical => 0,
            Severity::High => 1,
            Severity::Medium => 2,
            Severity::Low => 3,
            Severity::Info => 4,
        };
        counts[idx] += 1;
    }
    let t = app.theme;
    let labels = ["CRIT", "HIGH", "MED", "LOW", "INFO"];
    let colors = [t.critical, t.high, t.medium, t.low, t.fg_muted];
    let items: Vec<(String, u64, ratatui::style::Color)> = labels
        .iter()
        .zip(counts.iter().zip(colors.iter()))
        .map(|(label, (count, color))| ((*label).to_owned(), *count, *color))
        .collect();
    crate::widgets::bar_chart(frame, &t, area, "Exposures by severity", 6, &items);
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

/// A finding's severity badge + title on one line (used by the Databases tab).
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
            Style::new()
                .fg(app.theme.fg_strong)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn render_network(frame: &mut Frame, app: &App, area: Rect) {
    let Some(net) = &app.network else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No network data — `ip`/`ss` unavailable.",
                Style::new().fg(app.theme.fg_dim),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    };

    // A bar chart of exposures by severity sits above the map on every view.
    let bands = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    render_exposure_severity_chart(frame, app, bands[0]);
    let area = bands[1];

    // Clean by default: the exposure map full-width — the security-relevant
    // "what is listening". Dense adds connectivity, interfaces, DNS/routes,
    // connections and firewall panels.
    if !app.dense {
        render_exposure_panel(frame, app, area);
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);
    let left = Layout::vertical([Constraint::Min(0), Constraint::Length(7)]).split(cols[0]);
    render_exposure_panel(frame, app, left[0]);
    render_connectivity_panel(frame, app, left[1]);
    let right = Layout::vertical([
        Constraint::Percentage(26),
        Constraint::Percentage(20),
        Constraint::Percentage(24),
        Constraint::Percentage(30),
    ])
    .split(cols[1]);
    render_net_interfaces(frame, app, net, right[0]);
    render_net_dns_routes(frame, app, net, right[1]);
    render_net_connections(frame, app, net, right[2]);
    render_firewall_panel(frame, app, right[3]);
}

/// Firewall summary: the active manager/engine, table & chain names and the rule
/// count, plus any caveats (e.g. that the listing needs privilege).
fn render_firewall_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let fw = &app.firewall;
    let badge = if fw.active {
        fw.backend.as_str()
    } else {
        "off"
    };
    let block = panel_block(&t, "Firewall", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (state_label, state_color) = if fw.active {
        ("active", t.accent)
    } else if fw.backend == "none" {
        ("none", t.fg_dim)
    } else {
        ("inactive", t.high)
    };
    let field = |key: &str, value: String, color| {
        Line::from(vec![
            Span::styled(format!("{key:<8} "), Style::new().fg(t.fg_dim)),
            Span::styled(value, Style::new().fg(color)),
        ])
    };
    let mut lines = vec![field(
        "backend",
        format!("{} ({})", badge, state_label),
        state_color,
    )];
    if !fw.tables.is_empty() {
        lines.push(field("tables", fw.tables.join(" · "), t.fg_muted));
    }
    if !fw.chains.is_empty() {
        lines.push(field("chains", fw.chains.join(" · "), t.fg_muted));
    }
    if fw.rule_count > 0 {
        lines.push(field(
            "rules",
            format!("{} active", fw.rule_count),
            t.fg_strong,
        ));
    }
    for note in &fw.notes {
        lines.push(Line::from(Span::styled(
            format!("! {note}"),
            Style::new().fg(t.high),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// On-demand reachability probes against the host's gateway and resolvers.
/// Empty until the user runs them with `c` (they are active probes, not part of
/// the passive refresh).
fn render_connectivity_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let title = if app.connectivity_running {
        "Connectivity tests · running…"
    } else {
        "Connectivity tests"
    };
    let block = panel_block(&t, title, app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.connectivity.is_empty() {
        let msg = if app.connectivity_running {
            "probing gateway · DNS…"
        } else {
            "press c to test reachability (gateway · DNS)"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    let lines: Vec<Line> = app
        .connectivity
        .iter()
        .map(|r| {
            let (mark, color) = if r.reachable {
                ("ok ", t.accent)
            } else {
                ("fail", t.critical)
            };
            Line::from(vec![
                Span::styled(format!("→ {:<15} ", r.target), Style::new().fg(t.fg_strong)),
                Span::styled(format!("{mark} "), Style::new().fg(color)),
                Span::styled(format!("{:<8} ", r.label), Style::new().fg(t.fg_dim)),
                Span::styled(r.detail.clone(), Style::new().fg(t.fg_muted)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The ranked exposure map: one row per listening socket, address colour-coded
/// by bind scope and a severity RISK badge (spec §14).
/// A severity's rung on the 4-step risk ladder (Info=0 … Critical=4).
fn severity_rung(s: Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Low => 1,
        Severity::Medium => 2,
        Severity::High => 3,
        Severity::Critical => 4,
    }
}

/// The design's 4-step risk ladder: filled rungs colored low→crit (green/cyan/
/// amber/red), empty rungs dim. Returns spans for inline use in a table cell.
fn risk_ladder(theme: &Theme, severity: Severity) -> Vec<Span<'static>> {
    let lvl = severity_rung(severity);
    let tiers = [theme.accent, theme.low, theme.high, theme.critical];
    (0..4u8)
        .map(|i| {
            if i < lvl {
                Span::styled("▰", Style::new().fg(tiers[i as usize]))
            } else {
                Span::styled("▱", Style::new().fg(theme.fg_dim))
            }
        })
        .collect()
}

fn render_exposure_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(
        &t,
        &format!("Exposure map · {} listening", app.exposures.len()),
        app.domain_color(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.exposures.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no listening sockets",
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    }

    let header = Row::new(["PROTO", "ADDRESS", "PROCESS", "SERVICE", "RISK"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = app.exposures.iter().map(|e| {
        let proto = format!("{:?}", e.listener.protocol).to_lowercase();
        let ip = &e.listener.local_ip;
        let addr_color = if ip == "0.0.0.0" || ip == "::" {
            t.high
        } else if e.scope == BindScope::Loopback {
            t.fg_dim
        } else {
            t.fg
        };
        let owner = match (&e.listener.process, &e.listener.unit) {
            (Some(p), Some(unit)) => format!("{} ({unit})", p.name),
            (Some(p), None) => p.name.clone(),
            (None, _) => "—".to_owned(),
        };
        let service = e.sensitive_service.unwrap_or("—").to_owned();
        Row::new([
            Cell::from(Span::styled(proto, Style::new().fg(t.fg_muted))),
            Cell::from(Span::styled(
                format!("{ip}:{}", e.listener.port),
                Style::new().fg(addr_color),
            )),
            Cell::from(Span::styled(owner, Style::new().fg(t.fg))),
            Cell::from(Span::styled(service, Style::new().fg(t.fg_muted))),
            Cell::from(Line::from(risk_ladder(&t, e.severity))),
        ])
    });
    let widths = [
        Constraint::Length(5),
        Constraint::Length(20),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(6),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        inner,
    );
}

fn render_net_interfaces(frame: &mut Frame, app: &App, net: &NetworkSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Interfaces", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if net.interfaces.is_empty() {
        lines.push(Line::from(Span::styled("none", Style::new().fg(t.fg_dim))));
    }
    for iface in &net.interfaces {
        let up = iface.state.eq_ignore_ascii_case("up");
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<10} ", iface.name),
                Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                iface.state.clone(),
                Style::new().fg(if up { t.accent } else { t.fg_dim }),
            ),
        ]));
        if !iface.addrs.is_empty() {
            let addrs = iface
                .addrs
                .iter()
                .map(|a| format!("{}/{}", a.ip, a.prefix_len))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(Line::from(Span::styled(
                format!("  {addrs}"),
                Style::new().fg(t.fg_muted),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_net_dns_routes(frame: &mut Frame, app: &App, net: &NetworkSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "DNS · routes", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let gateways: Vec<String> = net
        .routes
        .iter()
        .filter(|r| r.dst == "default")
        .filter_map(|r| r.gateway.clone().map(|g| format!("{g} via {}", r.dev)))
        .collect();
    let field = |key: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{key:<8} "), Style::new().fg(t.fg_dim)),
            Span::styled(value, Style::new().fg(t.fg)),
        ])
    };
    let lines = vec![
        field(
            "gateway",
            if gateways.is_empty() {
                "none".to_owned()
            } else {
                gateways.join(", ")
            },
        ),
        field(
            "dns",
            if net.dns.nameservers.is_empty() {
                "none".to_owned()
            } else {
                net.dns.nameservers.join(", ")
            },
        ),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_net_connections(frame: &mut Frame, app: &App, net: &NetworkSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Connections", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let counts = net.connection_state_counts();
    if counts.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no active connections",
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    }

    // Compact one-line summary by state, then the real established connections.
    let summary = counts
        .iter()
        .map(|(state, n)| format!("{} {n}", state.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut lines = vec![Line::from(Span::styled(
        summary,
        Style::new().fg(t.fg_muted),
    ))];

    let established: Vec<&Connection> = net
        .connections
        .iter()
        .filter(|c| c.state.starts_with("ESTAB") && is_real_peer(&c.peer_ip))
        .collect();
    let max = (inner.height as usize).saturating_sub(1);
    if established.is_empty() {
        lines.push(Line::from(Span::styled(
            "no established peers",
            Style::new().fg(t.fg_dim),
        )));
    } else {
        for c in established.iter().take(max) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(":{:<5} ", c.local_port),
                    Style::new().fg(t.fg_muted),
                ),
                Span::styled("⇄ ", Style::new().fg(t.fg_dim)),
                Span::styled(
                    format!("{}:{}", c.peer_ip, c.peer_port),
                    Style::new().fg(t.fg),
                ),
            ]));
        }
        let extra = established.len().saturating_sub(max);
        if extra > 0 {
            lines.push(Line::from(Span::styled(
                format!("+{extra} more established"),
                Style::new().fg(t.fg_dim),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Whether a peer address is a real remote endpoint worth listing (not a
/// wildcard/empty placeholder from `ss`).
fn is_real_peer(ip: &str) -> bool {
    !ip.is_empty() && ip != "*" && ip != "0.0.0.0" && ip != "::" && ip != "[::]"
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

    // A bar chart of per-container CPU sits above the table on every view.
    let bands = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    render_container_cpu_chart(frame, app, bands[0]);
    let area = bands[1];

    // Clean by default: the container table full-height. Dense adds the risk
    // checks, the selected-container detail, compose projects and image hygiene.
    if !app.dense {
        render_container_table(frame, app, area);
        return;
    }

    let rows = Layout::vertical([
        Constraint::Percentage(44),
        Constraint::Percentage(34),
        Constraint::Percentage(22),
    ])
    .split(area);
    render_container_table(frame, app, rows[0]);
    let mid =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).split(rows[1]);
    render_docker_risks(frame, app, mid[0]);
    render_container_detail(frame, app, mid[1]);
    let bottom =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).split(rows[2]);
    render_compose_projects(frame, app, bottom[0]);
    render_image_hygiene(frame, app, bottom[1]);
}

/// A bar chart of per-container CPU usage from `docker stats`.
fn render_container_cpu_chart(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    if app.container_stats.is_empty() {
        let block = panel_block(&t, "Container CPU %", app.domain_color());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled("no stats", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    let mut stats: Vec<&systui_collectors::ContainerStats> = app.container_stats.iter().collect();
    stats.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    let items: Vec<(String, u64, ratatui::style::Color)> = stats
        .iter()
        .take(10)
        .map(|s| {
            let name: String = s.name.trim_start_matches('/').chars().take(10).collect();
            (name, s.cpu_percent.round() as u64, usage_color(&t, s.cpu_percent))
        })
        .collect();
    crate::widgets::bar_chart(frame, &t, area, "Container CPU %", 10, &items);
}

/// Discovered Compose projects (`docker compose ls`): name, service count and
/// the config file backing each. Omitted-looking (an "(none)" line) when the
/// Compose plugin is absent or no projects exist.
fn render_compose_projects(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Compose projects", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.compose_projects.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("none discovered", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for p in &app.compose_projects {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<16} ", p.name),
                Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD),
            ),
            Span::styled(p.config_files.clone(), Style::new().fg(t.fg_dim)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {} services · {}", p.service_count, p.status),
            Style::new().fg(t.fg_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Image-store summary (`docker system df` + dangling count) with a prune hint.
fn render_image_hygiene(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let h = &app.image_hygiene;
    let block = panel_block(&t, "Image hygiene", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if h.total_images == 0 && h.dangling == 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("no image data", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} images", h.total_images),
                Style::new().fg(t.fg_strong),
            ),
            Span::styled(
                format!(" · {} total", h.total_size),
                Style::new().fg(t.fg_muted),
            ),
        ]),
        Line::from(Span::styled(
            format!("{} dangling", h.dangling),
            Style::new().fg(if h.dangling > 0 { t.high } else { t.fg_muted }),
        )),
    ];
    if !h.reclaimable.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{} reclaimable", h.reclaimable),
            Style::new().fg(t.fg_muted),
        )));
    }
    let hint = if app.mode == systui_core::ExecutionMode::ReadOnly {
        Span::styled("read-only — prune disabled", Style::new().fg(t.fg_dim))
    } else if h.dangling > 0 {
        Span::styled("press p to prune dangling", Style::new().fg(t.accent))
    } else {
        Span::styled("nothing to prune", Style::new().fg(t.fg_dim))
    };
    lines.push(Line::from(hint));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_container_table(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(
        &t,
        &format!("docker ps · {} containers", app.containers.len()),
        app.domain_color(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let header = Row::new([
        "",
        "CONTAINER",
        "IMAGE",
        "STATE",
        "HEALTH",
        "CPU%",
        "MEM%",
        "",
    ])
    .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = app.containers.iter().enumerate().map(|(i, c)| {
        let dot_color = if c.is_running() {
            t.accent
        } else if c.state == "restarting" {
            t.high
        } else {
            t.fg_dim
        };
        let (health, health_color) = container_health(app, c);
        let stats = app
            .container_stats
            .iter()
            .find(|s| s.id == c.id || s.name == c.name);
        let cpu = stats
            .map(|s| format!("{:.1}", s.cpu_percent))
            .unwrap_or_default();
        let mem = stats
            .map(|s| format!("{:.1}", s.mem_percent))
            .unwrap_or_default();
        let risk = container_risk(app, c);
        let risk_cell = match risk {
            Some(sev) => Span::styled(
                " RISK ",
                Style::new()
                    .fg(t.bg)
                    .bg(t.severity(sev))
                    .add_modifier(Modifier::BOLD),
            ),
            None => Span::styled("ok", Style::new().fg(t.fg_dim)),
        };
        let row = Row::new([
            Cell::from(Span::styled("●", Style::new().fg(dot_color))),
            Cell::from(Span::styled(c.name.clone(), Style::new().fg(t.fg_strong))),
            Cell::from(Span::styled(c.image.clone(), Style::new().fg(t.fg_muted))),
            Cell::from(Span::styled(c.state.clone(), Style::new().fg(dot_color))),
            Cell::from(Span::styled(health, Style::new().fg(health_color))),
            Cell::from(Span::styled(cpu, Style::new().fg(t.fg))),
            Cell::from(Span::styled(mem, Style::new().fg(t.fg))),
            Cell::from(risk_cell),
        ]);
        if i == app.containers_selected {
            row.style(Style::new().bg(t.bg_sel).add_modifier(Modifier::BOLD))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(1),
        Constraint::Min(14),
        Constraint::Length(24),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(6),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        inner,
    );
}

/// The worst severity among the selected container's inspect-based risk checks.
fn container_risk(app: &App, c: &Container) -> Option<Severity> {
    let inspect = app.container_inspects.iter().find(|i| i.id == c.id)?;
    systui_security::check_container(inspect)
        .into_iter()
        .map(|f| f.severity)
        .max()
}

/// Risk-check side panel: the worst Docker findings across all containers.
fn render_docker_risks(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Risk checks", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let risks: Vec<&Finding> = app
        .findings
        .iter()
        .filter(|f| f.module == ModuleId::Docker)
        .collect();
    if risks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no risks", Style::new().fg(t.accent))),
            inner,
        );
        return;
    }

    let max = (inner.height as usize).max(1);
    let mut lines: Vec<Line> = Vec::new();
    for f in risks.iter().take(max) {
        let color = t.severity(f.severity);
        let sev = f.severity.to_string().to_uppercase();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{sev:<5} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(f.title.clone(), Style::new().fg(t.fg_strong)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn container_health(app: &App, c: &Container) -> (String, ratatui::style::Color) {
    use systui_collectors::ContainerHealth;
    match c.health {
        Some(ContainerHealth::Healthy) => ("healthy".to_owned(), app.theme.accent),
        Some(ContainerHealth::Unhealthy) => ("unhealthy".to_owned(), app.theme.critical),
        Some(ContainerHealth::Starting) => ("starting".to_owned(), app.theme.high),
        None => ("-".to_owned(), app.theme.fg_dim),
    }
}

fn render_container_detail(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let dim = Style::new().fg(t.fg_muted);
    let text_s = Style::new().fg(t.fg);

    let title = app
        .selected_container()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "Container".to_owned());
    let block = panel_block(&t, &title, app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(inspect) = app.selected_inspect() else {
        let name = app
            .selected_container()
            .map(|c| c.name.as_str())
            .unwrap_or("container");
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("no inspect data for {name}"),
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    };

    let mut lines = vec![Line::from(Span::styled(inspect.image.clone(), dim))];

    let mem = if inspect.memory_limit_bytes == 0 {
        "unlimited".to_owned()
    } else {
        human_kb(inspect.memory_limit_bytes / 1024)
    };
    let priv_color = if inspect.privileged { t.critical } else { t.fg };
    let retries = if inspect.max_retry_count > 0 {
        format!("{}/{}", inspect.restart_count, inspect.max_retry_count)
    } else {
        inspect.restart_count.to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("privileged ", dim),
        Span::styled(
            format!("{}", inspect.privileged),
            Style::new().fg(priv_color),
        ),
        Span::styled(
            format!(
                "  ·  restart {} ({} retries)  ·  mem {}",
                inspect.restart_policy, retries, mem
            ),
            text_s,
        ),
    ]));
    if !inspect.networks.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("net {}", inspect.networks.join(", ")),
            dim,
        )));
    }
    if !inspect.published_ports.is_empty() {
        let ports = inspect
            .published_ports
            .iter()
            .map(|p| {
                format!(
                    "{}:{}->{}/{}",
                    p.host_ip, p.host_port, p.container_port, p.protocol
                )
            })
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(Line::from(vec![
            Span::styled("ports ", dim),
            Span::styled(ports, text_s),
        ]));
    }

    if let Some(stats) = app
        .container_stats
        .iter()
        .find(|s| s.id == inspect.id || s.name == inspect.name)
    {
        lines.push(Line::from(Span::styled(
            format!(
                "cpu {:.1}% · mem {:.1}% ({})",
                stats.cpu_percent, stats.mem_percent, stats.mem_usage
            ),
            text_s,
        )));
    }

    if !inspect.mounts.is_empty() {
        lines.push(Line::from(Span::styled("mounts", dim)));
        for m in &inspect.mounts {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} -> {} ({})",
                    m.source,
                    m.destination,
                    if m.rw { "rw" } else { "ro" }
                ),
                Style::new().fg(t.fg_dim),
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

fn render_crons(frame: &mut Frame, app: &App, area: Rect) {
    // The 24h schedule timeline tops the tab (design `cron.jsx`); the scheduled
    // jobs table sits below. Dense adds the preview, timers and health panels.
    let timeline_h = (app.crons.len().min(8) as u16 + 3).clamp(4, 11);
    let rows = Layout::vertical([Constraint::Length(timeline_h), Constraint::Min(0)]).split(area);
    render_cron_timeline(frame, app, rows[0]);
    let body = rows[1];

    if !app.dense {
        render_cron_table(frame, app, body);
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).split(body);
    render_cron_table(frame, app, cols[0]);
    let right = Layout::vertical([
        Constraint::Percentage(42),
        Constraint::Percentage(30),
        Constraint::Percentage(28),
    ])
    .split(cols[1]);
    render_cron_preview(frame, app, right[0]);
    render_cron_timers_panel(frame, app, right[1]);
    render_cron_summary(frame, app, right[2]);
}

/// CRON · TIMELINE (LAST 24H): a per-job heatmap of when each job is *scheduled*
/// to run across the next 24 hours, derived from its real cron expression. (Cron
/// keeps no run history, so this is the schedule, not recorded executions.)
fn render_cron_timeline(frame: &mut Frame, app: &App, area: Rect) {
    use chrono::Timelike;
    let t = app.theme;
    let block = panel_block(&t, "cron · timeline (next 24h)", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.crons.is_empty() || inner.height < 2 {
        frame.render_widget(
            Paragraph::new(Span::styled("no cron jobs", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    const LABEL_W: usize = 14;
    // Header: hour ruler, two cols per hour to match the cells below.
    let mut header = format!("{:>w$} ", "hour →", w = LABEL_W);
    for h in 0..24 {
        header.push_str(&format!("{h:02}"));
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(header, Style::new().fg(t.fg_dim)))),
        Rect { height: 1, ..inner },
    );

    let end = app.now + chrono::Duration::hours(24);
    let body_area = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let max_rows = body_area.height as usize;
    let job_rows = Layout::vertical(vec![Constraint::Length(1); max_rows.max(1)]).split(body_area);

    for (e, row) in app.crons.iter().take(max_rows).zip(job_rows.iter()) {
        // Bucket scheduled runs by hour-of-day across the 24h window.
        let mut buckets = [0u32; 24];
        if let Ok(sched) = parse_schedule(&e.schedule) {
            let mut cursor = app.now;
            for _ in 0..4000 {
                match sched.next_after(cursor) {
                    Some(next) if next <= end => {
                        buckets[next.hour() as usize] += 1;
                        cursor = next;
                    }
                    _ => break,
                }
            }
        }
        let name = clip(&short_cmd(&e.command), LABEL_W);
        let mut spans = vec![Span::styled(
            format!("{name:>LABEL_W$} "),
            Style::new().fg(if e.enabled { t.fg_muted } else { t.fg_dim }),
        )];
        for count in buckets {
            let (glyph, color) = match count {
                0 => ("··", t.fg_dim),
                1 => ("▓▓", if e.enabled { t.accent_dim } else { t.fg_dim }),
                _ => ("██", if e.enabled { t.accent } else { t.fg_dim }),
            };
            spans.push(Span::styled(glyph, Style::new().fg(color)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), *row);
    }
}

fn render_cron_timers_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(
        &t,
        &format!("Systemd timers · {}", app.timers.len()),
        app.domain_color(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.timers.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("none", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    let max = (inner.height as usize).max(1);
    let lines: Vec<Line> = app
        .timers
        .iter()
        .take(max)
        .map(|tm| {
            Line::from(vec![
                Span::styled(tm.unit.clone(), Style::new().fg(t.fg_strong)),
                Span::styled(format!("  next {}", tm.next), Style::new().fg(t.fg_muted)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_cron_table(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let user_jobs = app
        .crons
        .iter()
        .filter(|entry| entry.source == CronSource::User)
        .count();
    let block = panel_block(
        &t,
        &format!("Scheduled jobs · {} ({} user)", app.crons.len(), user_jobs),
        app.domain_color(),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.crons.is_empty() {
        let msg = if app.mode == systui_core::ExecutionMode::ReadOnly {
            "no cron jobs found"
        } else {
            "no cron jobs yet — press a to add one"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    let header = Row::new(["", "SCHEDULE", "NEXT RUN", "USER", "COMMAND"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let body = app.crons.iter().enumerate().map(|(i, e)| {
        let (schedule, next) = cron_schedule_cells(app, e);
        let dot_color = if e.enabled { t.accent } else { t.fg_dim };
        let cmd_color = if e.enabled { t.fg } else { t.fg_dim };
        let row = Row::new([
            Cell::from(Span::styled("●", Style::new().fg(dot_color))),
            Cell::from(Span::styled(schedule, Style::new().fg(t.fg))),
            Cell::from(Span::styled(next, Style::new().fg(t.fg_muted))),
            Cell::from(Span::styled(
                e.user.clone().unwrap_or_else(|| "—".to_owned()),
                Style::new().fg(t.fg_muted),
            )),
            Cell::from(Span::styled(e.command.clone(), Style::new().fg(cmd_color))),
        ]);
        if i == app.crons_selected {
            row.style(Style::new().bg(t.bg_sel).add_modifier(Modifier::BOLD))
        } else {
            row
        }
    });
    let widths = [
        Constraint::Length(1),
        Constraint::Length(18),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Min(10),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        inner,
    );
}

/// Up to `n` upcoming run times for a cron expression, formatted for display.
fn cron_next_runs(app: &App, schedule_str: &str, n: usize) -> Vec<String> {
    let Ok(schedule) = parse_schedule(schedule_str) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(n);
    let mut cursor = app.now;
    for _ in 0..n {
        match schedule.next_after(cursor) {
            Some(next) => {
                out.push(next.format("%a %Y-%m-%d %H:%M").to_string());
                cursor = next;
            }
            None => break,
        }
    }
    out
}

fn render_cron_preview(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Preview", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(entry) = app.selected_cron() else {
        frame.render_widget(
            Paragraph::new(Span::styled("no job selected", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    };

    let valid = parse_schedule(&entry.schedule).is_ok();
    let field = |key: &str, value: String, color| {
        Line::from(vec![
            Span::styled(format!("{key:<9} "), Style::new().fg(t.fg_dim)),
            Span::styled(value, Style::new().fg(color)),
        ])
    };
    let mut lines = vec![
        field(
            "schedule",
            entry.schedule.clone(),
            if valid { t.fg } else { t.critical },
        ),
        field("human", cron_schedule_cells(app, entry).0, t.fg_muted),
        field(
            "user",
            entry.user.clone().unwrap_or_else(|| "—".to_owned()),
            t.fg,
        ),
        field("source", format!("{:?}", entry.source), t.fg_muted),
        field("command", entry.command.clone(), t.fg_strong),
    ];

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "next runs",
        Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD),
    )));
    let runs = cron_next_runs(app, &entry.schedule, 3);
    if runs.is_empty() {
        lines.push(Line::from(Span::styled("  —", Style::new().fg(t.fg_dim))));
    }
    for r in runs {
        lines.push(Line::from(Span::styled(
            format!("  → {r}"),
            Style::new().fg(t.fg),
        )));
    }

    if entry.source == CronSource::User && app.mode != systui_core::ExecutionMode::ReadOnly {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "edits back up the crontab first",
            Style::new().fg(t.fg_dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_cron_summary(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Cron health", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let warnings: Vec<&Finding> = app
        .findings
        .iter()
        .filter(|f| f.module == ModuleId::Crons)
        .collect();

    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{}", app.crons.len()),
            Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" jobs · ", Style::new().fg(t.fg_muted)),
        Span::styled(
            format!("{}", app.timers.len()),
            Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" timers · ", Style::new().fg(t.fg_muted)),
        Span::styled(
            format!("{}", warnings.len()),
            Style::new()
                .fg(if warnings.is_empty() {
                    t.accent
                } else {
                    t.high
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" warnings", Style::new().fg(t.fg_muted)),
    ])];
    lines.push(Line::from(""));

    if warnings.is_empty() {
        lines.push(Line::from(Span::styled(
            "no warnings",
            Style::new().fg(t.accent),
        )));
    } else {
        let max = (inner.height as usize).saturating_sub(3).max(1);
        for f in warnings.iter().take(max) {
            let color = t.severity(f.severity);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<5} ", f.severity.to_string().to_uppercase()),
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(f.title.clone(), Style::new().fg(t.fg_strong)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
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

    // Clean by default: the instances table full-height. Dense adds the
    // selected-instance detail pane.
    if !app.dense {
        render_database_table(frame, app, area);
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
                "✓ no findings — nothing flagged",
                Style::new().fg(app.theme.accent),
            ))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let rows = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).split(area);
    render_security_header(frame, app, rows[0]);
    render_security_findings(frame, app, rows[1]);
}

/// Severity distribution as a bar chart, with a week-over-week trend note.
fn render_security_header(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let counts = app.finding_counts();

    // Reserve a slim right column for the trend; the bar chart fills the rest.
    let split = Layout::horizontal([Constraint::Min(0), Constraint::Length(24)]).split(area);
    severity_bars(
        frame,
        &t,
        split[0],
        [
            counts[0] as u64,
            counts[1] as u64,
            counts[2] as u64,
            counts[3] as u64,
            counts[4] as u64,
        ],
    );

    // Findings trend vs ~7 days ago, from the persisted snapshots.
    if let Some(base) = app.state.baseline(&app.host_label, app.now.date(), 7) {
        let now_total = counts[0] + counts[1] + counts[2];
        let base_total = base.findings();
        let (arrow, word, color) = match now_total.cmp(&base_total) {
            std::cmp::Ordering::Less => ("↓", base_total - now_total, t.accent),
            std::cmp::Ordering::Greater => ("↑", now_total - base_total, t.critical),
            std::cmp::Ordering::Equal => ("=", 0, t.fg_dim),
        };
        let text = if word == 0 {
            "no change\nvs last week".to_owned()
        } else {
            format!("{arrow}{word}\nfrom last week")
        };
        let block = panel_block(&t, "Trend", app.domain_color());
        let inner = block.inner(split[1]);
        frame.render_widget(block, split[1]);
        frame.render_widget(
            Paragraph::new(Span::styled(text, Style::new().fg(color)))
                .wrap(Wrap { trim: true }),
            inner,
        );
    }
}

/// Evidence-based findings list: severity edge bar, title + id/module, an inset
/// evidence line and the recommendation (spec §14).
fn render_security_findings(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Findings · evidence-based", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let per_finding = 3;
    let max = (inner.height as usize / per_finding).max(1);
    let start = app.security_selected.saturating_sub(max.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    for (idx, f) in app.findings.iter().enumerate().skip(start).take(max) {
        let color = t.severity(f.severity);
        let selected = idx == app.security_selected;
        let marker = if selected { ">" } else { " " };
        let title_style = if selected {
            Style::new()
                .fg(t.fg_strong)
                .bg(t.bg_elev)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.fg_strong).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::new().fg(t.accent)),
            Span::styled("▌ ", Style::new().fg(color)),
            Span::styled(
                format!("{:<5} ", f.severity.to_string().to_uppercase()),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("[{}] ", f.status.label()),
                Style::new().fg(t.fg_dim),
            ),
            Span::styled(f.title.clone(), title_style),
            Span::styled(
                format!("   {} · {}", f.id, f.module),
                Style::new().fg(t.fg_dim),
            ),
        ]));
        if let Some(evidence) = f.evidence.first() {
            lines.push(Line::from(Span::styled(
                format!("    {evidence}"),
                Style::new().fg(t.fg_muted).bg(t.bg_elev),
            )));
        }
        if !f.recommendation.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    → {}", f.recommendation),
                Style::new().fg(t.accent),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
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
/// Dashboard: metric tiles + health score + critical findings + at-a-glance,
/// laid out as a two-column multi-panel screen (spec §14).
/// The overview cockpit: a dense, fully-graphical grid (design `tabs1.jsx`
/// `TabOverview`). A full-width pulse row of live stat tiles, then a 3×2 grid of
/// memory/disk gauges, top processes by memory, the health ring, exposure by
/// risk, recent alerts and session notes. Every value is real collector data.
fn render_dashboard(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let rows = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).split(area);
    render_pulse(frame, app, snap, rows[0]);

    let cols = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(rows[1]);
    let c1 = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(cols[0]);
    let c2 = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(cols[1]);
    let c3 = Layout::vertical([Constraint::Min(0), Constraint::Length(5)]).split(cols[2]);

    render_mem_disks(frame, app, snap, c1[0]);
    render_top_processes(frame, app, c1[1]);
    render_health_panel(frame, app, c2[0]);
    render_exposure_overview(frame, app, c2[1]);
    render_recent_alerts(frame, app, c3[0]);
    render_session_notes(frame, app, c3[1]);
}

/// Clip a string to `max` display columns, appending `…` when truncated.
fn clip(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_owned();
    }
    let take = max.saturating_sub(1);
    format!("{}…", chars[..take].iter().collect::<String>())
}

/// The short, human name of a process: the basename of its first argv token.
fn short_cmd(cmd: &str) -> String {
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    first.rsplit('/').next().unwrap_or(first).to_owned()
}

/// SYSTEM · PULSE: a full-width row of live stat tiles with sparklines.
fn render_pulse(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = &app.theme;
    let block = panel_block(t, "system · pulse", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cpu = snap.cpu.busy_percent;
    let mem = snap.memory.used_percent();
    let swap = snap.swap.used_percent();
    let mem_used_g = snap.memory.used_kb() as f64 / 1_048_576.0;
    let mem_total_g = snap.memory.total_kb as f64 / 1_048_576.0;
    let worst_disk = snap.disks.iter().max_by_key(|d| d.use_percent);
    let disk_pct = worst_disk.map(|d| d.use_percent as f64).unwrap_or(0.0);

    struct Tile<'a> {
        label: String,
        value: String,
        color: ratatui::style::Color,
        spark: Option<&'a [u64]>,
        sub: String,
    }
    let tiles = [
        Tile { label: "CPU".into(), value: format!("{cpu:.0}%"), color: usage_color(t, cpu), spark: Some(&app.cpu_history), sub: String::new() },
        Tile { label: format!("MEM {mem_used_g:.1}/{mem_total_g:.0}G"), value: format!("{mem:.0}%"), color: usage_color(t, mem), spark: Some(&app.mem_history), sub: String::new() },
        Tile { label: "SWAP".into(), value: format!("{swap:.0}%"), color: usage_color(t, swap), spark: None, sub: format!("{:.1}/{:.0}G", snap.swap.used_kb() as f64 / 1_048_576.0, snap.swap.total_kb as f64 / 1_048_576.0) },
        Tile { label: "LOAD 1·5·15".into(), value: format!("{:.2}", snap.load.one), color: t.fg_strong, spark: None, sub: format!("{:.2} · {:.2}", snap.load.five, snap.load.fifteen) },
        Tile { label: "DISK".into(), value: format!("{disk_pct:.0}%"), color: usage_color(t, disk_pct), spark: None, sub: worst_disk.map(|d| d.mount.clone()).unwrap_or_default() },
        Tile { label: "PROCS".into(), value: format!("{}", app.processes.len()), color: t.low, spark: None, sub: format!("{} cores", snap.cpu.cores) },
        Tile { label: "UPTIME".into(), value: fmt_uptime(snap.uptime_secs), color: t.accent, spark: None, sub: String::new() },
    ];

    let cells = Layout::horizontal(vec![Constraint::Ratio(1, tiles.len() as u32); tiles.len()]).split(inner);
    for (tile, cell) in tiles.iter().zip(cells.iter()) {
        let r = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(*cell);
        frame.render_widget(
            Paragraph::new(Span::styled(tile.label.clone(), Style::new().fg(t.fg_dim))),
            r[0],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                tile.value.clone(),
                Style::new().fg(tile.color).add_modifier(Modifier::BOLD),
            )),
            r[1],
        );
        if let Some(data) = tile.spark {
            frame.render_widget(
                Sparkline::default()
                    .data(data)
                    .max(100)
                    .style(Style::new().fg(tile.color)),
                r[2],
            );
        } else if !tile.sub.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(tile.sub.clone(), Style::new().fg(t.fg_muted))),
                r[2],
            );
        }
    }
}

/// MEMORY · DISKS: RAM/swap and per-filesystem usage as inline gauge bars.
fn render_mem_disks(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = &app.theme;
    let block = panel_block(t, "memory · disks", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut items: Vec<(String, f64, String, ratatui::style::Color)> = Vec::new();
    let ram = snap.memory.used_percent();
    items.push(("RAM".into(), ram, format!("{ram:.0}%"), usage_color(t, ram)));
    if snap.swap.total_kb > 0 {
        let sw = snap.swap.used_percent();
        items.push(("swap".into(), sw, format!("{sw:.0}%"), usage_color(t, sw)));
    }
    for d in &snap.disks {
        let p = d.use_percent as f64;
        items.push((
            clip(&d.mount, 8),
            p,
            format!("{}%", d.use_percent),
            usage_color(t, p),
        ));
    }
    let rows = Layout::vertical(vec![Constraint::Length(1); items.len()]).split(inner);
    for ((label, pct, reading, color), row) in items.iter().zip(rows.iter()) {
        labeled_gauge(frame, t, *row, label, 9, *pct, reading, *color);
    }
}

/// PROCESSES · TOP BY MEM: the heaviest processes as proportional gauge bars.
fn render_top_processes(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel_block(t, "processes · top by mem", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.processes.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no process data", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    let mut procs: Vec<_> = app.processes.iter().collect();
    procs.sort_by(|a, b| {
        b.mem_percent
            .partial_cmp(&a.mem_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let n = (inner.height as usize).min(procs.len());
    let rows = Layout::vertical(vec![Constraint::Length(1); n]).split(inner);
    for (p, row) in procs.iter().take(n).zip(rows.iter()) {
        labeled_gauge(
            frame,
            t,
            *row,
            &clip(&short_cmd(&p.command), 14),
            15,
            p.mem_percent,
            &format!("{:.1}%", p.mem_percent),
            t.magenta,
        );
    }
}

/// EXPOSURE · BY RISK: listening sockets ranked by severity, with scope + owner.
fn render_exposure_overview(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel_block(t, "exposure · by risk", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.exposures.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no listening sockets", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    let mut ex: Vec<_> = app.exposures.iter().collect();
    ex.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut lines = Vec::new();
    for e in ex.iter().take(inner.height as usize) {
        let proto = match e.listener.protocol {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        };
        let (scope, scope_color) = match e.scope {
            BindScope::External => (
                "extern",
                if e.severity >= Severity::High {
                    t.critical
                } else {
                    t.high
                },
            ),
            BindScope::Loopback => ("loop", t.accent),
        };
        let proc = e
            .listener
            .process
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".to_owned());
        let mut spans = risk_ladder(t, e.severity);
        spans.push(Span::styled(
            format!(" {proto}:{:<5}", e.listener.port),
            Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {scope:<6} "), Style::new().fg(scope_color)));
        spans.push(Span::styled(clip(&proc, 14), Style::new().fg(t.magenta)));
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// RECENT · ALERTS & EVENTS: the latest log lines, colored by severity.
fn render_recent_alerts(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = panel_block(t, "recent · alerts & events", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.logs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no recent log lines", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }
    let msg_width = (inner.width as usize).saturating_sub(28);
    let mut lines = Vec::new();
    for e in app.logs.iter().take(inner.height as usize) {
        let color = log_priority_color(app, e);
        let lvl = if e.priority <= 3 {
            "ERR"
        } else if e.priority == 4 {
            "WARN"
        } else if e.priority <= 6 {
            "INFO"
        } else {
            "DBG"
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", e.time), Style::new().fg(t.fg_dim)),
            Span::styled(
                format!("{lvl:<4} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(clip(&e.identifier, 13), Style::new().fg(t.magenta)),
            Span::raw(" "),
            Span::styled(clip(&e.message, msg_width), Style::new().fg(t.fg)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Per-host session notes, persisted in the local state store. The newest notes
/// are shown last; `n` starts a new note.
fn render_session_notes(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Session notes", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let notes = app.state.notes(&app.host_label);
    let mut lines: Vec<Line> = Vec::new();
    if notes.is_empty() {
        lines.push(Line::from(Span::styled(
            "no notes — press n to add one",
            Style::new().fg(t.fg_dim),
        )));
    } else {
        // Show the most recent few (the panel is short).
        let start = notes.len().saturating_sub(4);
        for note in &notes[start..] {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", note.at), Style::new().fg(t.fg_dim)),
                Span::styled(note.text.clone(), Style::new().fg(t.fg)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// A square, titled panel block in the theme's surface style. The dense CRT
/// look: single-line borders and an UPPERCASE accented title.
fn panel_block(theme: &Theme, title: &str, accent: ratatui::style::Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.border))
        .style(Style::new().bg(theme.bg_elev))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            Style::new().fg(accent).add_modifier(Modifier::BOLD),
        ))
}

/// Color for a 0–100 utilisation: green / amber / red by band.
fn usage_color(theme: &Theme, percent: f64) -> ratatui::style::Color {
    if percent >= 85.0 {
        theme.critical
    } else if percent >= 60.0 {
        theme.high
    } else {
        theme.accent
    }
}

/// Health-score panel: the score, a gauge, and the deduction breakdown.
fn render_health_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel_block(&app.theme, "Health score", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(health) = &app.health else {
        frame.render_widget(
            Paragraph::new(Span::styled("no data", Style::new().fg(app.theme.fg_dim))),
            inner,
        );
        return;
    };

    let color = score_color(app, health.score);
    let rows =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    // Trend vs ~7 days ago, from the persisted snapshots (fills over time).
    let label = match app.state.baseline(&app.host_label, app.now.date(), 7) {
        Some(base) => format!("{}/100  (was {} 7d ago)", health.score, base.score),
        None => format!("{}/100", health.score),
    };
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(color).bg(app.theme.bg_elev))
            .ratio((health.score as f64).clamp(0.0, 100.0) / 100.0)
            .label(Span::styled(
                label,
                Style::new()
                    .fg(app.theme.fg_strong)
                    .add_modifier(Modifier::BOLD),
            )),
        rows[0],
    );

    let mut lines = Vec::new();
    if health.checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no deductions — healthy",
            Style::new().fg(app.theme.accent),
        )));
    } else {
        for check in health.checks.iter().take(6) {
            lines.push(finding_line(app, check));
        }
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);
}

fn finding_line(app: &App, check: &systui_collectors::Check) -> Line<'static> {
    let color = app.theme.severity(check.severity);
    let label = format!("{:?}", check.severity).to_uppercase();
    Line::from(vec![
        Span::styled(
            format!("  -{:<3} ", check.points),
            Style::new().fg(app.theme.fg_dim),
        ),
        Span::styled(format!("{label:<5} "), Style::new().fg(color)),
        Span::styled(check.message.clone(), Style::new().fg(app.theme.fg)),
    ])
}

/// The System tab: a multi-panel hardware/identity view built entirely from the
/// live `SystemSnapshot` (no mock data). Left: identity + disks; right: memory + users.
fn render_system(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let cols =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

    // Clean by default: identity + load history on the left, memory on the right.
    // Dense adds the disks and logged-in users panels.
    if app.dense {
        let left = Layout::vertical([
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Min(0),
        ])
        .split(cols[0]);
        let right = Layout::vertical([Constraint::Length(7), Constraint::Min(0)]).split(cols[1]);
        render_system_identity(frame, app, snap, left[0]);
        render_system_load(frame, app, snap, left[1]);
        render_system_disks(frame, app, snap, left[2]);
        render_system_memory(frame, app, snap, right[0]);
        render_system_users(frame, app, snap, right[1]);
    } else {
        let left = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).split(cols[0]);
        render_system_identity(frame, app, snap, left[0]);
        render_system_load(frame, app, snap, left[1]);
        render_system_memory(frame, app, snap, cols[1]);
    }
}

/// LOAD · CPU history: the 1/5/15 load averages with a live CPU-busy sparkline.
fn render_system_load(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "load · cpu history", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let sat = if snap.cpu.cores > 0 {
        snap.load.one / snap.cpu.cores as f64
    } else {
        snap.load.one
    };
    let sat_color = usage_color(&t, sat * 100.0);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{:.2}", snap.load.one),
                Style::new().fg(sat_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ·  {:.2}  ·  {:.2}", snap.load.five, snap.load.fifteen),
                Style::new().fg(t.fg_muted),
            ),
            Span::styled(
                format!("   {} cores · sat {sat:.2}", snap.cpu.cores),
                Style::new().fg(t.fg_dim),
            ),
        ])),
        rows[0],
    );
    if app.cpu_history.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("gathering history…", Style::new().fg(t.fg_dim))),
            rows[1],
        );
    } else {
        frame.render_widget(
            Sparkline::default()
                .data(&app.cpu_history)
                .max(100)
                .style(Style::new().fg(t.accent)),
            rows[1],
        );
    }
}

/// Identity & vitals: hostname, OS, kernel, uptime, CPU and load.
fn render_system_identity(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let block = panel_block(&app.theme, "System", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        label_value(app, "Hostname", &snap.hostname),
        label_value(app, "OS", snap.os.as_deref().unwrap_or("unknown")),
        label_value(app, "Kernel", &snap.kernel),
    ];
    if let Some(virt) = snap.virtualization.as_deref() {
        let shown = if virt == "none" { "bare metal" } else { virt };
        lines.push(label_value(app, "Virt", shown));
    }
    lines.push(label_value(app, "Uptime", &human_uptime(snap.uptime_secs)));
    let cpu = match snap.cpu_model.as_deref() {
        Some(model) => format!(
            "{:.0}% busy · {} cores · {}",
            snap.cpu.busy_percent, snap.cpu.cores, model
        ),
        None => format!(
            "{:.0}% busy · {} cores",
            snap.cpu.busy_percent, snap.cpu.cores
        ),
    };
    lines.push(label_value(app, "CPU", &cpu));
    lines.push(label_value(
        app,
        "Load",
        &format!(
            "{:.2}  {:.2}  {:.2}",
            snap.load.one, snap.load.five, snap.load.fifteen
        ),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Memory & swap as labelled, colour-coded gauges.
fn render_system_memory(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Memory", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1); 2]).split(inner);
    let ram = snap.memory.used_percent();
    labeled_gauge(
        frame,
        &t,
        rows[0],
        "RAM",
        6,
        ram,
        &format!(
            "{} / {}",
            human_kb(snap.memory.used_kb()),
            human_kb(snap.memory.total_kb)
        ),
        usage_color(&t, ram),
    );
    if snap.swap.total_kb > 0 {
        let swap = snap.swap.used_percent();
        labeled_gauge(
            frame,
            &t,
            rows[1],
            "Swap",
            6,
            swap,
            &format!(
                "{} / {}",
                human_kb(snap.swap.used_kb()),
                human_kb(snap.swap.total_kb)
            ),
            usage_color(&t, swap),
        );
    } else {
        labeled_gauge(frame, &t, rows[1], "Swap", 6, 0.0, "none", t.fg_dim);
    }
}

/// Disks: a filled gauge bar per mount, colour-coded by utilisation.
fn render_system_disks(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Disks", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snap.disks.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no disk data", Style::new().fg(t.fg_dim))),
            inner,
        );
        return;
    }

    let rows = Layout::vertical(
        std::iter::repeat_n(Constraint::Length(1), snap.disks.len()).collect::<Vec<_>>(),
    )
    .split(inner);
    for (d, row) in snap.disks.iter().zip(rows.iter()) {
        let pct = d.use_percent as f64;
        labeled_gauge(
            frame,
            &t,
            *row,
            &d.mount,
            13,
            pct,
            &format!("{}% · {} / {}", d.use_percent, human_kb(d.used_kb), human_kb(d.size_kb)),
            usage_color(&t, pct),
        );
    }
}

/// Logged-in users.
fn render_system_users(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Logged in", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snap.users.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no users logged in",
                Style::new().fg(t.fg_dim),
            )),
            inner,
        );
        return;
    }

    let header = Row::new(["USER", "TTY", "LOGIN", "FROM"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let rows = snap.users.iter().map(|u| {
        Row::new(vec![
            Cell::from(u.name.clone()),
            Cell::from(u.tty.clone()),
            Cell::from(u.login_time.clone()),
            Cell::from(u.from.clone().unwrap_or_default()),
        ])
    });
    let widths = [
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Min(8),
        Constraint::Min(8),
    ];
    frame.render_widget(
        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .style(Style::new().fg(t.fg)),
        inner,
    );
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

/// Status bar: contextual key hints on the left, a live indicator on the right.
fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    frame.render_widget(Block::default().style(Style::new().bg(app.theme.bg_elev)), area);
    let hints: &[(&str, &str)] = match app.current_tab() {
        Tab::Crons => &[
            ("r", "refresh"),
            ("a", "add"),
            ("e", "edit"),
            ("d", "delete"),
            ("x", "toggle"),
            ("n", "run now"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Logs => &[
            ("r", "refresh"),
            ("/", "search"),
            ("l", "level"),
            ("t", "window"),
            ("S", "save search"),
            ("e", "export json"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Dashboard => &[
            ("r", "refresh"),
            ("n", "add note"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Processes => &[
            ("r", "refresh"),
            ("s", "sort"),
            ("t", "list/tree"),
            ("a", "signal"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Services => &[
            ("r", "refresh"),
            ("f", "filter"),
            ("a", "actions"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Network => &[
            ("r", "refresh"),
            ("c", "connectivity"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Docker => &[
            ("r", "refresh"),
            ("a", "actions"),
            ("p", "prune images"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Tab::Security => &[
            ("r", "refresh"),
            ("a", "accept"),
            ("i", "ignore"),
            ("o", "open"),
            ("f/v", "fixed/false"),
            ("?", "help"),
            ("q", "quit"),
        ],
        _ => &[
            ("r", "refresh"),
            ("/", "search"),
            ("a", "actions"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", Style::new().fg(app.theme.fg_dim)));
        }
        spans.push(Span::styled(
            *key,
            Style::new()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::new().fg(app.theme.fg_muted),
        ));
    }

    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(46)]).split(area);
    // A transient status message (e.g. an export result) takes over the hint row.
    if let Some(msg) = &app.status_message {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {msg}"),
                Style::new()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            cols[0],
        );
    } else {
        frame.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
    }

    // Right: theme + visual style + live/attached indicator.
    let (dot, label, color) = if app.snapshot.is_some() {
        ("●", "live", app.theme.accent)
    } else {
        ("○", "detached", app.theme.fg_dim)
    };
    let key_style = Style::new()
        .fg(app.theme.accent)
        .add_modifier(Modifier::BOLD);
    let right = Line::from(vec![
        Span::styled("view ", Style::new().fg(app.theme.fg_dim)),
        Span::styled(
            format!("{}  ", app.current_tab().title().to_lowercase()),
            Style::new().fg(app.theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("T ", key_style),
        Span::styled(
            format!("{}  ", app.theme_kind.label()),
            Style::new().fg(app.theme.fg_dim),
        ),
        Span::styled(format!("{dot} "), Style::new().fg(color)),
        Span::styled(format!("{label}  "), Style::new().fg(app.theme.fg_muted)),
        Span::styled(
            format!("v{} ", env!("CARGO_PKG_VERSION")),
            Style::new().fg(app.theme.fg_dim),
        ),
    ]);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), cols[1]);
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
        ("n", "run the selected user cron job now (Crons tab)"),
        ("r", "refresh"),
        ("s", "sort processes by CPU/memory (Processes)"),
        ("t", "Processes: toggle list/tree · Logs: time window"),
        ("f", "cycle Services filter (all/failed/running/…)"),
        ("c", "run connectivity probes (Network tab)"),
        ("p", "prune dangling images (Docker tab)"),
        ("n", "add a session note (Dashboard)"),
        ("S / ↵", "save / apply a log search (Logs tab)"),
        ("/", "search logs (Esc to clear)"),
        ("l", "cycle log level (Logs tab)"),
        ("e", "export the current logs to JSON (Logs tab)"),
        ("T", "cycle theme (phosphor / ember)"),
        ("V", "cycle visual style (sober / rich)"),
        ("D", "toggle dense mode (more detail per screen)"),
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
    use systui_collectors::Disk;
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
    fn cron_builder_shows_live_preview() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(7); // Crons
        app.open_add_cron_form();
        assert!(app.cron_builder.is_some());

        let out = render_to_string(&app, 120, 32);
        assert!(out.contains("New cron job"));
        assert!(out.contains("Frequency"));
        assert!(out.contains("Daily")); // default frequency
        // Live preview: the generated expression and its human description.
        assert!(out.contains("0 9 * * *"));
        assert!(out.contains("Every day"));
    }

    #[test]
    fn cron_builder_frequency_changes_visible_fields() {
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.open_add_cron_form();
        // Frequency starts on Daily; one increment moves to Weekly, which adds
        // the Weekday row.
        app.cron_form_increment(); // Daily -> Weekly
        let out = render_to_string(&app, 120, 32);
        assert!(out.contains("Weekly"));
        assert!(out.contains("Weekday"));
    }

    #[test]
    fn renders_chrome_and_empty_state() {
        let app = App::new("prod-01", ExecutionMode::ReadOnly);
        // 120 cols so the full 10-tab bar (through "security") is visible.
        let out = render_to_string(&app, 120, 24);
        assert!(out.contains('█')); // ASCII SYSTOOLS wordmark
        assert!(out.contains("prod-01"));
        assert!(out.contains("READ-ONLY"));
        assert!(out.contains("dashboard"));
        assert!(out.contains("security"));
        assert!(out.contains("q quit"));
        assert!(out.contains("No data yet"));
    }

    #[test]
    #[ignore = "manual: prints the rendered overview to stdout"]
    fn dump_overview() {
        use systui_collectors::{Check, HealthReport, HostCapabilities};
        use systui_core::Severity;
        let mut app = App::new("aurora-prod-01", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        app.capabilities = Some(HostCapabilities {
            user: "root".to_owned(),
            uid: Some(0),
            can_sudo: true,
        });
        app.health = Some(HealthReport {
            score: 67,
            checks: vec![
                Check {
                    severity: Severity::High,
                    message: "2 failed services".to_owned(),
                    points: 14,
                },
                Check {
                    severity: Severity::Medium,
                    message: "3 security updates pending".to_owned(),
                    points: 8,
                },
            ],
        });
        app.cpu_history = (0..60).map(|i| 30 + (i * 7 % 40) as u64).collect();
        app.mem_history = (0..60).map(|i| 50 + (i * 3 % 25) as u64).collect();
        let proc = |pid, user: &str, cpu, mem: f64, cmd: &str| systui_collectors::Process {
            pid,
            ppid: 1,
            user: user.to_owned(),
            cpu_percent: cpu,
            mem_percent: mem,
            rss_kb: (mem * 6_400.0) as u64,
            command: cmd.to_owned(),
        };
        app.processes = vec![
            proc(1602, "postgres", 3.2, 28.4, "/usr/lib/postgresql/15/bin/postgres -D /var/lib"),
            proc(3104, "prom", 8.2, 15.1, "/usr/bin/prometheus --config.file=/etc/prom.yml"),
            proc(2890, "root", 6.7, 11.6, "nginx: master process /usr/sbin/nginx"),
            proc(3204, "grafana", 1.4, 9.7, "grafana-server --homepath=/usr/share/grafana"),
            proc(1812, "redis", 1.1, 4.3, "redis-server *:6379"),
            proc(1442, "alvaro", 12.4, 2.6, "systools --tui"),
        ];
        let listener = |port, ip: &str, name: &str| systui_collectors::Listener {
            protocol: systui_collectors::Protocol::Tcp,
            local_ip: ip.to_owned(),
            port,
            process: Some(systui_collectors::ProcessRef {
                pid: 0,
                name: name.to_owned(),
            }),
            unit: None,
        };
        let expo = |port, ip: &str, name: &str, scope, sev| systui_collectors::ExposureEntry {
            listener: listener(port, ip, name),
            scope,
            sensitive_service: None,
            severity: sev,
            evidence: String::new(),
        };
        app.exposures = vec![
            expo(9090, "0.0.0.0", "prometheus", BindScope::External, Severity::High),
            expo(2376, "0.0.0.0", "dockerd", BindScope::External, Severity::Critical),
            expo(443, "0.0.0.0", "nginx", BindScope::External, Severity::Low),
            expo(22, "0.0.0.0", "sshd", BindScope::External, Severity::Medium),
            expo(5432, "127.0.0.1", "postgres", BindScope::Loopback, Severity::Info),
        ];
        let log = |time: &str, prio, id: &str, msg: &str| LogEntry {
            time: time.to_owned(),
            priority: prio,
            identifier: id.to_owned(),
            message: msg.to_owned(),
        };
        app.logs = vec![
            log("12:42:11", 3, "systools-collect", "docker collector timed out (3.0s)"),
            log("12:41:03", 4, "fail2ban", "banned 2 IPs in /24"),
            log("12:40:55", 6, "docker", "log-shipper restarted (back-off 14s)"),
            log("12:38:00", 6, "apt", "cache refreshed · 3 security updates"),
            log("12:35:18", 4, "exposure", "dockerd :2376 reachable from LAN"),
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(0);
        println!("\n{}", render_to_string(&app, 120, 38));
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
        assert!(out.contains("PRIVILEGED"));
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

    #[test]
    #[ignore = "manual: prints the rendered cron tab to stdout"]
    fn dump_cron() {
        use systui_collectors::{CronEntry, CronSource};
        let mut app = App::new("aurora-prod-01", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        let cron = |sched: &str, user: &str, cmd: &str, src| CronEntry {
            schedule: sched.to_owned(),
            user: Some(user.to_owned()),
            command: cmd.to_owned(),
            source: src,
            origin: "/etc/crontab".to_owned(),
            enabled: true,
        };
        app.crons = vec![
            cron("17 * * * *", "root", "run-parts /etc/cron.hourly", CronSource::System),
            cron("25 6 * * *", "root", "run-parts /etc/cron.daily", CronSource::System),
            cron("*/5 * * * *", "alvaro", "/home/alvaro/bin/sync-notes.sh", CronSource::User),
            cron("0 */2 * * *", "alvaro", "/srv/systools/scripts/snapshot.sh", CronSource::User),
            cron("30 3 * * *", "postgres", "/usr/bin/pg_dump -Fc db_main", CronSource::CronD),
            cron("5-55/10 * * * *", "root", "debian-sa1 1 1", CronSource::CronD),
        ];
        app.now = chrono::NaiveDate::from_ymd_opt(2026, 5, 29)
            .unwrap()
            .and_hms_opt(8, 49, 0)
            .unwrap();
        app.view_state = ViewState::Ready;
        app.select_tab(7); // Crons
        println!("\n{}", render_to_string(&app, 120, 38));
    }

    #[test]
    #[ignore = "manual: prints the rendered processes tab to stdout"]
    fn dump_process() {
        use systui_collectors::Process;
        let mut app = App::new("aurora-prod-01", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        let p = |pid, ppid, user: &str, cpu, mem: f64, rss, cmd: &str| Process {
            pid,
            ppid,
            user: user.to_owned(),
            cpu_percent: cpu,
            mem_percent: mem,
            rss_kb: rss,
            command: cmd.to_owned(),
        };
        app.processes = vec![
            p(1, 0, "root", 0.1, 0.2, 8_200, "/sbin/init"),
            p(821, 1, "root", 0.5, 0.3, 17_800, "/usr/sbin/sshd -D"),
            p(1602, 1, "postgres", 3.2, 28.4, 281_000, "/usr/lib/postgresql/15/bin/postgres"),
            p(3104, 1, "prom", 8.2, 15.1, 151_200, "/usr/bin/prometheus"),
            p(2890, 1, "root", 6.7, 11.6, 192_800, "nginx: master process"),
            p(3204, 1, "grafana", 1.4, 9.7, 96_800, "grafana-server"),
            p(1812, 1, "redis", 1.1, 4.3, 43_200, "redis-server *:6379"),
            p(1442, 821, "alvaro", 12.4, 2.6, 52_800, "systools --tui"),
        ];
        app.dense = true;
        app.view_state = ViewState::Ready;
        app.select_tab(2);
        println!("\n{}", render_to_string(&app, 120, 38));
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
            cpu_model: Some("Intel(R) Xeon(R) E5-2670".to_owned()),
            virtualization: Some("kvm".to_owned()),
        }
    }

    #[test]
    fn renders_dashboard_when_ready() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 100, 24);
        // The hostname from the snapshot is shown in the top-bar meta grid; the
        // pulse row + gauges show the snapshot vitals.
        assert!(out.contains("prod-01"));
        assert!(out.contains("CPU"));
        assert!(out.contains("RAM"));
        assert!(out.contains("DISK"));
        assert!(out.contains("LOAD"));
        assert!(out.contains("89")); // worst disk usage %
    }

    #[test]
    fn renders_system_detail_when_ready() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(1); // System
        app.dense = true; // disks + users panels live in dense mode

        let out = render_to_string(&app, 100, 30);
        assert!(out.contains("Hostname"));
        assert!(out.contains("Kernel"));
        assert!(out.contains("6.1.0-18-amd64"));
        // Multi-panel layout: identity, memory gauges, disks and logged-in users.
        // Panel titles render UPPERCASE in the CRT look.
        assert!(out.contains("MEMORY"));
        assert!(out.contains("DISKS"));
        assert!(out.contains("LOGGED IN"));
        assert!(out.contains("admin"));
        // New vitals: CPU model and virtualization.
        assert!(out.contains("Xeon"));
        assert!(out.contains("Virt"));
        assert!(out.contains("kvm"));
    }

    #[test]
    fn processes_show_detail_panel_and_tree() {
        use systui_collectors::Process;
        let mut app = App::new("prod-01", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.processes = vec![
            Process {
                pid: 1,
                ppid: 0,
                user: "root".into(),
                cpu_percent: 0.1,
                mem_percent: 0.2,
                rss_kb: 8_200,
                command: "systemd".into(),
            },
            Process {
                pid: 880,
                ppid: 1,
                user: "root".into(),
                cpu_percent: 0.4,
                mem_percent: 1.1,
                rss_kb: 14_200,
                command: "sshd".into(),
            },
            Process {
                pid: 3300,
                ppid: 880,
                user: "admin".into(),
                cpu_percent: 12.4,
                mem_percent: 0.8,
                rss_kb: 52_800,
                command: "node".into(),
            },
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(2);
        app.dense = true; // the process detail pane lives in dense mode

        // Detail panel reflects the selected process (top of the CPU sort: node).
        // Render tall: a CPU bar chart now sits above the table on this tab.
        let out = render_to_string(&app, 110, 36);
        assert!(out.contains("PROCESS")); // detail panel title (uppercase)
        assert!(out.contains("Parent")); // parent row
        assert!(out.contains("sshd (880)")); // node's parent resolved by ppid

        // Tree view indents children and exposes the child count in the detail.
        app.toggle_process_view();
        assert_eq!(app.process_view, ProcessView::Tree);
        assert_eq!(app.processes_selected, 0); // reset on toggle → systemd
        let tree = render_to_string(&app, 110, 36);
        assert!(tree.contains("· tree"));
        assert!(tree.contains("Children")); // systemd has children
        // The tree is drawn with connector glyphs, not bare indentation.
        assert!(tree.contains("├─ ") || tree.contains("└─ "));
    }

    #[test]
    fn tree_prefix_draws_connectors() {
        // Pre-order depths for: systemd / ├ sshd / └ nginx / (nginx's child) worker.
        let depths = [0u16, 1, 1, 2];
        assert_eq!(tree_prefix(&depths, 0), "");
        assert_eq!(tree_prefix(&depths, 1), "├─ "); // sshd has a following sibling
        assert_eq!(tree_prefix(&depths, 2), "└─ "); // nginx is the last child
        assert_eq!(tree_prefix(&depths, 3), "   └─ "); // worker under the last child
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
                rss_kb: 8_200,
                command: "systemd".to_owned(),
            },
            Process {
                pid: 3300,
                ppid: 1,
                user: "admin".to_owned(),
                cpu_percent: 12.4,
                mem_percent: 0.8,
                rss_kb: 52_800,
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
    fn dashboard_shows_graphical_panels() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 120, 36);
        // The overview is a graphical grid of titled panels (no text status cards).
        assert!(out.contains("SYSTEM · PULSE"));
        assert!(out.contains("MEMORY · DISKS"));
        assert!(out.contains("PROCESSES · TOP BY MEM"));
        assert!(out.contains("EXPOSURE · BY RISK"));
        assert!(out.contains("RECENT · ALERTS & EVENTS"));
        // Pulse + gauge readings come from the real snapshot.
        assert!(out.contains("RAM"));
        assert!(out.contains("UPTIME"));
    }

    #[test]
    fn dashboard_pulse_and_panels_show_real_data() {
        use systui_collectors::{ExposureEntry, Listener, Process, ProcessRef, Protocol};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.processes = vec![Process {
            pid: 1602,
            ppid: 1,
            user: "postgres".into(),
            cpu_percent: 3.2,
            mem_percent: 28.4,
            rss_kb: 281_000,
            command: "/usr/lib/postgresql/15/bin/postgres".into(),
        }];
        app.exposures = vec![ExposureEntry {
            listener: Listener {
                protocol: Protocol::Tcp,
                local_ip: "0.0.0.0".into(),
                port: 9090,
                process: Some(ProcessRef {
                    pid: 3104,
                    name: "prometheus".into(),
                }),
                unit: None,
            },
            scope: BindScope::External,
            sensitive_service: None,
            severity: Severity::High,
            evidence: String::new(),
        }];
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 120, 36);
        // Process bar uses the basename + real mem%; exposure shows port/scope/proc.
        assert!(out.contains("postgres"));
        assert!(out.contains("28.4%"));
        assert!(out.contains("tcp:9090"));
        assert!(out.contains("extern"));
        assert!(out.contains("prometheus"));
    }

    #[test]
    fn dashboard_shows_health_trend_and_session_notes() {
        use systui_collectors::HealthReport;
        let mut app = App::new("prod-01", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.health = Some(HealthReport {
            score: 78,
            checks: Vec::new(),
        });
        app.now = chrono::NaiveDate::from_ymd_opt(2026, 5, 25)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap();
        // A snapshot 7 days ago provides the baseline; a note is persisted.
        app.state
            .record_snapshot("prod-01", "2026-05-18", 91, 0, 0, 0);
        app.state
            .add_note("prod-01", "2026-05-25 14:18", "reviewed nginx errors");
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 120, 32);
        assert!(out.contains("was 91 7d ago"));
        assert!(out.contains("SESSION NOTES"));
        // The notes column is narrow, so the text may wrap; check a contiguous run.
        assert!(out.contains("reviewed nginx"));
    }

    #[test]
    fn note_input_overlay_shows_when_typing() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.open_note();
        app.note_push_char('h');
        app.note_push_char('i');
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("New session note"));
        assert!(out.contains("hi_"));
    }

    #[test]
    fn logs_tab_shows_saved_searches() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(4); // Logs
        app.dense = true; // the analysis rail (saved searches) lives in dense mode
        app.state.add_search("nginx timeout");
        let out = render_to_string(&app, 130, 30);
        assert!(out.contains("SAVED SEARCHES"));
        assert!(out.contains("nginx timeout"));
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
    fn services_detail_shows_pid_deps_and_logs() {
        use systui_collectors::{LogEntry, ServiceUnit, UnitDetail};
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.all_units = vec![ServiceUnit {
            name: "nginx.service".to_owned(),
            load: "loaded".to_owned(),
            active: "active".to_owned(),
            sub: "running".to_owned(),
            description: "nginx web server".to_owned(),
        }];
        app.selected_unit_detail = Some(UnitDetail {
            name: "nginx.service".to_owned(),
            main_pid: Some(1832),
            unit_file_state: "enabled".to_owned(),
            fragment_path: "/lib/systemd/system/nginx.service".to_owned(),
            ..Default::default()
        });
        app.selected_unit_deps = vec!["network.target".to_owned(), "system.slice".to_owned()];
        app.selected_unit_logs = vec![LogEntry {
            time: "12:01:02".to_owned(),
            priority: 4,
            identifier: "nginx.service".to_owned(),
            message: "upstream timed out".to_owned(),
        }];
        app.view_state = ViewState::Ready;
        app.select_tab(3); // Services
        app.dense = true; // the detail pane lives in dense mode

        let out = render_to_string(&app, 120, 28);
        assert!(out.contains("main PID"));
        assert!(out.contains("1832"));
        assert!(out.contains("Dependencies"));
        assert!(out.contains("network.target"));
        assert!(out.contains("Recent logs"));
        assert!(out.contains("upstream timed out"));
        assert!(out.contains("nginx.service"));
    }

    #[test]
    fn services_tab_reports_no_failures() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(3); // Services

        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("no failed units"));
    }

    #[test]
    fn services_tab_shows_full_list_and_filter_bar() {
        use crate::app::ServiceFilter;
        let unit = |name: &str, active: &str, sub: &str| ServiceUnit {
            name: name.to_owned(),
            load: "loaded".to_owned(),
            active: active.to_owned(),
            sub: sub.to_owned(),
            description: String::new(),
        };
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;
        app.select_tab(3); // Services
        app.all_units = vec![
            unit("nginx.service", "active", "running"),
            unit("bluetooth.service", "inactive", "dead"),
        ];
        app.enabled_units = vec!["nginx.service".to_owned()];

        let out = render_to_string(&app, 100, 24);
        // Filter bar labels and the full list (not just failed) are shown.
        assert!(out.contains("ALL"));
        assert!(out.contains("ENABLED"));
        assert!(out.contains("nginx.service"));
        assert!(out.contains("bluetooth.service"));

        // Cycling to the INACTIVE filter narrows the list to dead units.
        app.service_filter = ServiceFilter::Inactive;
        let out = render_to_string(&app, 100, 24);
        assert!(out.contains("bluetooth.service"));
        assert!(!out.contains("nginx.service"));
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
        assert!(out.contains("HEALTH SCORE"));
        assert!(out.contains("72"));
        assert!(out.contains("/100"));
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
        app.dense = true; // interfaces/connections panels live in dense mode

        // The exposure map is information-dense; render wide (the prototype is 200 cols).
        let out = render_to_string(&app, 130, 34);
        assert!(out.contains("INTERFACES"));
        assert!(out.contains("eth0"));
        assert!(out.contains("192.168.1.1")); // gateway
        assert!(out.contains("EXPOSURE MAP"));
        assert!(out.contains("CRIT")); // redis on 0.0.0.0:6379
        assert!(out.contains("6379"));
        assert!(out.contains("redis.service"));
        // Connections panel lists the real established peer, not just a count.
        assert!(out.contains("CONNECTIONS"));
        assert!(out.contains("10.0.0.2:51000"));
    }

    #[test]
    fn network_clean_default_hides_secondary_panels() {
        use systui_collectors::exposure_map;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        let net = sample_network();
        app.exposures = exposure_map(&net.listeners);
        app.network = Some(net);
        app.view_state = ViewState::Ready;
        app.select_tab(5); // Network, clean default (dense off)

        let out = render_to_string(&app, 130, 30);
        // The primary exposure map is shown…
        assert!(out.contains("EXPOSURE MAP"));
        assert!(out.contains("6379"));
        // …but the secondary rail panels are hidden until dense mode.
        assert!(!out.contains("INTERFACES"));
        assert!(!out.contains("Connections"));
    }

    #[test]
    fn network_tab_shows_firewall_panel() {
        use systui_collectors::FirewallSnapshot;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.network = Some(sample_network());
        app.firewall = FirewallSnapshot {
            backend: "nftables".to_owned(),
            active: true,
            tables: vec!["inet filter".to_owned()],
            chains: vec!["input".to_owned(), "forward".to_owned()],
            rule_count: 6,
            notes: Vec::new(),
        };
        app.view_state = ViewState::Ready;
        app.select_tab(5); // Network
        app.dense = true; // firewall panel lives in dense mode

        // Render tall: a severity bar chart now bands the top of the Network tab.
        let out = render_to_string(&app, 130, 40);
        assert!(out.contains("FIREWALL"));
        assert!(out.contains("nftables"));
        assert!(out.contains("6 active"));
    }

    #[test]
    fn network_tab_shows_connectivity_panel() {
        use crate::app::ConnectivityResult;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.network = Some(sample_network());
        app.view_state = ViewState::Ready;
        app.select_tab(5); // Network
        app.dense = true; // connectivity panel lives in dense mode

        // Before running, the panel invites the user to probe.
        let out = render_to_string(&app, 130, 30);
        assert!(out.contains("CONNECTIVITY TESTS"));
        assert!(out.contains("press c to test"));

        // After a run, results are listed with their reachability.
        app.connectivity = vec![
            ConnectivityResult {
                target: "192.168.1.1".to_owned(),
                label: "gateway".to_owned(),
                reachable: true,
                detail: "0.4ms avg · 0% loss".to_owned(),
            },
            ConnectivityResult {
                target: "1.1.1.1".to_owned(),
                label: "dns".to_owned(),
                reachable: false,
                detail: "no reply".to_owned(),
            },
        ];
        let out = render_to_string(&app, 130, 30);
        assert!(out.contains("192.168.1.1"));
        assert!(out.contains("gateway"));
        assert!(out.contains("no reply"));
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
        assert!(out.contains("no findings"));
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
        assert!(out.contains("LVL"));
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
        app.dense = true; // risk checks + detail pane live in dense mode

        // Render tall: a CPU bar chart now bands the top of the Docker tab.
        let out = render_to_string(&app, 110, 40);
        assert!(out.contains("redis"));
        assert!(out.contains("redis:latest"));
        assert!(out.contains("unhealthy"));
        assert!(out.contains("RISK CHECKS"));
        // The privileged container surfaces in the detail panel and as a RISK badge.
        assert!(out.contains("privileged"));
        assert!(out.contains("RISK"));
        // Published ports now appear in the detail panel.
        assert!(out.contains("0.0.0.0:6379->6379/tcp"));
    }

    #[test]
    fn docker_tab_shows_compose_and_image_hygiene() {
        use systui_collectors::{ComposeProject, ImageHygiene};
        let mut app = App::new("local", ExecutionMode::Privileged);
        app.snapshot = Some(sample_snapshot());
        app.docker_available = true;
        app.containers = vec![sample_container()];
        app.container_inspects = vec![sample_inspect()];
        app.compose_projects = vec![ComposeProject {
            name: "acme-stack".to_owned(),
            status: "running(5)".to_owned(),
            config_files: "/srv/acme/docker-compose.yml".to_owned(),
            service_count: 5,
        }];
        app.image_hygiene = ImageHygiene {
            total_images: 23,
            total_size: "9.4GB".to_owned(),
            reclaimable: "2.1GB (22%)".to_owned(),
            dangling: 2,
        };
        app.view_state = ViewState::Ready;
        app.select_tab(6); // Docker
        app.dense = true; // compose + image hygiene panels live in dense mode

        // Render tall: a CPU bar chart now bands the top of the Docker tab.
        let out = render_to_string(&app, 120, 44);
        assert!(out.contains("COMPOSE PROJECTS"));
        assert!(out.contains("acme-stack"));
        assert!(out.contains("5 services"));
        assert!(out.contains("IMAGE HYGIENE"));
        assert!(out.contains("23 images"));
        assert!(out.contains("2 dangling"));
        // A writable mode invites the prune.
        assert!(out.contains("press p to prune"));
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
        app.dense = true; // preview/timers/summary panels live in dense mode

        let out = render_to_string(&app, 110, 36);
        assert!(out.contains("Every day at 02:00"));
        assert!(out.contains("2026-05-25 02:00")); // next run
        assert!(out.contains("/opt/backup.sh"));
        assert!(out.contains("logrotate.timer"));
        assert!(out.contains("warnings"));
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
        app.dense = true; // the operational detail pane lives in dense mode

        // Taller CRT header → render a bit taller so the dense panels all fit.
        let out = render_to_string(&app, 120, 40);
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
        assert!(out.contains("level"));
        assert!(out.contains("err+"));
        assert!(out.contains("search: n"));
    }

    #[test]
    fn log_fingerprints_group_lines_differing_only_in_numbers() {
        let entries = [
            LogEntry {
                time: "09:00:01".to_owned(),
                priority: 3,
                identifier: "sshd".to_owned(),
                message: "Failed password from 1.2.3.4 port 51220".to_owned(),
            },
            LogEntry {
                time: "09:00:09".to_owned(),
                priority: 3,
                identifier: "sshd".to_owned(),
                message: "Failed password from 1.2.3.4 port 51224".to_owned(),
            },
            LogEntry {
                time: "09:00:10".to_owned(),
                priority: 6, // info — excluded from fingerprints
                identifier: "systemd".to_owned(),
                message: "Started something".to_owned(),
            },
        ];
        let refs: Vec<&LogEntry> = entries.iter().collect();
        let fps = log_fingerprints(&refs);
        assert_eq!(fps.len(), 1);
        assert_eq!(fps[0].count, 2);
        assert_eq!(fps[0].first, "09:00:01");
        assert_eq!(fps[0].last, "09:00:09");
    }
}
