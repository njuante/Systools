//! Shared visual building blocks for the cockpit and drill-down screens.
//!
//! These are pure helpers over a [`Theme`] — they take a `Frame` and an area and
//! draw, so the render layer stays a pure function of `App`. They belong to the
//! "sober" visual style; the "rich" style will provide ornate variants later.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::theme::Theme;

/// A domain's health at a glance, mapped to a color and a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLevel {
    /// Healthy / nothing to do.
    Ok,
    /// Worth a look, not urgent.
    Warn,
    /// Needs attention now.
    Crit,
    /// Not applicable / not installed / no data yet.
    Idle,
}

impl StatusLevel {
    /// The status glyph (filled dot for live states, hollow for idle).
    pub const fn dot(self) -> &'static str {
        match self {
            StatusLevel::Idle => "○",
            _ => "●",
        }
    }

    /// The status color from the active theme.
    pub fn color(self, theme: &Theme) -> Color {
        match self {
            StatusLevel::Ok => theme.accent,
            StatusLevel::Warn => theme.high,
            StatusLevel::Crit => theme.critical,
            StatusLevel::Idle => theme.fg_dim,
        }
    }
}

/// A cockpit status card: a domain-accented title with a status dot, a bold
/// headline verdict colored by status, and an optional dim detail line.
#[allow(clippy::too_many_arguments)]
pub fn status_card(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    accent: Color,
    title: &str,
    status: StatusLevel,
    headline: &str,
    detail: &str,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let status_color = status.color(theme);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", status.dot()), Style::new().fg(status_color)),
        Span::styled(
            headline.to_owned(),
            Style::new().fg(status_color).add_modifier(Modifier::BOLD),
        ),
    ])];
    if !detail.is_empty() {
        lines.push(Line::from(Span::styled(
            detail.to_owned(),
            Style::new().fg(theme.fg_dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Lay an area out into a grid of `cols` columns and as many rows as needed for
/// `count` cells, returning the per-cell rects in row-major order. Rows share the
/// height evenly. Useful for card grids.
pub fn grid(area: Rect, cols: usize, count: usize) -> Vec<Rect> {
    if cols == 0 || count == 0 {
        return Vec::new();
    }
    let rows = count.div_ceil(cols);
    let row_rects = Layout::vertical(vec![Constraint::Ratio(1, rows as u32); rows]).split(area);
    let mut cells = Vec::with_capacity(count);
    for row in row_rects.iter() {
        let col_rects =
            Layout::horizontal(vec![Constraint::Ratio(1, cols as u32); cols]).split(*row);
        for col in col_rects.iter() {
            cells.push(*col);
            if cells.len() == count {
                return cells;
            }
        }
    }
    cells
}
