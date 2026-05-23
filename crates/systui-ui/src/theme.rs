//! Color theme for the UI.

use ratatui::style::Color;

/// The set of colors used across the UI. Only a dark theme exists for now;
/// `config.ui.theme` selection arrives with the dashboard work (v0.1+).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub title: Color,
    pub accent: Color,
    pub text: Color,
    pub dim: Color,
    pub border: Color,
    pub selected_fg: Color,
    pub selected_bg: Color,
}

impl Theme {
    /// The default dark theme.
    pub fn dark() -> Self {
        Self {
            title: Color::Cyan,
            accent: Color::White,
            text: Color::Gray,
            dim: Color::DarkGray,
            border: Color::DarkGray,
            selected_fg: Color::Black,
            selected_bg: Color::Cyan,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
