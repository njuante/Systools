//! Exporting the current logs view to JSON for fast incident capture.
//!
//! The UI requests an export (a flag on [`App`]); the event loop performs the
//! file write off the render path. The dump is a plain JSON document — host,
//! timestamp, the active filter, an error-fingerprint aggregation and the raw
//! entries — so a failing host's logs can be grabbed and shared quickly.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Local;
use serde_json::json;
use systui_core::{CoreError, Result};

use crate::app::App;

/// Write the current logs view to a timestamped JSON file under the exports
/// directory, returning the path written.
pub fn export_logs(app: &App) -> Result<PathBuf> {
    let dir = systui_storage::paths::exports_dir()?;
    std::fs::create_dir_all(&dir)?;

    let now = Local::now();
    let safe_host: String = app
        .host_label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = dir.join(format!(
        "systui-logs-{safe_host}-{}.json",
        now.format("%Y%m%d-%H%M%S")
    ));

    let body = serde_json::to_string_pretty(&logs_json(app))
        .map_err(|e| CoreError::Config(format!("serialising logs: {e}")))?;
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Build the export document for the current logs view: host, timestamp, the
/// active filter, an error-fingerprint aggregation and the raw entries. Pure
/// (no I/O) so it can be tested directly.
pub fn logs_json(app: &App) -> serde_json::Value {
    // Error/warning fingerprints: group the error lines by a normalised key so
    // the export carries a ready-made "what is failing" summary.
    let mut fingerprints: BTreeMap<String, u64> = BTreeMap::new();
    for entry in app.logs.iter().filter(|e| e.is_error()) {
        *fingerprints.entry(fingerprint(&entry.message)).or_default() += 1;
    }
    let mut fingerprint_list: Vec<_> = fingerprints
        .into_iter()
        .map(|(pattern, count)| json!({ "pattern": pattern, "count": count }))
        .collect();
    fingerprint_list.sort_by_key(|v| std::cmp::Reverse(v["count"].as_u64().unwrap_or(0)));

    json!({
        "host": app.host_label,
        "generated_at": Local::now().to_rfc3339(),
        "filter": {
            "level": app.log_level_label(),
            "window": app.log_window_label(),
            "search": app.log_search,
        },
        "count": app.logs.len(),
        "error_fingerprints": fingerprint_list,
        "entries": app.logs,
    })
}

/// Normalise a log message into a fingerprint: lowercased, with digit runs
/// collapsed to `#` so otherwise-identical errors group together.
fn fingerprint(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut prev_digit = false;
    for c in message.chars() {
        if c.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            out.extend(c.to_lowercase());
            prev_digit = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use systui_collectors::LogEntry;
    use systui_core::ExecutionMode;

    #[test]
    fn logs_json_captures_entries_and_fingerprints() {
        let mut app = App::new("prod-01", ExecutionMode::ReadOnly);
        app.logs = vec![
            LogEntry {
                time: "09:00:01".to_owned(),
                priority: 3,
                identifier: "nginx".to_owned(),
                message: "upstream 10.0.0.1 timed out".to_owned(),
            },
            LogEntry {
                time: "09:00:02".to_owned(),
                priority: 3,
                identifier: "nginx".to_owned(),
                message: "upstream 10.0.0.9 timed out".to_owned(),
            },
            LogEntry {
                time: "09:00:03".to_owned(),
                priority: 6,
                identifier: "systemd".to_owned(),
                message: "started something".to_owned(),
            },
        ];

        let value = logs_json(&app);
        assert_eq!(value["host"], "prod-01");
        assert_eq!(value["count"], 3);
        assert_eq!(value["entries"].as_array().unwrap().len(), 3);

        // The two timeouts differ only by IP digits, so they fingerprint together.
        let prints = value["error_fingerprints"].as_array().unwrap();
        assert_eq!(prints.len(), 1);
        assert_eq!(prints[0]["count"], 2);
        assert!(
            prints[0]["pattern"]
                .as_str()
                .unwrap()
                .contains("upstream")
        );

        // The whole document round-trips as valid JSON.
        let s = serde_json::to_string(&value).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&s).is_ok());
    }
}
