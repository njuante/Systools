//! Typed errors for the SysTUI core and its contracts.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

use crate::mode::ExecutionMode;

/// Convenience alias used throughout the workspace.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Errors produced by transports, collectors, actions and configuration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A command ran but exited unsuccessfully.
    #[error("command `{program}` failed (exit {code:?}): {stderr}")]
    CommandFailed {
        program: String,
        code: Option<i32>,
        stderr: String,
    },

    /// The program was not found on the target host.
    #[error("command not found: `{0}`")]
    CommandNotFound(String),

    /// The operation was denied by the OS (missing privileges).
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// The current [`ExecutionMode`] forbids the requested action.
    #[error("action `{action}` is not allowed in {mode} mode")]
    ModeForbidden { mode: ExecutionMode, action: String },

    /// A required file does not exist.
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),

    /// Caller-supplied input was invalid or unsafe.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The transport itself failed (connection, SSH, IO at the boundary).
    #[error("transport error: {0}")]
    Transport(String),

    /// An operation exceeded its time budget.
    #[error("timed out after {0:?}")]
    Timeout(Duration),

    /// Failed to parse command output or a file.
    #[error("parse error in {context}: {message}")]
    Parse { context: String, message: String },

    /// Configuration was missing or malformed.
    #[error("configuration error: {0}")]
    Config(String),

    /// An underlying IO error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl CoreError {
    /// Build a [`CoreError::Parse`] from a context label and message.
    pub fn parse(context: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Parse {
            context: context.into(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_forbidden_renders_clearly() {
        let e = CoreError::ModeForbidden {
            mode: ExecutionMode::ReadOnly,
            action: "restart nginx.service".into(),
        };
        assert_eq!(
            e.to_string(),
            "action `restart nginx.service` is not allowed in read-only mode"
        );
    }

    #[test]
    fn io_error_converts() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let e: CoreError = io.into();
        assert!(matches!(e, CoreError::Io(_)));
    }
}
