//! Append-only local audit log: one JSON object per line at
//! `~/.local/share/systui/audit.log` (`Product.md` §3).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use systui_core::{AuditRecord, CoreError, Result};

use crate::paths;

/// Writer for the local audit log.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    /// Open the audit log at the default data-directory location.
    pub fn at_default_location() -> Result<Self> {
        Ok(Self {
            path: paths::data_dir()?.join("audit.log"),
        })
    }

    /// Use a specific path (useful for tests).
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append one record as a JSON line, creating the file/parent if needed.
    pub fn append(&self, record: &AuditRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(record)
            .map_err(|e| CoreError::Config(format!("audit serialization: {e}")))?;
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        // Restrict the audit trail to the owner: it records who did what on which
        // host and should not be world-readable on a shared machine.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::{AuditStatus, ModuleId};

    fn tmp_path() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("systui-audit-{}-{nanos}.log", std::process::id()))
    }

    fn record(action: &str) -> AuditRecord {
        AuditRecord {
            timestamp: "2026-05-24T10:00:00Z".to_owned(),
            host: "prod-01".to_owned(),
            user: "admin".to_owned(),
            module: ModuleId::Services,
            action: action.to_owned(),
            target: "nginx.service".to_owned(),
            status: AuditStatus::Success,
            duration_ms: 12,
        }
    }

    #[test]
    fn appends_json_lines_and_reads_them_back() {
        let path = tmp_path();
        let log = AuditLog::with_path(&path);

        log.append(&record("Restart nginx.service")).unwrap();
        log.append(&record("Reload nginx.service")).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: AuditRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.action, "Restart nginx.service");
        assert_eq!(first.status, AuditStatus::Success);

        std::fs::remove_file(&path).unwrap();
    }
}
