//! Visual style selection for the UI.
//!
//! Independent of the color [`Theme`](crate::theme::Theme): the style controls
//! how *dense and ornate* the rendering is, not which colors are used. Two
//! styles ship — a clean, terminal-safe **sober** style and a high-resolution
//! **rich** style (braille/canvas meters) — selected at runtime, cycled with the
//! style key and persisted in `[general] visual_style` of the config.

/// Which visual style is active. Cycled at runtime and persisted in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualStyle {
    /// Clean, restrained rendering: chips, bars, sparklines, severity dots.
    /// Legible on any terminal and the default.
    #[default]
    Sober,
    /// Ornate rendering: braille/canvas high-resolution meters and charts.
    /// More striking, but heavier and font/terminal dependent.
    Rich,
}

impl VisualStyle {
    /// All styles, in cycle order.
    pub const ALL: [VisualStyle; 2] = [VisualStyle::Sober, VisualStyle::Rich];

    /// The next style in the cycle, wrapping around.
    pub const fn next(self) -> Self {
        match self {
            VisualStyle::Sober => VisualStyle::Rich,
            VisualStyle::Rich => VisualStyle::Sober,
        }
    }

    /// Stable name written to / read from the config file.
    pub const fn config_name(self) -> &'static str {
        match self {
            VisualStyle::Sober => "sober",
            VisualStyle::Rich => "rich",
        }
    }

    /// Short human label shown in the UI (status bar / help).
    pub const fn label(self) -> &'static str {
        match self {
            VisualStyle::Sober => "sober",
            VisualStyle::Rich => "rich",
        }
    }

    /// Resolve a config style name to a [`VisualStyle`]. Unknown names fall back
    /// to the sober style.
    pub fn from_config_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "rich" => VisualStyle::Rich,
            _ => VisualStyle::Sober,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_visits_every_style_and_wraps() {
        let mut s = VisualStyle::Sober;
        let mut seen = Vec::new();
        for _ in 0..VisualStyle::ALL.len() {
            seen.push(s);
            s = s.next();
        }
        assert_eq!(s, VisualStyle::Sober, "cycle wraps back to the start");
        for style in VisualStyle::ALL {
            assert!(seen.contains(&style), "cycle visits {style:?}");
        }
    }

    #[test]
    fn config_name_roundtrips() {
        for style in VisualStyle::ALL {
            assert_eq!(VisualStyle::from_config_name(style.config_name()), style);
        }
    }

    #[test]
    fn unknown_and_default_fall_back_to_sober() {
        assert_eq!(VisualStyle::from_config_name("nonsense"), VisualStyle::Sober);
        assert_eq!(VisualStyle::from_config_name("RICH"), VisualStyle::Rich);
        assert_eq!(VisualStyle::default(), VisualStyle::Sober);
    }
}
