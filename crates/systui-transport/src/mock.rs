//! In-memory transport for tests: returns pre-programmed responses keyed by the
//! command's display form, with no access to the real machine.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use systui_core::{CommandOutput, CommandSpec, CoreError, DirEntry, Result, Transport};

/// A configurable, deterministic transport used by unit tests.
///
/// Commands are matched on their [`CommandSpec`] display form (e.g.
/// `"ss -tulpn"`). An unconfigured command returns a [`CoreError::Transport`]
/// so tests fail loudly instead of silently.
#[derive(Debug, Clone, Default)]
pub struct MockTransport {
    label: String,
    commands: HashMap<String, CommandOutput>,
    files: HashMap<String, Vec<u8>>,
    dirs: HashMap<String, Vec<DirEntry>>,
}

impl MockTransport {
    /// Create an empty mock transport labelled `"mock"`.
    pub fn new() -> Self {
        Self {
            label: "mock".to_owned(),
            ..Self::default()
        }
    }

    /// Override the transport label (e.g. to mimic `"ssh://admin@prod-01"`).
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Register a full [`CommandOutput`] for a command line.
    #[must_use]
    pub fn with_command(mut self, command_line: impl Into<String>, output: CommandOutput) -> Self {
        self.commands.insert(command_line.into(), output);
        self
    }

    /// Register a successful command that emits `stdout`.
    #[must_use]
    pub fn with_stdout(
        mut self,
        command_line: impl Into<String>,
        stdout: impl Into<String>,
    ) -> Self {
        self.commands.insert(
            command_line.into(),
            CommandOutput {
                exit_code: Some(0),
                stdout: stdout.into(),
                stderr: String::new(),
                duration: Duration::ZERO,
            },
        );
        self
    }

    /// Register file contents at `path`.
    #[must_use]
    pub fn with_file(mut self, path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.into(), contents.into());
        self
    }

    /// Register directory listing at `path`.
    #[must_use]
    pub fn with_dir(mut self, path: impl Into<String>, entries: Vec<DirEntry>) -> Self {
        self.dirs.insert(path.into(), entries);
        self
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn label(&self) -> &str {
        &self.label
    }

    async fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
        let key = command.to_string();
        self.commands.get(&key).cloned().ok_or_else(|| {
            CoreError::Transport(format!("mock: no response configured for `{key}`"))
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| CoreError::FileNotFound(PathBuf::from(path)))
    }

    async fn file_exists(&self, path: &str) -> Result<bool> {
        Ok(self.files.contains_key(path) || self.dirs.contains_key(path))
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        self.dirs
            .get(path)
            .cloned()
            .ok_or_else(|| CoreError::FileNotFound(PathBuf::from(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::FileType;

    #[tokio::test]
    async fn returns_configured_command() {
        let t = MockTransport::new().with_stdout("ss -tulpn", "Netid State Recv-Q");
        let out = t.run(&CommandSpec::new("ss").arg("-tulpn")).await.unwrap();
        assert!(out.success());
        assert!(out.stdout.contains("Netid"));
    }

    #[tokio::test]
    async fn unconfigured_command_errors_loudly() {
        let t = MockTransport::new();
        let err = t.run(&CommandSpec::new("uptime")).await.unwrap_err();
        assert!(matches!(err, CoreError::Transport(_)));
    }

    #[tokio::test]
    async fn serves_files_and_existence() {
        let t = MockTransport::new().with_file("/etc/hostname", b"prod-01\n".to_vec());
        assert!(t.file_exists("/etc/hostname").await.unwrap());
        assert_eq!(t.read_file("/etc/hostname").await.unwrap(), b"prod-01\n");
        assert!(!t.file_exists("/missing").await.unwrap());
        assert!(matches!(
            t.read_file("/missing").await.unwrap_err(),
            CoreError::FileNotFound(_)
        ));
    }

    #[tokio::test]
    async fn serves_directory_listings() {
        let entries = vec![DirEntry {
            name: "backup.sh".to_owned(),
            file_type: FileType::File,
        }];
        let t = MockTransport::new().with_dir("/etc/cron.d", entries);
        assert!(t.file_exists("/etc/cron.d").await.unwrap());
        let listed = t.list_dir("/etc/cron.d").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "backup.sh");
    }

    #[tokio::test]
    async fn custom_label() {
        let t = MockTransport::new().with_label("ssh://admin@prod-01");
        assert_eq!(t.label(), "ssh://admin@prod-01");
    }
}
