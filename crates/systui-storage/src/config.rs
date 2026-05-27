//! Loading and parsing of the SysTUI configuration file.
//!
//! A missing config file is not an error: SysTUI falls back to the secure
//! defaults defined in [`systui_core::Config`].

use std::path::Path;

use systui_core::config::Host;
use systui_core::{Config, CoreError, Result};
use toml_edit::{Array, DocumentMut, Item, Table, value};

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

/// Add or update the `[hosts.<id>]` entry in the default config file.
pub fn save_host(id: &str, host: &Host) -> Result<()> {
    save_host_to(&paths::config_file()?, id, host)
}

/// Add or update the `[hosts.<id>]` entry in a specific config file, **preserving
/// the rest of the file** (other tables, comments, ordering). Creates the file and
/// its parent directory when missing. Only the keys SysTUI manages are written;
/// optional/false fields are removed so the entry stays minimal.
pub fn save_host_to(path: &Path, id: &str, host: &Host) -> Result<()> {
    let mut doc = read_document(path)?;
    let hosts = doc
        .entry("hosts")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| CoreError::Config(format!("{}: `hosts` is not a table", path.display())))?;
    // Render as `[hosts.<id>]` rather than an explicit `[hosts]` header.
    hosts.set_implicit(true);
    let table = hosts
        .entry(id)
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            CoreError::Config(format!("{}: `hosts.{id}` is not a table", path.display()))
        })?;
    write_host_fields(table, host);
    write_document(path, &doc)
}

/// Persist the active theme into `[general] theme` of the default config file,
/// preserving the rest of the file (hosts, comments, other keys).
pub fn save_general_theme(theme: &str) -> Result<()> {
    save_general_theme_to(&paths::config_file()?, theme)
}

/// Persist the active theme into `[general] theme` of a specific config file.
/// Creates the file and its parent directory when missing.
pub fn save_general_theme_to(path: &Path, theme: &str) -> Result<()> {
    let mut doc = read_document(path)?;
    let general = doc
        .entry("general")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            CoreError::Config(format!("{}: `general` is not a table", path.display()))
        })?;
    general["theme"] = value(theme);
    write_document(path, &doc)
}

/// Remove the `[hosts.<id>]` entry from the default config file. Returns whether
/// it existed.
pub fn remove_host(id: &str) -> Result<bool> {
    remove_host_from(&paths::config_file()?, id)
}

/// Remove the `[hosts.<id>]` entry from a specific config file, preserving the rest
/// of the file. Returns whether the entry existed.
pub fn remove_host_from(path: &Path, id: &str) -> Result<bool> {
    let mut doc = read_document(path)?;
    let Some(hosts) = doc.get_mut("hosts").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let existed = hosts.remove(id).is_some();
    if hosts.is_empty() {
        doc.remove("hosts");
    }
    if existed {
        write_document(path, &doc)?;
    }
    Ok(existed)
}

/// Parse the config file into an editable document, or an empty document when the
/// file does not exist.
fn read_document(path: &Path) -> Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(contents) => contents
            .parse::<DocumentMut>()
            .map_err(|e| CoreError::Config(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(CoreError::Config(format!("{}: {e}", path.display()))),
    }
}

/// Write only the keys SysTUI manages onto a host table; optional/false fields are
/// removed so editing a host does not leave stale keys behind.
fn write_host_fields(table: &mut Table, host: &Host) {
    table["host"] = value(host.host.as_str());
    set_or_remove_str(table, "user", host.user.as_deref());
    table["port"] = value(i64::from(host.port));
    if host.tags.is_empty() {
        table.remove("tags");
    } else {
        let mut tags = Array::new();
        for tag in &host.tags {
            tags.push(tag.as_str());
        }
        table["tags"] = value(tags);
    }
    set_or_remove_bool(table, "read_only", host.read_only);
    set_or_remove_bool(table, "favorite", host.favorite);
    set_or_remove_str(table, "policy", host.policy.as_deref());
}

fn set_or_remove_str(table: &mut Table, key: &str, val: Option<&str>) {
    match val {
        Some(v) => table[key] = value(v),
        None => {
            table.remove(key);
        }
    }
}

fn set_or_remove_bool(table: &mut Table, key: &str, val: bool) {
    if val {
        table[key] = value(true);
    } else {
        table.remove(key);
    }
}

/// Write the document atomically: to a sibling temp file, then rename into place,
/// so a crash mid-write never leaves a half-written `config.toml`.
fn write_document(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Config(format!("{}: {e}", parent.display())))?;
        }
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())
        .map_err(|e| CoreError::Config(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| CoreError::Config(format!("{}: {e}", path.display())))
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

    fn sample_host() -> Host {
        Host {
            host: "10.0.0.1".to_owned(),
            user: Some("admin".to_owned()),
            port: 2222,
            tags: vec!["web".to_owned(), "prod".to_owned()],
            read_only: true,
            favorite: false,
            policy: None,
        }
    }

    #[test]
    fn save_host_preserves_existing_content_and_comments() {
        let path = tmp_path("save.toml");
        std::fs::write(
            &path,
            "# my config\n[general]\ntheme = \"light\"  # keep this\n",
        )
        .unwrap();

        save_host_to(&path, "prod-01", &sample_host()).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        // The comment and the existing table survive.
        assert!(raw.contains("# my config"));
        assert!(raw.contains("# keep this"));
        assert!(raw.contains("[hosts.prod-01]"));

        // And it parses back into a Config with the host present.
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.general.theme, "light");
        let host = &cfg.hosts["prod-01"];
        assert_eq!(host.host, "10.0.0.1");
        assert_eq!(host.user.as_deref(), Some("admin"));
        assert_eq!(host.port, 2222);
        assert_eq!(host.tags, ["web", "prod"]);
        assert!(host.read_only);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_host_into_missing_file_creates_it() {
        let path = tmp_path("new.toml");
        save_host_to(&path, "db-01", &sample_host()).unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.hosts["db-01"].host, "10.0.0.1");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn updating_a_host_drops_cleared_optional_fields() {
        let path = tmp_path("update.toml");
        save_host_to(&path, "h", &sample_host()).unwrap();

        // Re-save with user/tags cleared and read_only off.
        let mut updated = sample_host();
        updated.user = None;
        updated.tags = Vec::new();
        updated.read_only = false;
        save_host_to(&path, "h", &updated).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("user"));
        assert!(!raw.contains("tags"));
        assert!(!raw.contains("read_only"));

        let cfg = load_from(&path).unwrap();
        let host = &cfg.hosts["h"];
        assert_eq!(host.user, None);
        assert!(host.tags.is_empty());
        assert!(!host.read_only);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn remove_host_reports_existence_and_keeps_other_content() {
        let path = tmp_path("remove.toml");
        std::fs::write(&path, "[general]\ntheme = \"dark\"\n").unwrap();
        save_host_to(&path, "a", &sample_host()).unwrap();
        save_host_to(&path, "b", &sample_host()).unwrap();

        assert!(remove_host_from(&path, "a").unwrap());
        // Removing a second time reports it was already gone.
        assert!(!remove_host_from(&path, "a").unwrap());

        let cfg = load_from(&path).unwrap();
        assert!(!cfg.hosts.contains_key("a"));
        assert!(cfg.hosts.contains_key("b"));
        assert_eq!(cfg.general.theme, "dark");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_theme_sets_general_theme_and_preserves_hosts() {
        let path = tmp_path("theme.toml");
        std::fs::write(&path, "# top\n[general]\ndefault_refresh_seconds = 7\n").unwrap();
        save_host_to(&path, "h", &sample_host()).unwrap();

        save_general_theme_to(&path, "midnight").unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# top"));
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.general.theme, "midnight");
        // Unrelated general keys and hosts survive the edit.
        assert_eq!(cfg.general.default_refresh_seconds, 7);
        assert!(cfg.hosts.contains_key("h"));

        // Switching again overwrites the previous value.
        save_general_theme_to(&path, "light").unwrap();
        assert_eq!(load_from(&path).unwrap().general.theme, "light");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_theme_into_missing_file_creates_it() {
        let path = tmp_path("theme-new.toml");
        save_general_theme_to(&path, "dark").unwrap();
        assert_eq!(load_from(&path).unwrap().general.theme, "dark");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn removing_last_host_drops_empty_hosts_table() {
        let path = tmp_path("lasthost.toml");
        save_host_to(&path, "only", &sample_host()).unwrap();
        assert!(remove_host_from(&path, "only").unwrap());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("hosts"));
        std::fs::remove_file(&path).unwrap();
    }
}
