//! The fleet overview TUI: a read-only, selectable table of inventory hosts,
//! worst-first (`Product.md` §4.16). It renders an already-gathered
//! [`FleetOverview`] and lets the operator drill into a host. It never executes
//! actions; gathering and the per-host drill-in are driven by the caller, which
//! owns the transports.

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::{DefaultTerminal, Frame};
use systui_core::Result;
use systui_report::{FleetOutcome, FleetOverview, findings_summary};

use crate::Theme;

/// How often the event loop wakes to poll for input.
const TICK: Duration = Duration::from_millis(250);

/// What the operator chose from the fleet overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetExit {
    /// Leave fleet mode.
    Quit,
    /// Re-gather the fleet.
    Refresh,
    /// Drill into the host with this inventory id.
    Enter(String),
}

/// Run the fleet overview TUI over an already-gathered [`FleetOverview`].
///
/// Returns the operator's choice. The caller handles re-gathering on
/// [`FleetExit::Refresh`] and launching the per-host TUI on [`FleetExit::Enter`],
/// then re-enters this view. Sets up and restores the terminal around the loop.
pub fn run_fleet(overview: &FleetOverview) -> Result<FleetExit> {
    let mut terminal = ratatui::try_init()?;
    let result = fleet_loop(&mut terminal, overview);
    let _ = ratatui::try_restore();
    result
}

fn fleet_loop(terminal: &mut DefaultTerminal, overview: &FleetOverview) -> Result<FleetExit> {
    let theme = Theme::dark();
    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| render_fleet(frame, overview, &theme, selected))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(FleetExit::Quit);
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(FleetExit::Quit),
                    KeyCode::Char('r') => return Ok(FleetExit::Refresh),
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = next_index(selected, overview.hosts.len());
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = prev_index(selected, overview.hosts.len());
                    }
                    KeyCode::Enter => {
                        if let Some(host) = overview.hosts.get(selected) {
                            return Ok(FleetExit::Enter(host.id.clone()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
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

/// Render the fleet overview frame. A pure function of the overview and the
/// selected index, so it is testable headlessly with ratatui's `TestBackend`.
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
        " \u{2191}/\u{2193} select \u{00b7} Enter open host \u{00b7} r refresh \u{00b7} q quit ",
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
                "No hosts in the inventory.",
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

    #[test]
    fn index_navigation_wraps() {
        assert_eq!(next_index(0, 3), 1);
        assert_eq!(next_index(2, 3), 0);
        assert_eq!(prev_index(0, 3), 2);
        assert_eq!(prev_index(1, 3), 0);
        // Empty list never moves off zero.
        assert_eq!(next_index(0, 0), 0);
        assert_eq!(prev_index(0, 0), 0);
    }

    #[test]
    fn truncate_caps_long_text() {
        assert_eq!(truncate("short", 48), "short");
        assert_eq!(truncate("abcdef", 4).chars().count(), 4);
    }

    #[test]
    fn renders_hosts_worst_first() {
        let backend = TestBackend::new(80, 10);
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
        assert!(rendered.contains("1 reviewed"));
        assert!(rendered.contains("1 unreachable"));
        // The failed host (worst) is listed before the reviewed one.
        let db = rendered.find("db-01").unwrap();
        let prod = rendered.find("prod-01").unwrap();
        assert!(db < prod);
    }
}
