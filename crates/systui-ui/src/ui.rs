//! Rendering of the application frame. This is a pure function of [`App`], which
//! makes it testable headlessly with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, Wrap,
};
use regex::RegexBuilder;
use systui_collectors::{
    BindScope, Connection, Container, CronEntry, CronSource, DatabaseInstance, LogEntry,
    NetworkSnapshot, ServiceUnit, SystemSnapshot, parse_schedule,
};
use systui_core::{Finding, ModuleId, Severity};

use crate::app::{ActionStage, App, InputMode, ProcessView, ServiceFilter, Tab, ViewState};
use crate::theme::{Domain, Theme};
use crate::widgets::{StatusLevel, grid, status_card};

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, app: &App) {
    // Fill the whole frame with the theme background so the truecolor palette
    // covers gutters between panels.
    frame.render_widget(
        Block::default().style(Style::new().bg(app.theme.bg).fg(app.theme.fg)),
        frame.area(),
    );

    let rows = Layout::vertical([
        Constraint::Length(3), // top bar
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
        .border_type(BorderType::Rounded)
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

/// The 3-row top bar: brand + host-attached pill on the left, health gauge and
/// execution-mode badge on the right, inside a rounded panel (spec §13).
fn render_top_bar(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(app.theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Brand + host pill on a single, vertically-centered line.
    let mut left = vec![
        Span::styled(
            "SysTUI",
            Style::new()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}  ", env!("CARGO_PKG_VERSION")),
            Style::new().fg(app.theme.fg_dim),
        ),
    ];
    // Host-attached pill: status dot · label · transport/mode.
    let attached = app.snapshot.is_some();
    let dot_color = match &app.health {
        Some(h) if h.score >= 80 => app.theme.accent,
        Some(h) if h.score >= 50 => app.theme.high,
        Some(_) => app.theme.critical,
        None => app.theme.fg_dim,
    };
    left.push(Span::styled("● ", Style::new().fg(dot_color)));
    left.push(Span::styled(
        app.host_label.clone(),
        Style::new()
            .fg(app.theme.fg_strong)
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(caps) = &app.capabilities {
        left.push(Span::styled(
            format!("  {}", caps.label()),
            Style::new().fg(app.theme.fg_muted),
        ));
    }
    left.push(Span::styled(
        if attached { " · live" } else { " · detached" },
        Style::new().fg(app.theme.fg_dim),
    ));
    if app.refreshing {
        left.push(Span::styled(
            " · ⟳ refreshing",
            Style::new().fg(app.theme.accent),
        ));
    }

    let (badge_text, badge_color) = mode_badge(&app.theme, app.mode);
    let health_score = app.health.as_ref().map(|h| h.score);

    // Right side: HEALTH nn/100 [gauge]  [MODE]. Built as spans so it lives on
    // the same centered row as the brand/host pill.
    let mut right: Vec<Span> = Vec::new();
    if let Some(score) = health_score {
        let color = score_color(app, score);
        right.push(Span::styled("HEALTH ", Style::new().fg(app.theme.fg_muted)));
        right.push(Span::styled(
            format!("{score}"),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));
        right.push(Span::styled("/100 ", Style::new().fg(app.theme.fg_dim)));
        right.push(Span::styled(
            gauge_bar(score as f64, 10),
            Style::new().fg(color),
        ));
        right.push(Span::raw("  "));
    }
    right.push(Span::styled(
        format!(" {badge_text} "),
        Style::new()
            .fg(app.theme.bg)
            .bg(badge_color)
            .add_modifier(Modifier::BOLD),
    ));

    // Vertically center within the 3-row panel (inner is 1 row tall here).
    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width(&right))])
        .split(inner);
    frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        cols[1],
    );
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

/// Approximate rendered width of a span run, for right-alignment sizing.
fn right_width(spans: &[Span]) -> u16 {
    spans
        .iter()
        .map(|s| s.content.chars().count() as u16)
        .sum::<u16>()
        .saturating_add(1)
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

/// A solid block-character gauge of `width` cells filled to `percent`.
fn gauge_bar(percent: f64, width: usize) -> String {
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width);
    s.extend(std::iter::repeat_n('█', filled));
    s.extend(std::iter::repeat_n('░', width - filled));
    s
}

/// Numbered tab bar with per-tab count badges (spec §13/§14).
fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let active = i == app.active_tab;
        let key = if i < 9 { (b'1' + i as u8) as char } else { '0' };
        spans.push(Span::styled(
            format!("{key} "),
            Style::new().fg(app.theme.fg_dim),
        ));
        let name_style = if active {
            Style::new()
                .fg(app.theme.domain(tab.domain()))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new().fg(app.theme.fg_muted)
        };
        spans.push(Span::styled(tab.title(), name_style));
        if let Some((count, color)) = tab_badge(app, *tab) {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(" {count} "),
                Style::new()
                    .fg(app.theme.bg)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled("  ", Style::new().fg(app.theme.fg_dim)));
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
    // Clean by default: the process table full-width. Dense adds the detail pane.
    if !app.dense {
        render_process_list(frame, app, area);
        return;
    }
    let cols =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
    render_process_list(frame, app, cols[0]);
    render_process_detail(frame, app, cols[1]);
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
        let risk = if e.severity > Severity::Info {
            Span::styled(
                format!(" {} ", severity_abbr(e.severity)),
                Style::new()
                    .fg(t.bg)
                    .bg(t.severity(e.severity))
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("ok", Style::new().fg(t.fg_dim))
        };
        Row::new([
            Cell::from(Span::styled(proto, Style::new().fg(t.fg_muted))),
            Cell::from(Span::styled(
                format!("{ip}:{}", e.listener.port),
                Style::new().fg(addr_color),
            )),
            Cell::from(Span::styled(owner, Style::new().fg(t.fg))),
            Cell::from(Span::styled(service, Style::new().fg(t.fg_muted))),
            Cell::from(risk),
        ])
    });
    let widths = [
        Constraint::Length(5),
        Constraint::Length(20),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(7),
    ];
    frame.render_widget(
        Table::new(body, widths)
            .header(header)
            .style(Style::new().fg(t.fg)),
        inner,
    );
}

/// A short severity tag for tight badge columns: CRIT/HIGH/MED/LOW/INFO.
fn severity_abbr(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "CRIT",
        Severity::High => "HIGH",
        Severity::Medium => "MED",
        Severity::Low => "LOW",
        Severity::Info => "INFO",
    }
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
    // Clean by default: the scheduled-jobs table full-width. Dense adds the
    // schedule preview, systemd timers and the cron-health summary.
    if !app.dense {
        render_cron_table(frame, app, area);
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).split(area);
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
    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);
    render_security_header(frame, app, rows[0]);
    render_security_findings(frame, app, rows[1]);
}

/// Severity-counter header band: one tile per severity with a big count.
fn render_security_header(frame: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let block = panel_block(&t, "Security findings", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let counts = app.finding_counts();
    let labels = ["CRITICAL", "HIGH", "MEDIUM", "LOW", "INFO"];
    let colors = [t.critical, t.high, t.medium, t.low, t.fg_muted];

    let split = Layout::horizontal([Constraint::Min(0), Constraint::Length(22)]).split(inner);
    let cells = Layout::horizontal([Constraint::Ratio(1, 5); 5]).split(split[0]);
    for (i, label) in labels.iter().enumerate() {
        let n = counts[i];
        let count_color = if n > 0 { colors[i] } else { t.fg_dim };
        let lines = vec![
            Line::from(Span::styled(
                format!("{n}"),
                Style::new().fg(count_color).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(*label, Style::new().fg(t.fg_dim))),
        ];
        frame.render_widget(Paragraph::new(lines), cells[i]);
    }

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
            "no change vs last week".to_owned()
        } else {
            format!("{arrow}{word} from last week")
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, Style::new().fg(color))))
                .alignment(Alignment::Right),
            split[1],
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
fn render_dashboard(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let cols =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
    let left = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(cols[0]);
    let right = Layout::vertical([
        Constraint::Length(12),
        Constraint::Min(0),
        Constraint::Length(6),
    ])
    .split(cols[1]);

    render_metric_tiles(frame, app, snap, left[0]);
    render_domain_cards(frame, app, snap, left[1]);
    render_health_panel(frame, app, right[0]);
    render_findings_panel(frame, app, right[1]);
    render_session_notes(frame, app, right[2]);
}

/// Map a 0–100 utilisation to a cockpit status band.
fn usage_status(percent: f64) -> StatusLevel {
    if percent >= 85.0 {
        StatusLevel::Crit
    } else if percent >= 60.0 {
        StatusLevel::Warn
    } else {
        StatusLevel::Ok
    }
}

/// A per-domain status card with its computed verdict.
struct DomainCard {
    accent: ratatui::style::Color,
    title: &'static str,
    status: StatusLevel,
    headline: String,
    detail: String,
}

/// The cockpit's per-domain status cards: one accented card per area, each with
/// a status dot and a one-line, plain-language verdict derived from the real
/// snapshot. Replaces a dense text grid — the landing answers "is this host OK?"
/// at a glance; detail lives one keystroke away in each tab.
fn render_domain_cards(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let block = panel_block(&app.theme, "Status", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cards = domain_cards(app, snap);
    let cells = grid(inner, 4, cards.len());
    for (card, cell) in cards.iter().zip(cells.iter()) {
        // Clean by default: just the verdict. Dense mode reveals the secondary
        // breakdown/totals line.
        let detail = if app.dense { card.detail.as_str() } else { "" };
        status_card(
            frame,
            &app.theme,
            *cell,
            card.accent,
            card.title,
            card.status,
            &card.headline,
            detail,
        );
    }
}

/// Build the cockpit verdicts. Every value comes from a real collector; cards for
/// areas with no data report an idle state rather than inventing numbers.
fn domain_cards(app: &App, snap: &SystemSnapshot) -> Vec<DomainCard> {
    let mut cards = Vec::new();

    // Services: failed units (the only service tier always collected).
    let failed = app.failed_units.len();
    cards.push(DomainCard {
        accent: app.theme.domain(Domain::Services),
        title: "Services",
        status: if failed > 0 {
            StatusLevel::Crit
        } else {
            StatusLevel::Ok
        },
        headline: if failed > 0 {
            format!("{failed} failed")
        } else {
            "all up".to_owned()
        },
        detail: String::new(),
    });

    // Network: externally reachable, sensitive exposures.
    let risky = app.risky_exposure_count();
    let listening = app.exposures.len();
    cards.push(if app.network.is_none() && listening == 0 {
        DomainCard {
            accent: app.theme.domain(Domain::Network),
            title: "Network",
            status: StatusLevel::Idle,
            headline: "no data".to_owned(),
            detail: String::new(),
        }
    } else {
        DomainCard {
            accent: app.theme.domain(Domain::Network),
            title: "Network",
            status: if risky > 0 {
                StatusLevel::Crit
            } else {
                StatusLevel::Ok
            },
            headline: if risky > 0 {
                format!("{risky} risky")
            } else {
                "no risky ports".to_owned()
            },
            detail: format!("{listening} listening"),
        }
    });

    // Docker: running containers (informational; risks live in the Docker tab).
    let running = app.containers.iter().filter(|c| c.is_running()).count();
    cards.push(if !app.docker_available {
        DomainCard {
            accent: app.theme.domain(Domain::Docker),
            title: "Docker",
            status: StatusLevel::Idle,
            headline: "not installed".to_owned(),
            detail: String::new(),
        }
    } else {
        DomainCard {
            accent: app.theme.domain(Domain::Docker),
            title: "Docker",
            status: if app.containers.is_empty() {
                StatusLevel::Idle
            } else {
                StatusLevel::Ok
            },
            headline: format!("{running} running"),
            detail: format!("{} total", app.containers.len()),
        }
    });

    // Security: active findings by severity.
    let [crit, high, med, ..] = app.finding_counts();
    cards.push(DomainCard {
        accent: app.theme.domain(Domain::Security),
        title: "Security",
        status: if crit > 0 {
            StatusLevel::Crit
        } else if high > 0 {
            StatusLevel::Warn
        } else {
            StatusLevel::Ok
        },
        headline: if crit > 0 {
            format!("{crit} critical")
        } else if high > 0 {
            format!("{high} high")
        } else {
            "no critical".to_owned()
        },
        detail: format!("{high} high · {med} med"),
    });

    // Logs: error/critical lines in the visible window.
    let errors = app.logs.iter().filter(|e| e.is_error()).count();
    cards.push(if app.logs.is_empty() {
        DomainCard {
            accent: app.theme.domain(Domain::Logs),
            title: "Logs",
            status: StatusLevel::Idle,
            headline: "no logs".to_owned(),
            detail: String::new(),
        }
    } else {
        DomainCard {
            accent: app.theme.domain(Domain::Logs),
            title: "Logs",
            status: if errors > 0 {
                StatusLevel::Warn
            } else {
                StatusLevel::Ok
            },
            headline: if errors > 0 {
                format!("{errors} errors")
            } else {
                "no errors".to_owned()
            },
            detail: "in window".to_owned(),
        }
    });

    // Crons: scheduled jobs + systemd timers (informational).
    let jobs = app.crons.len();
    let timers = app.timers.len();
    cards.push(DomainCard {
        accent: app.theme.domain(Domain::Crons),
        title: "Crons",
        status: if jobs == 0 && timers == 0 {
            StatusLevel::Idle
        } else {
            StatusLevel::Ok
        },
        headline: format!("{jobs} jobs"),
        detail: format!("{timers} timers"),
    });

    // Databases: detected instances, flagging externally reachable ones.
    let dbs = app.databases.instances.len();
    let exposed = app
        .databases
        .instances
        .iter()
        .filter(|i| i.is_externally_reachable())
        .count();
    cards.push(if dbs == 0 {
        DomainCard {
            accent: app.theme.domain(Domain::Databases),
            title: "Databases",
            status: StatusLevel::Idle,
            headline: "none".to_owned(),
            detail: String::new(),
        }
    } else {
        DomainCard {
            accent: app.theme.domain(Domain::Databases),
            title: "Databases",
            status: if exposed > 0 {
                StatusLevel::Crit
            } else {
                StatusLevel::Ok
            },
            headline: if exposed > 0 {
                format!("{exposed} exposed")
            } else {
                format!("{dbs} instances")
            },
            detail: if exposed > 0 {
                format!("{dbs} instances")
            } else {
                String::new()
            },
        }
    });

    // System: load relative to core count, plus uptime/users context.
    let load_pct = if snap.cpu.cores > 0 {
        (snap.load.one / snap.cpu.cores as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    cards.push(DomainCard {
        accent: app.theme.domain(Domain::System),
        title: "System",
        status: usage_status(load_pct),
        headline: format!("load {:.2}", snap.load.one),
        detail: format!(
            "up {} · {} users",
            human_uptime(snap.uptime_secs),
            snap.users.len()
        ),
    });

    // Updates: pending package updates, security ones highlighted. No domain hue
    // (it spans the host), so it uses the brand accent.
    let pkg = &app.packages;
    cards.push(if !pkg.available {
        DomainCard {
            accent: app.theme.accent,
            title: "Updates",
            status: StatusLevel::Idle,
            headline: "n/a".to_owned(),
            detail: String::new(),
        }
    } else {
        DomainCard {
            accent: app.theme.accent,
            title: "Updates",
            status: if pkg.security > 0 {
                StatusLevel::Crit
            } else if pkg.pending > 0 {
                StatusLevel::Warn
            } else {
                StatusLevel::Ok
            },
            headline: if pkg.security > 0 {
                format!("{} security", pkg.security)
            } else if pkg.pending > 0 {
                format!("{} pending", pkg.pending)
            } else {
                "up to date".to_owned()
            },
            detail: if pkg.security > 0 {
                format!("{} pending", pkg.pending)
            } else {
                String::new()
            },
        }
    });

    cards
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

/// A rounded, titled panel block in the theme's surface style.
fn panel_block(theme: &Theme, title: &str, accent: ratatui::style::Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {title} "),
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

/// The four top metric tiles: CPU, RAM, DISK, LOAD.
fn render_metric_tiles(frame: &mut Frame, app: &App, snap: &SystemSnapshot, area: Rect) {
    let cells = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);

    metric_tile(
        frame,
        &app.theme,
        cells[0],
        "CPU",
        format!("{:.0}", snap.cpu.busy_percent),
        "%",
        format!("{} cores", snap.cpu.cores),
        Some(&app.cpu_history),
        snap.cpu.busy_percent,
    );
    metric_tile(
        frame,
        &app.theme,
        cells[1],
        "RAM",
        format!("{:.0}", snap.memory.used_percent()),
        "%",
        format!(
            "{} / {}",
            human_kb(snap.memory.used_kb()),
            human_kb(snap.memory.total_kb)
        ),
        Some(&app.mem_history),
        snap.memory.used_percent(),
    );

    let worst = snap.disks.iter().max_by_key(|d| d.use_percent);
    let (dvalue, dsub, dpct) = match worst {
        Some(d) => (
            format!("{}", d.use_percent),
            format!("{} {}", d.mount, human_kb(d.size_kb)),
            d.use_percent as f64,
        ),
        None => ("–".to_owned(), "no disks".to_owned(), 0.0),
    };
    metric_tile(
        frame, &app.theme, cells[2], "DISK", dvalue, "%", dsub, None, dpct,
    );

    let load_pct = if snap.cpu.cores > 0 {
        (snap.load.one / snap.cpu.cores as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    metric_tile(
        frame,
        &app.theme,
        cells[3],
        "LOAD",
        format!("{:.2}", snap.load.one),
        "",
        format!(
            "{:.2} {:.2} · {} cores",
            snap.load.five, snap.load.fifteen, snap.cpu.cores
        ),
        None,
        load_pct,
    );
}

/// One metric tile: label, big value, sparkline-or-gauge, sub-line.
#[allow(clippy::too_many_arguments)]
fn metric_tile(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    label: &str,
    value: String,
    unit: &str,
    sub: String,
    history: Option<&[u64]>,
    percent: f64,
) {
    let block = panel_block(theme, label, theme.accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let color = usage_color(theme, percent);
    let rows = Layout::vertical([
        Constraint::Length(1), // value
        Constraint::Length(1), // sparkline / gauge
        Constraint::Min(0),    // sub
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                value,
                Style::new()
                    .fg(theme.fg_strong)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {unit}"), Style::new().fg(theme.fg_muted)),
        ])),
        rows[0],
    );

    match history {
        Some(h) if !h.is_empty() => {
            frame.render_widget(
                Sparkline::default()
                    .data(h)
                    .max(100)
                    .style(Style::new().fg(color)),
                rows[1],
            );
        }
        _ => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    gauge_bar(percent, rows[1].width as usize),
                    Style::new().fg(color),
                )),
                rows[1],
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Span::styled(sub, Style::new().fg(theme.fg_dim))).wrap(Wrap { trim: true }),
        rows[2],
    );
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
    let mut score_line = vec![
        Span::styled(
            format!("{}", health.score),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" /100  ", Style::new().fg(app.theme.fg_dim)),
        Span::styled(gauge_bar(health.score as f64, 16), Style::new().fg(color)),
    ];
    // Trend vs ~7 days ago, from the persisted snapshots (fills over time).
    if let Some(base) = app.state.baseline(&app.host_label, app.now.date(), 7) {
        score_line.push(Span::styled(
            format!("  was {} 7d ago", base.score),
            Style::new().fg(app.theme.fg_dim),
        ));
    }
    let mut lines = vec![Line::from(score_line), Line::from("")];
    if health.checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no deductions — healthy",
            Style::new().fg(app.theme.accent),
        )));
    } else {
        for check in health.checks.iter().take(7) {
            lines.push(finding_line(app, check));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Worst-first findings list with severity-colored left edge bars (spec §14).
fn render_findings_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel_block(&app.theme, "Top findings", app.domain_color());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let t = app.theme;

    if app.findings.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no findings — clean",
                Style::new().fg(t.accent),
            )),
            inner,
        );
        return;
    }

    let max = (inner.height as usize / 2).max(1);
    let mut lines: Vec<Line> = Vec::new();
    for f in app.findings.iter().take(max) {
        let color = t.severity(f.severity);
        let sev = format!("{:?}", f.severity).to_uppercase();
        lines.push(Line::from(vec![
            Span::styled("▌ ", Style::new().fg(color)),
            Span::styled(
                format!("{sev:<5} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(f.title.clone(), Style::new().fg(t.fg_strong)),
        ]));
        // Prefer evidence; fall back to the recommendation; never leak the raw id.
        if let Some(evidence) = f.evidence.first() {
            lines.push(Line::from(Span::styled(
                format!("    {evidence}"),
                Style::new().fg(t.fg_muted),
            )));
        } else if !f.recommendation.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    → {}", f.recommendation),
                Style::new().fg(t.fg_dim),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
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

    // Clean by default: identity + memory, the "what is this host" essentials.
    // Dense adds the disks and logged-in users panels.
    if app.dense {
        let left = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).split(cols[0]);
        let right = Layout::vertical([Constraint::Length(7), Constraint::Min(0)]).split(cols[1]);
        render_system_identity(frame, app, snap, left[0]);
        render_system_disks(frame, app, snap, left[1]);
        render_system_memory(frame, app, snap, right[0]);
        render_system_users(frame, app, snap, right[1]);
    } else {
        render_system_identity(frame, app, snap, cols[0]);
        render_system_memory(frame, app, snap, cols[1]);
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

    let bar_w = (inner.width as usize).saturating_sub(13).clamp(4, 24);
    let mut lines = vec![
        gauge_line(&t, "RAM", snap.memory.used_percent(), bar_w),
        Line::from(Span::styled(
            format!(
                "        {} / {}",
                human_kb(snap.memory.used_kb()),
                human_kb(snap.memory.total_kb)
            ),
            Style::new().fg(t.fg_dim),
        )),
    ];
    if snap.swap.total_kb > 0 {
        lines.push(gauge_line(&t, "Swap", snap.swap.used_percent(), bar_w));
        lines.push(Line::from(Span::styled(
            format!(
                "        {} / {}",
                human_kb(snap.swap.used_kb()),
                human_kb(snap.swap.total_kb)
            ),
            Style::new().fg(t.fg_dim),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  Swap  ", Style::new().fg(t.dim)),
            Span::styled("none", Style::new().fg(t.fg_dim)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A "label ████░░ NN%" gauge line, coloured by utilisation band.
fn gauge_line(theme: &Theme, label: &str, percent: f64, width: usize) -> Line<'static> {
    let color = usage_color(theme, percent);
    Line::from(vec![
        Span::styled(format!("  {label:<5} "), Style::new().fg(theme.dim)),
        Span::styled(gauge_bar(percent, width), Style::new().fg(color)),
        Span::styled(format!(" {percent:.0}%"), Style::new().fg(theme.fg)),
    ])
}

/// Disks: per-mount usage with a colour-coded gauge.
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

    let header = Row::new(["MOUNT", "USE", "USED / SIZE", "FS"])
        .style(Style::new().fg(t.fg_dim).add_modifier(Modifier::BOLD));
    let bar_w = 10usize;
    let rows = snap.disks.iter().map(|d| {
        let pct = d.use_percent as f64;
        let color = usage_color(&t, pct);
        Row::new(vec![
            Cell::from(d.mount.clone()),
            Cell::from(Line::from(vec![
                Span::styled(gauge_bar(pct, bar_w), Style::new().fg(color)),
                Span::styled(format!(" {:>3}%", d.use_percent), Style::new().fg(color)),
            ])),
            Cell::from(format!("{} / {}", human_kb(d.used_kb), human_kb(d.size_kb))),
            Cell::from(d.filesystem.clone()),
        ])
    });
    let widths = [
        Constraint::Length(14),
        Constraint::Length(bar_w as u16 + 5),
        Constraint::Length(20),
        Constraint::Min(6),
    ];
    frame.render_widget(
        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .style(Style::new().fg(t.fg)),
        inner,
    );
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
            ("↵", "apply"),
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

    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(40)]).split(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), cols[0]);

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
        Span::styled("V ", key_style),
        Span::styled(
            format!("{}  ", app.visual_style.label()),
            Style::new().fg(app.theme.fg_dim),
        ),
        Span::styled("T ", key_style),
        Span::styled(
            format!("{}  ", app.theme_kind.label()),
            Style::new().fg(app.theme.fg_dim),
        ),
        Span::styled(format!("{dot} "), Style::new().fg(color)),
        Span::styled(format!("{label} "), Style::new().fg(app.theme.fg_muted)),
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
        ("T", "cycle theme (dark / midnight / light)"),
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
        // 120 cols so the full 10-tab bar (through "Security") is visible.
        let out = render_to_string(&app, 120, 24);
        assert!(out.contains("SysTUI"));
        assert!(out.contains("prod-01"));
        assert!(out.contains("READ-ONLY"));
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
        // The hostname now lives in the chrome (host label "local"); the body
        // shows the metric tiles for the snapshot.
        assert!(out.contains("local"));
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
        assert!(out.contains("Memory"));
        assert!(out.contains("Disks"));
        assert!(out.contains("Logged in"));
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
                command: "systemd".into(),
            },
            Process {
                pid: 880,
                ppid: 1,
                user: "root".into(),
                cpu_percent: 0.4,
                mem_percent: 1.1,
                command: "sshd".into(),
            },
            Process {
                pid: 3300,
                ppid: 880,
                user: "admin".into(),
                cpu_percent: 12.4,
                mem_percent: 0.8,
                command: "node".into(),
            },
        ];
        app.view_state = ViewState::Ready;
        app.select_tab(2);
        app.dense = true; // the process detail pane lives in dense mode

        // Detail panel reflects the selected process (top of the CPU sort: node).
        let out = render_to_string(&app, 110, 18);
        assert!(out.contains("Process")); // detail panel title
        assert!(out.contains("Parent")); // parent row
        assert!(out.contains("sshd (880)")); // node's parent resolved by ppid

        // Tree view indents children and exposes the child count in the detail.
        app.toggle_process_view();
        assert_eq!(app.process_view, ProcessView::Tree);
        assert_eq!(app.processes_selected, 0); // reset on toggle → systemd
        let tree = render_to_string(&app, 110, 18);
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
    fn dashboard_cockpit_shows_domain_status_cards() {
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.view_state = ViewState::Ready;

        let out = render_to_string(&app, 120, 36);
        // The cockpit replaces the dense at-a-glance grid with accented cards.
        assert!(out.contains("Status"));
        assert!(out.contains("Services"));
        assert!(out.contains("Security"));
        // With no failures collected, Services reports a clean verdict.
        assert!(out.contains("all up"));
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
        assert!(out.contains("Session notes"));
        assert!(out.contains("reviewed nginx errors"));
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
        assert!(out.contains("Saved searches"));
        assert!(out.contains("nginx timeout"));
    }

    #[test]
    fn dashboard_updates_card_highlights_security() {
        use systui_collectors::PackageUpdates;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.packages = PackageUpdates {
            manager: "apt".to_owned(),
            pending: 23,
            security: 2,
            available: true,
        };
        app.view_state = ViewState::Ready;
        app.dense = true; // the secondary "pending" detail shows in dense mode

        let out = render_to_string(&app, 120, 30);
        assert!(out.contains("Updates"));
        assert!(out.contains("2 security"));
        assert!(out.contains("23 pending"));
    }

    #[test]
    fn dense_mode_reveals_card_detail() {
        use systui_collectors::PackageUpdates;
        let mut app = App::new("local", ExecutionMode::ReadOnly);
        app.snapshot = Some(sample_snapshot());
        app.packages = PackageUpdates {
            manager: "apt".to_owned(),
            pending: 23,
            security: 2,
            available: true,
        };
        app.view_state = ViewState::Ready;

        // Clean default: only the verdict headline, no secondary detail.
        let clean = render_to_string(&app, 120, 30);
        assert!(clean.contains("2 security"));
        assert!(!clean.contains("23 pending"));

        // Dense reveals the breakdown/totals line.
        app.dense = true;
        let dense = render_to_string(&app, 120, 30);
        assert!(dense.contains("23 pending"));
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

        // The multi-panel dashboard (incl. the session-notes panel) targets a
        // tall terminal like the prototype's; render at a realistic height.
        let out = render_to_string(&app, 120, 36);
        assert!(out.contains("Services"));
        assert!(out.contains("1 failed"));
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
        assert!(out.contains("Health score"));
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
        let out = render_to_string(&app, 130, 30);
        assert!(out.contains("Interfaces"));
        assert!(out.contains("eth0"));
        assert!(out.contains("192.168.1.1")); // gateway
        assert!(out.contains("Exposure map"));
        assert!(out.contains("CRIT")); // redis on 0.0.0.0:6379
        assert!(out.contains("6379"));
        assert!(out.contains("redis.service"));
        // Connections panel lists the real established peer, not just a count.
        assert!(out.contains("Connections"));
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
        assert!(out.contains("Exposure map"));
        assert!(out.contains("6379"));
        // …but the secondary rail panels are hidden until dense mode.
        assert!(!out.contains("Interfaces"));
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

        let out = render_to_string(&app, 130, 30);
        assert!(out.contains("Firewall"));
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
        assert!(out.contains("Connectivity tests"));
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

        // The cockpit's findings rail targets a wide terminal like the prototype.
        let out = render_to_string(&app, 120, 40);
        // The Security cockpit card summarises the critical finding…
        assert!(out.contains("Security"));
        assert!(out.contains("1 critical"));
        // …and the finding itself surfaces in the Top findings panel.
        assert!(out.contains("Sensitive service exposed"));
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

        let out = render_to_string(&app, 110, 30);
        assert!(out.contains("redis"));
        assert!(out.contains("redis:latest"));
        assert!(out.contains("unhealthy"));
        assert!(out.contains("Risk checks"));
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

        let out = render_to_string(&app, 120, 36);
        assert!(out.contains("Compose projects"));
        assert!(out.contains("acme-stack"));
        assert!(out.contains("5 services"));
        assert!(out.contains("Image hygiene"));
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
