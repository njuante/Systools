//! Resolves the platform config, data and cache directories for SysTUI.
//!
//! On Linux these are `~/.config/systui`, `~/.local/share/systui` and
//! `~/.cache/systui` (or their `XDG_*` overrides).

use std::path::PathBuf;

use directories::ProjectDirs;
use systui_core::{CoreError, Result};

/// The resolved SysTUI project directories.
fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "systui")
        .ok_or_else(|| CoreError::Config("could not determine the user home directory".to_owned()))
}

/// Directory holding `config.toml`.
pub fn config_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().to_path_buf())
}

/// Directory for persistent data (profiles, audit log).
pub fn data_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.data_dir().to_path_buf())
}

/// Directory for disposable cache.
pub fn cache_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.cache_dir().to_path_buf())
}

/// Full path to the main configuration file.
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}
