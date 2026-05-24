//! Transport that executes against the local machine.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use systui_core::{CommandOutput, CommandSpec, CoreError, DirEntry, FileType, Result, Transport};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Runs commands and reads files on the current host.
///
/// Privilege escalation is intentionally *not* handled here: `requires_privilege`
/// on a [`CommandSpec`] is informational for the action engine (phase 2). A
/// command that needs root simply runs as the current user and will fail with a
/// permission error if it lacks rights.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalTransport;

impl LocalTransport {
    /// Create a local transport.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Transport for LocalTransport {
    fn label(&self) -> &str {
        "local"
    }

    async fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
        let start = Instant::now();

        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);
        cmd.kill_on_drop(true);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(if command.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let exec = run_child(&mut cmd, command.stdin.as_deref());
        let output = match command.timeout {
            Some(timeout) => tokio::time::timeout(timeout, exec)
                .await
                .map_err(|_| CoreError::Timeout(timeout))?,
            None => exec.await,
        };

        let output = output.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => CoreError::CommandNotFound(command.program.clone()),
            std::io::ErrorKind::PermissionDenied => {
                CoreError::PermissionDenied(command.program.clone())
            }
            _ => CoreError::Io(e),
        })?;

        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration: start.elapsed(),
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        tokio::fs::read(path).await.map_err(|e| map_fs_err(e, path))
    }

    async fn file_exists(&self, path: &str) -> Result<bool> {
        match tokio::fs::symlink_metadata(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(CoreError::PermissionDenied(path.to_owned()))
            }
            Err(e) => Err(CoreError::Io(e)),
        }
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let mut read_dir = tokio::fs::read_dir(path)
            .await
            .map_err(|e| map_fs_err(e, path))?;

        let mut entries = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.map_err(CoreError::Io)? {
            let ft = entry.file_type().await.map_err(CoreError::Io)?;
            let file_type = if ft.is_symlink() {
                FileType::Symlink
            } else if ft.is_dir() {
                FileType::Dir
            } else if ft.is_file() {
                FileType::File
            } else {
                FileType::Other
            };
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                file_type,
            });
        }
        Ok(entries)
    }
}

/// Spawn `cmd`, optionally writing `input` to its stdin, and collect its output.
async fn run_child(
    cmd: &mut Command,
    input: Option<&str>,
) -> std::io::Result<std::process::Output> {
    let mut child = cmd.spawn()?;
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(input.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    child.wait_with_output().await
}

fn map_fs_err(e: std::io::Error, path: &str) -> CoreError {
    match e.kind() {
        std::io::ErrorKind::NotFound => CoreError::FileNotFound(PathBuf::from(path)),
        std::io::ErrorKind::PermissionDenied => CoreError::PermissionDenied(path.to_owned()),
        _ => CoreError::Io(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(suffix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("systui-{}-{nanos}-{suffix}", std::process::id()))
    }

    #[tokio::test]
    async fn runs_echo() {
        let t = LocalTransport::new();
        let out = t.run(&CommandSpec::new("echo").arg("hello")).await.unwrap();
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn writes_stdin_to_the_process() {
        let t = LocalTransport::new();
        let out = t
            .run(&CommandSpec::new("cat").stdin("piped input"))
            .await
            .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, "piped input");
    }

    #[tokio::test]
    async fn unknown_program_is_command_not_found() {
        let t = LocalTransport::new();
        let err = t
            .run(&CommandSpec::new("systui-no-such-binary-xyz"))
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::CommandNotFound(_)));
    }

    #[tokio::test]
    async fn reads_file_and_reports_existence() {
        let path = tmp_path("file.txt");
        let path_str = path.to_str().unwrap();
        tokio::fs::write(&path, b"data").await.unwrap();

        let t = LocalTransport::new();
        assert!(t.file_exists(path_str).await.unwrap());
        assert_eq!(t.read_file(path_str).await.unwrap(), b"data");

        tokio::fs::remove_file(&path).await.unwrap();
        assert!(!t.file_exists(path_str).await.unwrap());
    }

    #[tokio::test]
    async fn missing_file_is_file_not_found() {
        let t = LocalTransport::new();
        let err = t
            .read_file("/nonexistent/systui/definitely/missing")
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::FileNotFound(_)));
    }

    #[tokio::test]
    async fn lists_directory_entries() {
        let dir = tmp_path("dir");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), b"x").await.unwrap();

        let t = LocalTransport::new();
        let entries = t.list_dir(dir.to_str().unwrap()).await.unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "a.txt" && e.file_type == FileType::File)
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
