//! Color themes for the UI.
//!
//! [`Theme`] is the single source of truth for every color in the TUI: screen
//! code reads colors from here, and there are no inline `Color::Rgb(...)` calls
//! elsewhere. Three themes ship today — an enriched dark, a neon "midnight" and
//! a light theme — selected at runtime via [`ThemeKind`] and cycled with the
//! theme key. The selection is persisted in `[general] theme` of the config.
//!
//! On top of the base tokens, each theme exposes a set of **domain hues**
//! (teal/cyan/blue/indigo/violet/magenta/rose). Screens tint their panels by
//! domain so each tab reads as its own area at a glance; severity colors stay
//! independent of the domain hue.

use ratatui::style::Color;

/// Which palette is active. Cycled at runtime and persisted in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    /// Enriched dark theme: dark identity, high contrast, green accent.
    DarkRich,
    /// Deep navy base with saturated neon accents.
    Midnight,
    /// Light theme for bright environments.
    Light,
}

impl ThemeKind {
    /// All themes, in cycle order.
    pub const ALL: [ThemeKind; 3] = [ThemeKind::DarkRich, ThemeKind::Midnight, ThemeKind::Light];

    /// The next theme in the cycle, wrapping around.
    pub const fn next(self) -> Self {
        match self {
            ThemeKind::DarkRich => ThemeKind::Midnight,
            ThemeKind::Midnight => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::DarkRich,
        }
    }

    /// Stable name written to / read from the config file.
    pub const fn config_name(self) -> &'static str {
        match self {
            ThemeKind::DarkRich => "dark",
            ThemeKind::Midnight => "midnight",
            ThemeKind::Light => "light",
        }
    }

    /// Short human label shown in the UI (status bar / help).
    pub const fn label(self) -> &'static str {
        match self {
            ThemeKind::DarkRich => "dark",
            ThemeKind::Midnight => "midnight",
            ThemeKind::Light => "light",
        }
    }

    /// Resolve a config theme name to a [`ThemeKind`]. Unknown names (and the
    /// legacy `"dark_green"`) fall back to the enriched dark theme.
    pub fn from_config_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "midnight" | "neon" => ThemeKind::Midnight,
            "light" => ThemeKind::Light,
            _ => ThemeKind::DarkRich,
        }
    }

    /// Build the concrete [`Theme`] for this kind.
    pub const fn theme(self) -> Theme {
        match self {
            ThemeKind::DarkRich => Theme::dark_rich(),
            ThemeKind::Midnight => Theme::midnight(),
            ThemeKind::Light => Theme::light(),
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
    /// Enriched dark theme: dark identity, lifted contrast, green accent.
    pub const fn dark_rich() -> Self {
        Self::build(
            ThemeKind::DarkRich,
            Surface {
                bg: Color::Rgb(0x11, 0x15, 0x1c),
                bg_elev: Color::Rgb(0x18, 0x1d, 0x27),
                bg_elev_2: Color::Rgb(0x1f, 0x25, 0x31),
                bg_hover: Color::Rgb(0x23, 0x2b, 0x3a),
                bg_sel: Color::Rgb(0x27, 0x34, 0x4a),
                border: Color::Rgb(0x2c, 0x34, 0x42),
                border_soft: Color::Rgb(0x22, 0x2a, 0x36),
                border_strong: Color::Rgb(0x3a, 0x44, 0x56),
                fg: Color::Rgb(0xe3, 0xe8, 0xf0),
                fg_strong: Color::Rgb(0xf4, 0xf7, 0xfb),
                fg_muted: Color::Rgb(0x94, 0xa0, 0xb2),
                fg_dim: Color::Rgb(0x5a, 0x65, 0x77),
                accent: Color::Rgb(0x5f, 0xd3, 0x9a),
                accent_dim: Color::Rgb(0x3f, 0x8a, 0x5e),
            },
            SeverityColors {
                critical: Color::Rgb(0xf0, 0x72, 0x6f),
                high: Color::Rgb(0xf0, 0xb2, 0x4e),
                medium: Color::Rgb(0xf2, 0xd9, 0x7a),
                low: Color::Rgb(0x74, 0xa6, 0xff),
            },
            Domains {
                teal: Color::Rgb(0x4f, 0xd6, 0xc4),
                cyan: Color::Rgb(0x56, 0xc6, 0xe8),
                blue: Color::Rgb(0x6a, 0xa6, 0xff),
                indigo: Color::Rgb(0x8b, 0x9c, 0xff),
                violet: Color::Rgb(0xb7, 0x94, 0xff),
                magenta: Color::Rgb(0xe5, 0x8f, 0xdc),
                rose: Color::Rgb(0xf0, 0x8b, 0xb0),
            },
        )
    }

    /// Deep navy base with saturated neon accents.
    pub const fn midnight() -> Self {
        Self::build(
            ThemeKind::Midnight,
            Surface {
                bg: Color::Rgb(0x0a, 0x0e, 0x1a),
                bg_elev: Color::Rgb(0x11, 0x17, 0x26),
                bg_elev_2: Color::Rgb(0x18, 0x20, 0x33),
                bg_hover: Color::Rgb(0x1c, 0x26, 0x40),
                bg_sel: Color::Rgb(0x23, 0x34, 0x55),
                border: Color::Rgb(0x28, 0x35, 0x56),
                border_soft: Color::Rgb(0x1d, 0x27, 0x3f),
                border_strong: Color::Rgb(0x3a, 0x4d, 0x77),
                fg: Color::Rgb(0xdd, 0xe6, 0xf5),
                fg_strong: Color::Rgb(0xf2, 0xf6, 0xff),
                fg_muted: Color::Rgb(0x8b, 0x9b, 0xbd),
                fg_dim: Color::Rgb(0x56, 0x68, 0x8c),
                accent: Color::Rgb(0x4a, 0xde, 0x80),
                accent_dim: Color::Rgb(0x2f, 0x9e, 0x5c),
            },
            SeverityColors {
                critical: Color::Rgb(0xfb, 0x71, 0x85),
                high: Color::Rgb(0xfb, 0xbf, 0x24),
                medium: Color::Rgb(0xfc, 0xd3, 0x4d),
                low: Color::Rgb(0x60, 0xa5, 0xfa),
            },
            Domains {
                teal: Color::Rgb(0x2d, 0xd4, 0xbf),
                cyan: Color::Rgb(0x22, 0xd3, 0xee),
                blue: Color::Rgb(0x60, 0xa5, 0xfa),
                indigo: Color::Rgb(0x81, 0x8c, 0xf8),
                violet: Color::Rgb(0xc0, 0x84, 0xfc),
                magenta: Color::Rgb(0xe8, 0x79, 0xf9),
                rose: Color::Rgb(0xfb, 0x7e, 0xb0),
            },
        )
    }

    /// Light theme for bright environments. Accents darkened for contrast.
    pub const fn light() -> Self {
        Self::build(
            ThemeKind::Light,
            Surface {
                bg: Color::Rgb(0xf4, 0xf6, 0xfa),
                bg_elev: Color::Rgb(0xff, 0xff, 0xff),
                bg_elev_2: Color::Rgb(0xee, 0xf1, 0xf6),
                bg_hover: Color::Rgb(0xe6, 0xeb, 0xf3),
                bg_sel: Color::Rgb(0xd8, 0xe4, 0xf5),
                border: Color::Rgb(0xcf, 0xd6, 0xe2),
                border_soft: Color::Rgb(0xe0, 0xe5, 0xee),
                border_strong: Color::Rgb(0xb3, 0xbc, 0xcc),
                fg: Color::Rgb(0x1f, 0x27, 0x33),
                fg_strong: Color::Rgb(0x0c, 0x11, 0x18),
                fg_muted: Color::Rgb(0x5a, 0x66, 0x78),
                fg_dim: Color::Rgb(0x8a, 0x94, 0xa6),
                accent: Color::Rgb(0x1f, 0x9d, 0x57),
                accent_dim: Color::Rgb(0x15, 0x7a, 0x42),
            },
            SeverityColors {
                critical: Color::Rgb(0xd8, 0x3a, 0x3a),
                high: Color::Rgb(0xc7, 0x7d, 0x12),
                medium: Color::Rgb(0xb5, 0x8a, 0x00),
                low: Color::Rgb(0x25, 0x63, 0xeb),
            },
            Domains {
                teal: Color::Rgb(0x0f, 0x9e, 0x8e),
                cyan: Color::Rgb(0x0e, 0x8f, 0xb0),
                blue: Color::Rgb(0x25, 0x63, 0xeb),
                indigo: Color::Rgb(0x4f, 0x46, 0xe5),
                violet: Color::Rgb(0x7c, 0x3a, 0xed),
                magenta: Color::Rgb(0xb5, 0x27, 0x9b),
                rose: Color::Rgb(0xc4, 0x3a, 0x72),
            },
        )
    }

    /// Backwards-compatible alias for the default dark theme.
    pub const fn dark() -> Self {
        Self::dark_rich()
    }

    /// Backwards-compatible alias used by older call sites.
    pub const fn dark_green() -> Self {
        Self::dark_rich()
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
        Self::dark_rich()
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
        let mut k = ThemeKind::DarkRich;
        let mut seen = Vec::new();
        for _ in 0..ThemeKind::ALL.len() {
            seen.push(k);
            k = k.next();
        }
        assert_eq!(k, ThemeKind::DarkRich, "cycle wraps back to the start");
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
    fn unknown_and_legacy_names_fall_back_to_dark() {
        assert_eq!(ThemeKind::from_config_name("dark_green"), ThemeKind::DarkRich);
        assert_eq!(ThemeKind::from_config_name("nonsense"), ThemeKind::DarkRich);
        assert_eq!(ThemeKind::from_config_name("MIDNIGHT"), ThemeKind::Midnight);
    }

    #[test]
    fn each_theme_reports_its_kind() {
        for kind in ThemeKind::ALL {
            assert_eq!(kind.theme().kind, kind);
        }
    }

    #[test]
    fn domains_have_distinct_hues() {
        let t = Theme::dark_rich();
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
