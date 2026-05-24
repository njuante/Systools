//! A small reusable multi-field form for the TUI: labelled text and boolean
//! fields with focus navigation, inline editing and an error line. It owns no I/O
//! and no domain logic — callers build the fields, drive keys, read values back
//! and decide what to do on submit. Used for inventory host and cron editing.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::Theme;

/// The kind of value a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Free text, edited character by character.
    Text,
    /// A boolean, toggled with space (stored as `"true"`/`"false"`).
    Bool,
}

/// One labelled field.
#[derive(Debug, Clone)]
pub struct Field {
    pub label: String,
    pub kind: FieldKind,
    pub value: String,
    pub hint: Option<String>,
}

impl Field {
    pub fn text(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind: FieldKind::Text,
            value: value.into(),
            hint: None,
        }
    }

    pub fn boolean(label: impl Into<String>, on: bool) -> Self {
        Self {
            label: label.into(),
            kind: FieldKind::Bool,
            value: if on { "true" } else { "false" }.to_owned(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Whether a boolean field is on.
    pub fn is_on(&self) -> bool {
        self.value == "true"
    }
}

/// A modal form: a title, a list of fields, the focused field and an optional
/// validation error to display.
#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    pub focused: usize,
    pub error: Option<String>,
}

impl Form {
    pub fn new(title: impl Into<String>, fields: Vec<Field>) -> Self {
        Self {
            title: title.into(),
            fields,
            focused: 0,
            error: None,
        }
    }

    pub fn focus_next(&mut self) {
        if !self.fields.is_empty() {
            self.focused = (self.focused + 1) % self.fields.len();
        }
    }

    pub fn focus_prev(&mut self) {
        if !self.fields.is_empty() {
            self.focused = (self.focused + self.fields.len() - 1) % self.fields.len();
        }
    }

    /// Apply a key edit to the focused field: type into text fields, toggle bools.
    pub fn push_char(&mut self, c: char) {
        if let Some(field) = self.fields.get_mut(self.focused) {
            if field.kind == FieldKind::Text {
                field.value.push(c);
            }
        }
    }

    pub fn pop_char(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focused) {
            if field.kind == FieldKind::Text {
                field.value.pop();
            }
        }
    }

    /// Toggle the focused field if it is a boolean.
    pub fn toggle(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focused) {
            if field.kind == FieldKind::Bool {
                let on = field.is_on();
                field.value = if on { "false" } else { "true" }.to_owned();
            }
        }
    }

    /// The trimmed value of a field by label, if present.
    pub fn value(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.trim())
            .unwrap_or("")
    }

    /// Whether a boolean field (by label) is on.
    pub fn flag(&self, label: &str) -> bool {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(Field::is_on)
            .unwrap_or(false)
    }
}

/// Render a form as a centered modal. Pure function of the form + theme.
pub fn render_form(frame: &mut Frame, form: &Form, theme: &Theme) {
    let area = centered_rect(60, frame.area(), form.fields.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme.accent))
        .title(format!(" {} ", form.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut constraints = vec![Constraint::Length(1); form.fields.len()];
    constraints.push(Constraint::Length(1)); // spacer / error
    constraints.push(Constraint::Length(1)); // footer
    let rows = Layout::vertical(constraints).split(inner);

    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.focused;
        let caret = if focused { "\u{2192} " } else { "  " };
        let shown = match field.kind {
            FieldKind::Bool => {
                if field.is_on() {
                    "[x]".to_owned()
                } else {
                    "[ ]".to_owned()
                }
            }
            FieldKind::Text => {
                if focused {
                    format!("{}_", field.value)
                } else {
                    field.value.clone()
                }
            }
        };
        let hint = field
            .hint
            .as_deref()
            .map(|h| format!("  ({h})"))
            .unwrap_or_default();
        let label_style = if focused {
            Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text)
        };
        let line = Line::from(vec![
            Span::styled(format!("{caret}{:<12} ", field.label), label_style),
            Span::styled(shown, Style::new().fg(theme.accent)),
            Span::styled(hint, Style::new().fg(theme.dim)),
        ]);
        frame.render_widget(Paragraph::new(line), rows[i]);
    }

    if let Some(error) = &form.error {
        let line = Line::from(Span::styled(
            format!("  {error}"),
            Style::new().fg(theme.danger),
        ));
        frame.render_widget(Paragraph::new(line), rows[form.fields.len()]);
    }

    let footer = Line::from(Span::styled(
        " Tab/\u{2191}\u{2193} move \u{00b7} Space toggle \u{00b7} Enter save \u{00b7} Esc cancel ",
        Style::new().fg(theme.dim),
    ));
    frame.render_widget(Paragraph::new(footer), rows[form.fields.len() + 1]);
}

/// A horizontally-centered rect of `percent_x` width and a height that fits the
/// fields plus the error and footer lines and the border.
fn centered_rect(percent_x: u16, area: Rect, field_count: usize) -> Rect {
    let height = (field_count as u16 + 2).saturating_add(2).min(area.height);
    let width = area.width * percent_x / 100;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_wraps_in_both_directions() {
        let mut form = Form::new(
            "t",
            vec![
                Field::text("a", ""),
                Field::text("b", ""),
                Field::boolean("c", false),
            ],
        );
        form.focus_next();
        assert_eq!(form.focused, 1);
        form.focus_prev();
        form.focus_prev();
        assert_eq!(form.focused, 2);
        form.focus_next();
        assert_eq!(form.focused, 0);
    }

    #[test]
    fn text_edits_only_text_fields() {
        let mut form = Form::new(
            "t",
            vec![Field::text("name", "ab"), Field::boolean("on", false)],
        );
        form.push_char('c');
        assert_eq!(form.value("name"), "abc");
        form.pop_char();
        assert_eq!(form.value("name"), "ab");
        // Typing into a bool field does nothing.
        form.focused = 1;
        form.push_char('x');
        assert_eq!(form.value("on"), "false");
    }

    #[test]
    fn toggle_flips_bool_fields_only() {
        let mut form = Form::new(
            "t",
            vec![Field::text("name", "x"), Field::boolean("on", false)],
        );
        form.toggle(); // focused is the text field → no-op
        assert!(!form.flag("on"));
        form.focused = 1;
        form.toggle();
        assert!(form.flag("on"));
    }
}
