//! Color themes for the UI.
//!
//! [`Theme`] is the single source of truth for every color in the TUI: screen
//! code reads colors from here, and there are no inline `Color::Rgb(...)` calls
//! elsewhere. Two CRT-terminal palettes ship — **phosphor** (cool greens/cyans
//! on near-black) and **ember** (warm ambers/reds on warm black) — selected at
//! runtime via [`ThemeKind`] and cycled with the theme key. The selection is
//! persisted in `[general] theme` of the config.
//!
//! On top of the base tokens, each theme exposes a set of **domain hues**
//! (teal/cyan/blue/indigo/violet/magenta/rose). Screens tint their panels by
//! domain so each tab reads as its own area at a glance; severity colors stay
//! independent of the domain hue.

use ratatui::style::Color;

/// Which palette is active. Cycled at runtime and persisted in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    /// Phosphor: cool greens/cyans on near-black. The default identity.
    Phosphor,
    /// Ember: warm ambers/reds on warm black.
    Ember,
}

impl ThemeKind {
    /// All themes, in cycle order.
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Phosphor, ThemeKind::Ember];

    /// The next theme in the cycle, wrapping around.
    pub const fn next(self) -> Self {
        match self {
            ThemeKind::Phosphor => ThemeKind::Ember,
            ThemeKind::Ember => ThemeKind::Phosphor,
        }
    }

    /// Stable name written to / read from the config file.
    pub const fn config_name(self) -> &'static str {
        match self {
            ThemeKind::Phosphor => "phosphor",
            ThemeKind::Ember => "ember",
        }
    }

    /// Short human label shown in the UI (status bar / help).
    pub const fn label(self) -> &'static str {
        match self {
            ThemeKind::Phosphor => "phosphor",
            ThemeKind::Ember => "ember",
        }
    }

    /// Resolve a config theme name to a [`ThemeKind`]. Unknown names (and the
    /// retired `dark`/`midnight`/`light` themes) fall back to phosphor.
    pub fn from_config_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "ember" => ThemeKind::Ember,
            _ => ThemeKind::Phosphor,
        }
    }

    /// Build the concrete [`Theme`] for this kind.
    pub const fn theme(self) -> Theme {
        match self {
            ThemeKind::Phosphor => Theme::phosphor(),
            ThemeKind::Ember => Theme::ember(),
        }
    }
}

/// A semantic area of the app. Each maps to a domain hue so panels in that area
/// share a recognizable color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Dashboard,
    System,
    Processes,
    Services,
    Logs,
    Network,
    Docker,
    Crons,
    Databases,
    Security,
}

/// The set of colors used across the UI.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Which palette this is (so the UI can label/persist it).
    pub kind: ThemeKind,

    // --- Base surface / text tokens ---
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

    // --- Severity ---
    /// Critical severity.
    pub critical: Color,
    /// High severity.
    pub high: Color,
    /// Medium severity.
    pub medium: Color,
    /// Low / info severity.
    pub low: Color,

    // --- Domain hues ---
    pub teal: Color,
    pub cyan: Color,
    pub blue: Color,
    pub indigo: Color,
    pub violet: Color,
    pub magenta: Color,
    pub rose: Color,

    // --- Legacy aliases (resolve to tokens above) ---
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
    /// Phosphor: cool greens/cyans on near-black. The default CRT identity.
    pub const fn phosphor() -> Self {
        Self::build(
            ThemeKind::Phosphor,
            Surface {
                bg: Color::Rgb(0x06, 0x0a, 0x0a),
                bg_elev: Color::Rgb(0x0a, 0x12, 0x13),
                bg_elev_2: Color::Rgb(0x0f, 0x1b, 0x1c),
                bg_hover: Color::Rgb(0x14, 0x26, 0x28),
                bg_sel: Color::Rgb(0x14, 0x3d, 0x3a),
                border: Color::Rgb(0x28, 0x4b, 0x50),
                border_soft: Color::Rgb(0x1d, 0x33, 0x36),
                border_strong: Color::Rgb(0x28, 0x4b, 0x50),
                fg: Color::Rgb(0xc8, 0xe6, 0xdc),
                fg_strong: Color::Rgb(0xe4, 0xf6, 0xee),
                fg_muted: Color::Rgb(0x8a, 0xa9, 0xa1),
                fg_dim: Color::Rgb(0x4a, 0x6b, 0x66),
                accent: Color::Rgb(0x5c, 0xe3, 0x9a),
                accent_dim: Color::Rgb(0x2f, 0x7a, 0x55),
            },
            SeverityColors {
                critical: Color::Rgb(0xff, 0x6b, 0x6b),
                high: Color::Rgb(0xff, 0xd1, 0x66),
                medium: Color::Rgb(0xf0, 0xb8, 0x5a),
                low: Color::Rgb(0x6f, 0xd3, 0xe8),
            },
            Domains {
                teal: Color::Rgb(0x4f, 0xd6, 0xc4),
                cyan: Color::Rgb(0x6f, 0xd3, 0xe8),
                blue: Color::Rgb(0x74, 0xa6, 0xff),
                indigo: Color::Rgb(0x9a, 0xa8, 0xff),
                violet: Color::Rgb(0xb7, 0x94, 0xff),
                magenta: Color::Rgb(0xd1, 0x8c, 0xff),
                rose: Color::Rgb(0xff, 0x8b, 0xb0),
            },
        )
    }

    /// Ember: warm ambers/reds on warm black.
    pub const fn ember() -> Self {
        Self::build(
            ThemeKind::Ember,
            Surface {
                bg: Color::Rgb(0x0a, 0x07, 0x06),
                bg_elev: Color::Rgb(0x12, 0x0c, 0x09),
                bg_elev_2: Color::Rgb(0x1a, 0x12, 0x0d),
                bg_hover: Color::Rgb(0x24, 0x18, 0x11),
                bg_sel: Color::Rgb(0x3a, 0x24, 0x12),
                border: Color::Rgb(0x4a, 0x2f, 0x1f),
                border_soft: Color::Rgb(0x2a, 0x1c, 0x14),
                border_strong: Color::Rgb(0x4a, 0x2f, 0x1f),
                fg: Color::Rgb(0xf0, 0xd8, 0xb6),
                fg_strong: Color::Rgb(0xfc, 0xe8, 0xc8),
                fg_muted: Color::Rgb(0xb0, 0x92, 0x77),
                fg_dim: Color::Rgb(0x74, 0x5a, 0x44),
                accent: Color::Rgb(0xff, 0xae, 0x42),
                accent_dim: Color::Rgb(0xb8, 0x7a, 0x2a),
            },
            SeverityColors {
                critical: Color::Rgb(0xff, 0x5e, 0x3a),
                high: Color::Rgb(0xff, 0xb8, 0x4a),
                medium: Color::Rgb(0xf0, 0xa8, 0x68),
                low: Color::Rgb(0xb8, 0xd3, 0x6a),
            },
            Domains {
                teal: Color::Rgb(0x6c, 0xc0, 0xb0),
                cyan: Color::Rgb(0x6f, 0xb8, 0xd0),
                blue: Color::Rgb(0x8f, 0xb0, 0xe0),
                indigo: Color::Rgb(0xb0, 0xa0, 0xe0),
                violet: Color::Rgb(0xd6, 0xa0, 0xe0),
                magenta: Color::Rgb(0xff, 0x7a, 0xb8),
                rose: Color::Rgb(0xff, 0x9a, 0x8a),
            },
        )
    }

    /// Backwards-compatible alias for the default theme (now phosphor).
    pub const fn dark() -> Self {
        Self::phosphor()
    }

    /// Backwards-compatible alias used by older call sites.
    pub const fn dark_green() -> Self {
        Self::phosphor()
    }

    /// Assemble a theme from its surface, severity and domain groups, deriving
    /// the legacy aliases. Keeps each palette above declarative.
    const fn build(kind: ThemeKind, s: Surface, sev: SeverityColors, d: Domains) -> Self {
        Self {
            kind,
            bg: s.bg,
            bg_elev: s.bg_elev,
            bg_elev_2: s.bg_elev_2,
            bg_hover: s.bg_hover,
            bg_sel: s.bg_sel,
            border: s.border,
            border_soft: s.border_soft,
            border_strong: s.border_strong,
            fg: s.fg,
            fg_strong: s.fg_strong,
            fg_muted: s.fg_muted,
            fg_dim: s.fg_dim,
            accent: s.accent,
            accent_dim: s.accent_dim,
            critical: sev.critical,
            high: sev.high,
            medium: sev.medium,
            low: sev.low,
            teal: d.teal,
            cyan: d.cyan,
            blue: d.blue,
            indigo: d.indigo,
            violet: d.violet,
            magenta: d.magenta,
            rose: d.rose,
            // Legacy aliases mapped onto the tokens above.
            title: s.accent,
            text: s.fg,
            dim: s.fg_muted,
            selected_fg: s.bg,
            selected_bg: s.accent,
            ok: s.accent,
            warn: sev.high,
            danger: sev.critical,
        }
    }

    /// The hue for a domain. Used to tint panels and the active tab.
    pub const fn domain(&self, domain: Domain) -> Color {
        match domain {
            Domain::Dashboard => self.accent,
            Domain::System => self.teal,
            Domain::Processes => self.magenta,
            Domain::Services => self.blue,
            Domain::Logs => self.high,
            Domain::Network => self.cyan,
            Domain::Docker => self.indigo,
            Domain::Crons => self.violet,
            Domain::Databases => self.rose,
            Domain::Security => self.critical,
        }
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
        Self::phosphor()
    }
}

/// Internal grouping passed to [`Theme::build`] — surface and text tokens.
struct Surface {
    bg: Color,
    bg_elev: Color,
    bg_elev_2: Color,
    bg_hover: Color,
    bg_sel: Color,
    border: Color,
    border_soft: Color,
    border_strong: Color,
    fg: Color,
    fg_strong: Color,
    fg_muted: Color,
    fg_dim: Color,
    accent: Color,
    accent_dim: Color,
}

/// Internal grouping passed to [`Theme::build`] — severity colors.
struct SeverityColors {
    critical: Color,
    high: Color,
    medium: Color,
    low: Color,
}

/// Internal grouping passed to [`Theme::build`] — domain hues.
struct Domains {
    teal: Color,
    cyan: Color,
    blue: Color,
    indigo: Color,
    violet: Color,
    magenta: Color,
    rose: Color,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_visits_every_theme_and_wraps() {
        let mut k = ThemeKind::Phosphor;
        let mut seen = Vec::new();
        for _ in 0..ThemeKind::ALL.len() {
            seen.push(k);
            k = k.next();
        }
        assert_eq!(k, ThemeKind::Phosphor, "cycle wraps back to the start");
        for kind in ThemeKind::ALL {
            assert!(seen.contains(&kind), "cycle visits {kind:?}");
        }
    }

    #[test]
    fn config_name_round_trips() {
        for kind in ThemeKind::ALL {
            assert_eq!(ThemeKind::from_config_name(kind.config_name()), kind);
        }
    }

    #[test]
    fn unknown_and_legacy_names_fall_back_to_phosphor() {
        assert_eq!(
            ThemeKind::from_config_name("dark_green"),
            ThemeKind::Phosphor
        );
        assert_eq!(ThemeKind::from_config_name("nonsense"), ThemeKind::Phosphor);
        assert_eq!(ThemeKind::from_config_name("midnight"), ThemeKind::Phosphor);
        assert_eq!(ThemeKind::from_config_name("EMBER"), ThemeKind::Ember);
    }

    #[test]
    fn each_theme_reports_its_kind() {
        for kind in ThemeKind::ALL {
            assert_eq!(kind.theme().kind, kind);
        }
    }

    #[test]
    fn domains_have_distinct_hues() {
        let t = Theme::phosphor();
        let domains = [
            Domain::Dashboard,
            Domain::System,
            Domain::Processes,
            Domain::Services,
            Domain::Logs,
            Domain::Network,
            Domain::Docker,
            Domain::Crons,
            Domain::Databases,
            Domain::Security,
        ];
        let colors: Vec<_> = domains.iter().map(|d| t.domain(*d)).collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "{:?} and {:?} share a hue",
                    domains[i], domains[j]
                );
            }
        }
    }
}
