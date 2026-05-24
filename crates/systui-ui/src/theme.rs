//! Color theme for the UI.
//!
//! Single source of truth for every color in the TUI. The canonical tokens map
//! 1:1 to the approved design spec (`docs/interfaz/SysTUI Ratatui Spec.html` §13)
//! and are expressed as truecolor [`Color::Rgb`]. Screen code must read colors
//! from here — there are no inline `Color::Rgb(...)` calls elsewhere.
//!
//! The fields below the canonical tokens are **legacy aliases** kept so screens
//! not yet migrated to the design keep compiling. They resolve to the same RGB
//! values and are retired as each screen moves to the canonical token names.

use ratatui::style::Color;

/// The set of colors used across the UI. Only the dark (green-accent) theme
/// exists for now; theme selection is deferred (`docs/phases/phase-08-2`).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // --- Canonical design tokens (spec §13) ---
    /// App background.
    pub bg: Color,
    /// Elevated surface (panels, cards).
    pub bg_elev: Color,
    /// Second elevation (chips, inset blocks).
    pub bg_elev_2: Color,
    /// Hover / subtle highlight background.
    pub bg_hover: Color,
    /// Selection background.
    pub bg_sel: Color,
    /// Default border.
    pub border: Color,
    /// Soft (barely-there) border.
    pub border_soft: Color,
    /// Strong border (chips, emphasis).
    pub border_strong: Color,
    /// Primary text.
    pub fg: Color,
    /// Emphasized text.
    pub fg_strong: Color,
    /// Secondary / muted text.
    pub fg_muted: Color,
    /// Dim text (hints, placeholders).
    pub fg_dim: Color,
    /// Accent (brand green).
    pub accent: Color,
    /// Dimmed accent.
    pub accent_dim: Color,
    /// Critical severity.
    pub critical: Color,
    /// High severity.
    pub high: Color,
    /// Medium severity.
    pub medium: Color,
    /// Low / info severity.
    pub low: Color,

    // --- Legacy aliases (migration bridge) ---
    pub title: Color,
    pub text: Color,
    pub dim: Color,
    pub selected_fg: Color,
    pub selected_bg: Color,
    pub ok: Color,
    pub warn: Color,
    pub danger: Color,
}

impl Theme {
    /// The default dark theme (green accent), matching the approved design.
    pub const fn dark_green() -> Self {
        // Canonical tokens.
        let bg = Color::Rgb(0x0b, 0x0e, 0x13);
        let bg_elev = Color::Rgb(0x10, 0x14, 0x1c);
        let bg_elev_2 = Color::Rgb(0x16, 0x1b, 0x25);
        let bg_hover = Color::Rgb(0x1a, 0x20, 0x30);
        let bg_sel = Color::Rgb(0x1f, 0x2a, 0x3e);
        let border = Color::Rgb(0x1c, 0x22, 0x30);
        let border_soft = Color::Rgb(0x16, 0x1b, 0x24);
        let border_strong = Color::Rgb(0x2a, 0x32, 0x42);
        let fg = Color::Rgb(0xcd, 0xd4, 0xdf);
        let fg_strong = Color::Rgb(0xee, 0xf1, 0xf6);
        let fg_muted = Color::Rgb(0x7a, 0x84, 0x94);
        let fg_dim = Color::Rgb(0x4a, 0x53, 0x60);
        let accent = Color::Rgb(0x6d, 0xd3, 0x93);
        let accent_dim = Color::Rgb(0x3f, 0x8a, 0x5e);
        let critical = Color::Rgb(0xe5, 0x74, 0x73);
        let high = Color::Rgb(0xe8, 0xb5, 0x51);
        let medium = Color::Rgb(0xf0, 0xd9, 0x7d);
        let low = Color::Rgb(0x7a, 0xa9, 0xff);

        Self {
            bg,
            bg_elev,
            bg_elev_2,
            bg_hover,
            bg_sel,
            border,
            border_soft,
            border_strong,
            fg,
            fg_strong,
            fg_muted,
            fg_dim,
            accent,
            accent_dim,
            critical,
            high,
            medium,
            low,

            // Legacy aliases mapped onto the tokens above.
            title: accent,
            text: fg,
            dim: fg_muted,
            selected_fg: bg,
            selected_bg: accent,
            ok: accent,
            warn: high,
            danger: critical,
        }
    }

    /// Backwards-compatible alias for [`Theme::dark_green`].
    pub const fn dark() -> Self {
        Self::dark_green()
    }

    /// Color for a severity band, used by findings/exposure rendering.
    pub const fn severity(&self, severity: systui_core::Severity) -> Color {
        use systui_core::Severity;
        match severity {
            Severity::Critical => self.critical,
            Severity::High => self.high,
            Severity::Medium => self.medium,
            Severity::Low => self.low,
            Severity::Info => self.fg_muted,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark_green()
    }
}
