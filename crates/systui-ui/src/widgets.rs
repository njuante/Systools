//! Shared visual building blocks for the cockpit and drill-down screens.
//!
//! These are pure helpers over a [`Theme`] — they take a `Frame` and an area and
//! draw, so the render layer stays a pure function of `App`. They belong to the
//! "sober" visual style; the "rich" style will provide ornate variants later.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Bar, BarChart, BarGroup, Block, BorderType, Borders, Chart, Dataset, Gauge, GraphType,
    Paragraph, Sparkline, Wrap,
};

use crate::theme::Theme;
use crate::visual_style::VisualStyle;

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
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
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

/// A labelled meter: a titled card holding a filled [`Gauge`] bar (colored by
/// `color`) with the reading drawn on top. Far more visual than a number + text.
pub fn meter_gauge(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    percent: f64,
    reading: &str,
    color: Color,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let gauge = Gauge::default()
        .gauge_style(Style::new().fg(color).bg(theme.bg_elev))
        .ratio((percent.clamp(0.0, 100.0)) / 100.0)
        .label(Span::styled(
            reading.to_owned(),
            Style::new()
                .fg(theme.fg_strong)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, inner);
}

/// A single row: a fixed-width text label followed by an inline filled
/// [`Gauge`] bar (colored by `color`) with `reading` drawn on it. Use it to
/// turn rows of "label NN%" text into real gauge bars.
#[allow(clippy::too_many_arguments)]
pub fn labeled_gauge(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    label: &str,
    label_width: u16,
    percent: f64,
    reading: &str,
    color: Color,
) {
    let cells = Layout::horizontal([Constraint::Length(label_width), Constraint::Min(4)]).split(area);
    frame.render_widget(
        Paragraph::new(Span::styled(label.to_owned(), Style::new().fg(theme.fg_muted))),
        cells[0],
    );
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::new().fg(color).bg(theme.bg_elev))
            .ratio(percent.clamp(0.0, 100.0) / 100.0)
            .label(Span::styled(
                reading.to_owned(),
                Style::new().fg(theme.fg_strong),
            )),
        cells[1],
    );
}

/// A time-series panel for two %-series (e.g. CPU and RAM history). In the
/// **Rich** style it draws real braille line charts; in **Sober** it stacks two
/// labelled [`Sparkline`]s. Either way it is a graph, not a table of numbers.
pub fn history_chart(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    style: VisualStyle,
    title: &str,
    cpu: &[u64],
    mem: &[u64],
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if matches!(style, VisualStyle::Rich) {
        let cpu_pts: Vec<(f64, f64)> = cpu
            .iter()
            .enumerate()
            .map(|(i, v)| (i as f64, *v as f64))
            .collect();
        let mem_pts: Vec<(f64, f64)> = mem
            .iter()
            .enumerate()
            .map(|(i, v)| (i as f64, *v as f64))
            .collect();
        let span = cpu.len().max(mem.len()).max(1) as f64 - 1.0;
        let datasets = vec![
            Dataset::default()
                .name("cpu")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(theme.accent))
                .data(&cpu_pts),
            Dataset::default()
                .name("ram")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(theme.low))
                .data(&mem_pts),
        ];
        let chart = Chart::new(datasets)
            .x_axis(Axis::default().bounds([0.0, span.max(1.0)]))
            .y_axis(
                Axis::default()
                    .bounds([0.0, 100.0])
                    .labels(["0", "50", "100"]),
            );
        frame.render_widget(chart, inner);
        return;
    }

    // Sober: two labelled sparklines stacked.
    let rows = Layout::vertical([Constraint::Ratio(1, 2); 2]).split(inner);
    for (row, (label, data, color)) in rows.iter().zip([
        ("cpu", cpu, theme.accent),
        ("ram", mem, theme.low),
    ]) {
        let cells =
            Layout::horizontal([Constraint::Length(4), Constraint::Min(0)]).split(*row);
        frame.render_widget(
            Paragraph::new(Span::styled(label, Style::new().fg(color))),
            cells[0],
        );
        frame.render_widget(
            Sparkline::default()
                .data(data)
                .max(100)
                .style(Style::new().fg(color)),
            cells[1],
        );
    }
}

/// A titled vertical [`BarChart`] from labelled, colored values. The shared
/// primitive behind the severity / top-process / log-volume charts.
pub fn bar_chart(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    bar_width: u16,
    items: &[(String, u64, Color)],
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {} ", title.to_uppercase()),
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let bars: Vec<Bar> = items
        .iter()
        .map(|(label, value, color)| {
            Bar::default()
                .value(*value)
                .label(Line::from(label.clone()))
                .style(Style::new().fg(*color))
                .value_style(Style::new().fg(theme.bg).bg(*color))
        })
        .collect();
    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(2)
        .label_style(Style::new().fg(theme.fg_dim));
    frame.render_widget(chart, inner);
}

/// A severity distribution (critical → info) as a colored bar chart.
pub fn severity_bars(frame: &mut Frame, theme: &Theme, area: Rect, counts: [u64; 5]) {
    let labels = ["CRIT", "HIGH", "MED", "LOW", "INFO"];
    let colors = [
        theme.critical,
        theme.high,
        theme.medium,
        theme.low,
        theme.fg_muted,
    ];
    let items: Vec<(String, u64, Color)> = counts
        .iter()
        .zip(labels.iter().zip(colors.iter()))
        .map(|(count, (label, color))| ((*label).to_owned(), *count, *color))
        .collect();
    bar_chart(frame, theme, area, "Findings by severity", 6, &items);
}

/// A squarified treemap of labelled, colored values laid into `area`. Each tile's
/// area is proportional to its value (Bruls et al. squarify). Tiles wide/tall
/// enough get a name and a value label drawn in a contrasting color. `items` is
/// `(name, value_label, value, color)`; render largest-first for the best layout.
pub fn treemap(frame: &mut Frame, theme: &Theme, area: Rect, items: &[(String, String, u64, Color)]) {
    if area.width == 0 || area.height == 0 || items.is_empty() {
        return;
    }
    let total: f64 = items.iter().map(|(_, _, v, _)| *v as f64).sum();
    if total <= 0.0 {
        return;
    }
    let (w, h) = (f64::from(area.width), f64::from(area.height));
    let scale = (w * h) / total;
    // (original index, scaled area), preserving caller order.
    let scaled: Vec<(usize, f64)> = items
        .iter()
        .enumerate()
        .map(|(i, (_, _, v, _))| (i, (*v as f64) * scale))
        .collect();

    let mut out: Vec<(usize, RectF)> = Vec::with_capacity(items.len());
    squarify(&scaled, RectF { x: 0.0, y: 0.0, w, h }, &mut out);

    for (idx, rf) in out {
        let x0 = rf.x.round() as u16;
        let y0 = rf.y.round() as u16;
        let x1 = (rf.x + rf.w).round() as u16;
        let y1 = (rf.y + rf.h).round() as u16;
        let tw = x1.saturating_sub(x0);
        let th = y1.saturating_sub(y0);
        if tw == 0 || th == 0 {
            continue;
        }
        let tile = Rect {
            x: area.x + x0,
            y: area.y + y0,
            width: tw.min(area.width.saturating_sub(x0)),
            height: th.min(area.height.saturating_sub(y0)),
        };
        let (name, vlabel, _, color) = &items[idx];
        // Fill the tile, leaving a 1-cell gutter via an inset block border color.
        frame.render_widget(
            Block::default().style(Style::new().bg(*color)),
            tile,
        );
        let mut lines = Vec::new();
        if tile.width >= 4 {
            let max = tile.width.saturating_sub(1) as usize;
            lines.push(Line::from(Span::styled(
                clip_label(name, max),
                Style::new().fg(theme.bg).add_modifier(Modifier::BOLD),
            )));
            if tile.height >= 2 {
                lines.push(Line::from(Span::styled(
                    clip_label(vlabel, max),
                    Style::new().fg(theme.bg),
                )));
            }
        }
        if !lines.is_empty() {
            let inner = Rect {
                x: tile.x + 1,
                width: tile.width.saturating_sub(1),
                ..tile
            };
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }
}

/// A float rectangle used by the treemap layout.
#[derive(Clone, Copy)]
struct RectF {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn clip_label(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_owned()
    } else {
        chars[..max].iter().collect()
    }
}

/// Worst aspect ratio of a row of tile areas laid along `side` (Bruls et al.).
fn tm_worst(row: &[f64], side: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 || side <= 0.0 {
        return f64::MAX;
    }
    let max = row.iter().cloned().fold(0.0_f64, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = sum * sum;
    (side * side * max / s2).max(s2 / (side * side * min))
}

/// Squarified treemap layout. Appends `(index, rect)` for every input tile.
fn squarify(items: &[(usize, f64)], mut rect: RectF, out: &mut Vec<(usize, RectF)>) {
    let mut i = 0;
    let mut row: Vec<(usize, f64)> = Vec::new();
    while i < items.len() {
        let side = rect.w.min(rect.h);
        let next = items[i];
        let cur: Vec<f64> = row.iter().map(|r| r.1).collect();
        let mut with_next = cur.clone();
        with_next.push(next.1);
        if row.is_empty() || tm_worst(&with_next, side) <= tm_worst(&cur, side) {
            row.push(next);
            i += 1;
        } else {
            tm_place_row(&row, &mut rect, out);
            row.clear();
        }
    }
    if !row.is_empty() {
        tm_place_row(&row, &mut rect, out);
    }
}

/// Place a finished row along the shorter side of `rect`, then shrink `rect`.
fn tm_place_row(row: &[(usize, f64)], rect: &mut RectF, out: &mut Vec<(usize, RectF)>) {
    let sum: f64 = row.iter().map(|r| r.1).sum();
    if sum <= 0.0 {
        return;
    }
    if rect.w >= rect.h {
        let row_w = sum / rect.h;
        let mut yy = rect.y;
        for &(idx, area) in row {
            let th = area / row_w;
            out.push((idx, RectF { x: rect.x, y: yy, w: row_w, h: th }));
            yy += th;
        }
        rect.x += row_w;
        rect.w -= row_w;
    } else {
        let row_h = sum / rect.w;
        let mut xx = rect.x;
        for &(idx, area) in row {
            let tw = area / row_h;
            out.push((idx, RectF { x: xx, y: rect.y, w: tw, h: row_h }));
            xx += tw;
        }
        rect.y += row_h;
        rect.h -= row_h;
    }
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
