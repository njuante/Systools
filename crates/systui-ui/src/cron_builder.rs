//! A guided, visual builder for creating and editing cron jobs.
//!
//! Instead of asking the operator to write a raw five-field cron expression, the
//! builder offers a frequency (daily / hourly / every N minutes / weekly /
//! monthly / at reboot / custom) and only the few inputs that frequency needs.
//! It renders a live preview — the generated expression, a plain-language
//! description and the next upcoming runs — that updates as fields change, so
//! the result is visible before saving. Raw cron is still available via the
//! `Custom` frequency for power users.

use chrono::NaiveDateTime;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use systui_collectors::parse_schedule;

use crate::Theme;
use crate::app::CronFormMode;

/// How often the job runs. Drives which inputs the builder shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Minutes,
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Reboot,
    Custom,
}

impl Frequency {
    const ALL: [Frequency; 7] = [
        Frequency::Minutes,
        Frequency::Hourly,
        Frequency::Daily,
        Frequency::Weekly,
        Frequency::Monthly,
        Frequency::Reboot,
        Frequency::Custom,
    ];

    fn label(self) -> &'static str {
        match self {
            Frequency::Minutes => "Every N minutes",
            Frequency::Hourly => "Hourly",
            Frequency::Daily => "Daily",
            Frequency::Weekly => "Weekly",
            Frequency::Monthly => "Monthly",
            Frequency::Reboot => "At startup",
            Frequency::Custom => "Custom (raw cron)",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// One editable row in the builder. The visible set depends on the frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderField {
    Frequency,
    Interval,
    Minute,
    Weekday,
    Day,
    Time,
    Custom,
    Command,
}

impl BuilderField {
    /// Whether this field is changed with ←/→ (a choice) vs. typed into.
    fn is_choice(self) -> bool {
        matches!(
            self,
            BuilderField::Frequency
                | BuilderField::Interval
                | BuilderField::Minute
                | BuilderField::Weekday
                | BuilderField::Day
        )
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The state of an open cron builder.
#[derive(Debug, Clone)]
pub struct CronBuilder {
    pub mode: CronFormMode,
    freq: Frequency,
    interval: u32, // every N minutes (1..=59)
    minute: u32,   // minute of the hour for Hourly (0..=59)
    weekday: u32,  // 0=Sun..6=Sat
    day: u32,      // day of month (1..=31)
    time: String,  // "HH:MM" for daily/weekly/monthly
    custom: String,
    command: String,
    focused: usize,
    pub error: Option<String>,
}

impl CronBuilder {
    /// A builder for a brand-new daily job.
    pub fn add() -> Self {
        Self {
            mode: CronFormMode::Add,
            freq: Frequency::Daily,
            interval: 15,
            minute: 0,
            weekday: 1,
            day: 1,
            time: "09:00".to_owned(),
            custom: String::new(),
            command: String::new(),
            focused: 0,
            error: None,
        }
    }

    /// A builder seeded from an existing job. The existing schedule is kept as a
    /// raw expression (Custom) rather than reverse-engineered into fields, so
    /// the edit is always faithful.
    pub fn edit(original_schedule: String, original_command: String) -> Self {
        let mut b = Self::add();
        b.mode = CronFormMode::Edit {
            original_schedule: original_schedule.clone(),
            original_command: original_command.clone(),
        };
        b.freq = Frequency::Custom;
        b.custom = original_schedule;
        b.command = original_command;
        b
    }

    pub fn title(&self) -> &'static str {
        match self.mode {
            CronFormMode::Add => "New cron job",
            CronFormMode::Edit { .. } => "Edit cron job",
        }
    }

    /// The ordered, visible fields for the current frequency.
    fn fields(&self) -> Vec<BuilderField> {
        let mut f = vec![BuilderField::Frequency];
        match self.freq {
            Frequency::Minutes => f.push(BuilderField::Interval),
            Frequency::Hourly => f.push(BuilderField::Minute),
            Frequency::Daily => f.push(BuilderField::Time),
            Frequency::Weekly => {
                f.push(BuilderField::Weekday);
                f.push(BuilderField::Time);
            }
            Frequency::Monthly => {
                f.push(BuilderField::Day);
                f.push(BuilderField::Time);
            }
            Frequency::Reboot => {}
            Frequency::Custom => f.push(BuilderField::Custom),
        }
        f.push(BuilderField::Command);
        f
    }

    fn focused_field(&self) -> BuilderField {
        let fields = self.fields();
        fields[self.focused.min(fields.len() - 1)]
    }

    pub fn focus_next(&mut self) {
        let len = self.fields().len();
        self.focused = (self.focused + 1) % len;
    }

    pub fn focus_prev(&mut self) {
        let len = self.fields().len();
        self.focused = (self.focused + len - 1) % len;
    }

    /// Decrease the focused choice field (←).
    pub fn decrement(&mut self) {
        self.error = None;
        match self.focused_field() {
            BuilderField::Frequency => {
                self.freq = self.freq.prev();
                self.clamp_focus();
            }
            BuilderField::Interval => self.interval = dec_wrap(self.interval, 1, 59),
            BuilderField::Minute => self.minute = dec_wrap(self.minute, 0, 59),
            BuilderField::Weekday => self.weekday = (self.weekday + 6) % 7,
            BuilderField::Day => self.day = dec_wrap(self.day, 1, 31),
            _ => {}
        }
    }

    /// Increase the focused choice field (→).
    pub fn increment(&mut self) {
        self.error = None;
        match self.focused_field() {
            BuilderField::Frequency => {
                self.freq = self.freq.next();
                self.clamp_focus();
            }
            BuilderField::Interval => self.interval = inc_wrap(self.interval, 1, 59),
            BuilderField::Minute => self.minute = inc_wrap(self.minute, 0, 59),
            BuilderField::Weekday => self.weekday = (self.weekday + 1) % 7,
            BuilderField::Day => self.day = inc_wrap(self.day, 1, 31),
            _ => {}
        }
    }

    /// Type a character into the focused text field.
    pub fn push_char(&mut self, c: char) {
        self.error = None;
        match self.focused_field() {
            BuilderField::Time
                if (c.is_ascii_digit() || c == ':') && self.time.chars().count() < 5 =>
            {
                self.time.push(c);
            }
            BuilderField::Custom => self.custom.push(c),
            BuilderField::Command => self.command.push(c),
            _ => {}
        }
    }

    pub fn pop_char(&mut self) {
        self.error = None;
        match self.focused_field() {
            BuilderField::Time => {
                self.time.pop();
            }
            BuilderField::Custom => {
                self.custom.pop();
            }
            BuilderField::Command => {
                self.command.pop();
            }
            _ => {}
        }
    }

    fn clamp_focus(&mut self) {
        let len = self.fields().len();
        if self.focused >= len {
            self.focused = len - 1;
        }
    }

    pub fn command(&self) -> &str {
        self.command.trim()
    }

    /// The generated cron expression, or `None` when current inputs can't form a
    /// valid one (e.g. a half-typed time, or an empty custom expression).
    pub fn expression(&self) -> Option<String> {
        match self.freq {
            Frequency::Minutes => (1..=59)
                .contains(&self.interval)
                .then(|| format!("*/{} * * * *", self.interval)),
            Frequency::Hourly => (self.minute <= 59).then(|| format!("{} * * * *", self.minute)),
            Frequency::Daily => {
                let (h, m) = parse_hm(&self.time)?;
                Some(format!("{m} {h} * * *"))
            }
            Frequency::Weekly => {
                let (h, m) = parse_hm(&self.time)?;
                Some(format!("{m} {h} * * {}", self.weekday))
            }
            Frequency::Monthly => {
                let (h, m) = parse_hm(&self.time)?;
                (1..=31)
                    .contains(&self.day)
                    .then(|| format!("{m} {h} {} * *", self.day))
            }
            Frequency::Reboot => Some("@reboot".to_owned()),
            Frequency::Custom => {
                let c = self.custom.trim();
                (!c.is_empty()).then(|| c.to_owned())
            }
        }
    }
}

/// Parse `"HH:MM"` into `(hour, minute)`, or `None` if malformed/out of range.
fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (h <= 23 && m <= 59).then_some((h, m))
}

fn inc_wrap(v: u32, min: u32, max: u32) -> u32 {
    if v >= max { min } else { v + 1 }
}

fn dec_wrap(v: u32, min: u32, max: u32) -> u32 {
    if v <= min { max } else { v - 1 }
}

/// Render the builder as a centered modal with a live preview.
pub fn render_cron_builder(frame: &mut Frame, b: &CronBuilder, theme: &Theme, now: NaiveDateTime) {
    let fields = b.fields();
    // Height: title border + one row per field + blank + 5 preview lines + footer.
    let body_rows = fields.len() as u16 + 1 + 5 + 1;
    let height = (body_rows + 2).min(frame.area().height);
    let width = (frame.area().width * 64 / 100).clamp(40, 76);
    let area = centered(frame.area(), width, height);
    frame.render_widget(Clear, area);

    // The modal is a themed card: fill the whole area with the elevated surface
    // (and theme fg) so it reads correctly under any theme, including light.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::new().fg(theme.violet))
        .style(Style::new().bg(theme.bg_elev).fg(theme.fg))
        .title(Span::styled(
            format!(" {} ", b.title()),
            Style::new().fg(theme.violet).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut constraints: Vec<Constraint> = fields.iter().map(|_| Constraint::Length(1)).collect();
    constraints.push(Constraint::Length(1)); // divider
    constraints.push(Constraint::Min(5)); // preview
    constraints.push(Constraint::Length(1)); // footer
    let rows = Layout::vertical(constraints).split(inner);

    for (i, field) in fields.iter().enumerate() {
        let focused = i == b.focused;
        frame.render_widget(field_line(b, *field, focused, theme), rows[i]);
    }

    // A labelled divider separating the inputs from the live preview.
    let divider_row = rows[fields.len()];
    let dashes = "─".repeat(divider_row.width.saturating_sub(10) as usize);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" preview ", Style::new().fg(theme.violet)),
            Span::styled(dashes, Style::new().fg(theme.border)),
        ])),
        divider_row,
    );

    // The preview sits on a second elevation so it reads as its own block.
    let preview_row = rows[fields.len() + 1];
    frame.render_widget(
        Block::default().style(Style::new().bg(theme.bg_elev_2)),
        preview_row,
    );
    render_preview(frame, b, theme, now, preview_row);

    let footer = Line::from(Span::styled(
        " ↑↓ move · ←→ change · type to edit · Enter save · Esc cancel ",
        Style::new().fg(theme.fg_dim),
    ));
    frame.render_widget(Paragraph::new(footer), rows[fields.len() + 2]);
}

/// One field row: a label, then a value styled by kind (choice vs. typed).
fn field_line(
    b: &CronBuilder,
    field: BuilderField,
    focused: bool,
    theme: &Theme,
) -> Paragraph<'static> {
    let (label, value) = match field {
        BuilderField::Frequency => ("Frequency", b.freq.label().to_owned()),
        BuilderField::Interval => ("Every", format!("{} min", b.interval)),
        BuilderField::Minute => ("At minute", format!(":{:02}", b.minute)),
        BuilderField::Weekday => ("Weekday", WEEKDAYS[b.weekday as usize % 7].to_owned()),
        BuilderField::Day => ("Day of month", b.day.to_string()),
        BuilderField::Time => ("Time", b.time.clone()),
        BuilderField::Custom => ("Cron expr", b.custom.clone()),
        BuilderField::Command => ("Command", b.command.clone()),
    };

    let caret = if focused { "→ " } else { "  " };
    let label_style = if focused {
        Style::new().fg(theme.violet).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.fg)
    };

    let value_span = if field.is_choice() {
        // Choice fields read as ‹ value › to signal ←/→ adjustment.
        Span::styled(
            format!("‹ {value} ›"),
            Style::new()
                .fg(theme.fg_strong)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        // Typed fields show a caret when focused.
        let shown = if focused { format!("{value}_") } else { value };
        Span::styled(shown, Style::new().fg(theme.fg_strong))
    };

    Paragraph::new(Line::from(vec![
        Span::styled(format!("{caret}{label:<13} "), label_style),
        value_span,
    ]))
}

/// The live preview: generated expression, human description and next runs.
fn render_preview(
    frame: &mut Frame,
    b: &CronBuilder,
    theme: &Theme,
    now: NaiveDateTime,
    area: Rect,
) {
    let label = |k: &str| Span::styled(format!("{k:<9} "), Style::new().fg(theme.fg_dim));
    let mut lines = Vec::new();

    match b.expression() {
        Some(expr) => match parse_schedule(&expr) {
            Ok(schedule) => {
                lines.push(Line::from(vec![
                    label("cron"),
                    Span::styled(
                        expr,
                        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    label("when"),
                    Span::styled(schedule.describe(), Style::new().fg(theme.fg)),
                ]));
                let runs = schedule.upcoming(now, 3);
                if runs.is_empty() {
                    lines.push(Line::from(vec![
                        label("next"),
                        Span::styled("on demand / at startup", Style::new().fg(theme.fg_muted)),
                    ]));
                } else {
                    for (i, r) in runs.iter().enumerate() {
                        let key = if i == 0 { label("next") } else { label("") };
                        lines.push(Line::from(vec![
                            key,
                            Span::styled(
                                r.format("%a %Y-%m-%d %H:%M").to_string(),
                                Style::new().fg(theme.fg_muted),
                            ),
                        ]));
                    }
                }
            }
            Err(e) => lines.push(Line::from(vec![
                label("cron"),
                Span::styled(format!("invalid: {e}"), Style::new().fg(theme.critical)),
            ])),
        },
        None => lines.push(Line::from(Span::styled(
            "fill in the fields above to preview the schedule",
            Style::new().fg(theme.fg_dim),
        ))),
    }

    if let Some(err) = &b.error {
        lines.push(Line::from(Span::styled(
            format!("✗ {err}"),
            Style::new().fg(theme.critical),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_builds_expected_expression() {
        let mut b = CronBuilder::add();
        b.time = "14:30".to_owned();
        assert_eq!(b.expression().as_deref(), Some("30 14 * * *"));
    }

    #[test]
    fn frequency_specific_expressions() {
        let mut b = CronBuilder::add();
        b.freq = Frequency::Minutes;
        b.interval = 15;
        assert_eq!(b.expression().as_deref(), Some("*/15 * * * *"));

        b.freq = Frequency::Hourly;
        b.minute = 5;
        assert_eq!(b.expression().as_deref(), Some("5 * * * *"));

        b.freq = Frequency::Weekly;
        b.weekday = 1; // Mon
        b.time = "09:00".to_owned();
        assert_eq!(b.expression().as_deref(), Some("0 9 * * 1"));

        b.freq = Frequency::Monthly;
        b.day = 1;
        b.time = "00:00".to_owned();
        assert_eq!(b.expression().as_deref(), Some("0 0 1 * *"));

        b.freq = Frequency::Reboot;
        assert_eq!(b.expression().as_deref(), Some("@reboot"));
    }

    #[test]
    fn half_typed_time_has_no_expression() {
        let mut b = CronBuilder::add();
        b.time = "14:".to_owned();
        assert_eq!(b.expression(), None);
    }

    #[test]
    fn custom_passes_raw_through() {
        let mut b = CronBuilder::add();
        b.freq = Frequency::Custom;
        b.custom = "*/5 9-17 * * 1-5".to_owned();
        assert_eq!(b.expression().as_deref(), Some("*/5 9-17 * * 1-5"));
    }

    #[test]
    fn editing_seeds_custom_with_existing_schedule() {
        let b = CronBuilder::edit("0 3 * * *".to_owned(), "/opt/x.sh".to_owned());
        assert_eq!(b.freq, Frequency::Custom);
        assert_eq!(b.expression().as_deref(), Some("0 3 * * *"));
        assert_eq!(b.command(), "/opt/x.sh");
    }

    #[test]
    fn visible_fields_track_frequency() {
        let mut b = CronBuilder::add();
        b.freq = Frequency::Reboot;
        // Frequency + Command only when nothing else is needed.
        assert_eq!(
            b.fields(),
            vec![BuilderField::Frequency, BuilderField::Command]
        );
        b.freq = Frequency::Weekly;
        assert_eq!(
            b.fields(),
            vec![
                BuilderField::Frequency,
                BuilderField::Weekday,
                BuilderField::Time,
                BuilderField::Command,
            ]
        );
    }

    #[test]
    fn changing_frequency_keeps_focus_in_range() {
        let mut b = CronBuilder::add();
        b.freq = Frequency::Weekly; // 4 fields
        b.focused = 3; // Command
        b.freq = Frequency::Reboot; // collapses to 2 fields
        b.clamp_focus();
        assert!(b.focused < b.fields().len());
    }
}
