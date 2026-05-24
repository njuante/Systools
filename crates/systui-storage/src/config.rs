//! Loading and parsing of the SysTUI configuration file.
//!
//! A missing config file is not an error: SysTUI falls back to the secure
//! defaults defined in [`systui_core::Config`].

use std::path::Path;

use systui_core::{Config, CoreError, Result};

use crate::paths;

/// Load the configuration from the default location.
///
/// Returns [`Config::default`] when the file does not exist.
pub fn load() -> Result<Config> {
    let path = paths::config_file()?;
    load_from(&path)
}

/// Load the configuration from a specific path.
///
/// Returns [`Config::default`] when the file does not exist; returns a
/// [`CoreError::Config`] when it exists but cannot be read or parsed.
pub fn load_from(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents)
            .map_err(|e| CoreError::Config(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(CoreError::Config(format!("{}: {e}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "systui-cfg-{}-{nanos}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let cfg = load_from(Path::new("/nonexistent/systui/config.toml")).unwrap();
        assert_eq!(cfg.general.theme, "dark");
        assert_eq!(cfg.thresholds.disk_critical, 90);
    }

    #[test]
    fn reads_and_parses_a_file() {
        let path = tmp_path("ok.toml");
        std::fs::write(
            &path,
            "[general]\ntheme = \"light\"\ndefault_refresh_seconds = 10\n",
        )
        .unwrap();

        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.general.theme, "light");
        assert_eq!(cfg.general.default_refresh_seconds, 10);
        // unspecified fields keep their defaults
        assert!(cfg.general.confirm_dangerous_actions);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn invalid_toml_is_a_config_error() {
        let path = tmp_path("bad.toml");
        std::fs::write(&path, "this is not = = valid toml").unwrap();

        let err = load_from(&path).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));

        std::fs::remove_file(&path).unwrap();
    }
}
