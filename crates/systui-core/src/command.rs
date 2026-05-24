//! Structured command specification and output.
//!
//! SysTUI never runs free-form command strings (`Product.md` §6 Fase 0): every
//! command is a [`CommandSpec`] with an explicit program and argument vector,
//! which removes a whole class of shell-injection and SSH-quoting bugs.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// A command to run via a transport. Program and arguments are kept separate so
/// no shell parsing ever happens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// The program to execute, e.g. `"systemctl"`.
    pub program: String,
    /// Arguments, passed verbatim (never shell-split), e.g. `["restart", "nginx"]`.
    pub args: Vec<String>,
    /// Whether this command needs elevated privileges (sudo/root).
    pub requires_privilege: bool,
    /// Optional time budget; transports should abort and return [`CoreError::Timeout`].
    pub timeout: Option<Duration>,
    /// Optional data written to the process's standard input. Lets commands be
    /// fed input without a shell pipe (e.g. piping a PEM into `openssl x509`).
    /// An empty string still closes stdin, so a command waiting on input
    /// (such as `openssl s_client`) returns instead of hanging.
    pub stdin: Option<String>,
}

impl CommandSpec {
    /// Start building a command for `program`.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            requires_privilege: false,
            timeout: None,
            stdin: None,
        }
    }

    /// Append a single argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Mark the command as requiring elevated privileges.
    #[must_use]
    pub fn privileged(mut self) -> Self {
        self.requires_privilege = true;
        self
    }

    /// Set a time budget for the command.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Provide data to write to the command's standard input.
    #[must_use]
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin = Some(input.into());
        self
    }
}

impl fmt::Display for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// The result of running a [`CommandSpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutput {
    /// Exit code, or `None` if the process was terminated by a signal.
    pub exit_code: Option<i32>,
    /// Captured standard output (UTF-8, lossily decoded by the transport).
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Wall-clock duration of the command.
    pub duration: Duration,
}

impl CommandOutput {
    /// `true` when the command exited with status code 0.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Iterator over non-empty trimmed stdout lines.
    pub fn stdout_lines(&self) -> impl Iterator<Item = &str> {
        self.stdout.lines()
    }

    /// Convert a non-zero exit into [`CoreError::CommandFailed`].
    pub fn into_result(self, program: &str) -> Result<Self> {
        if self.success() {
            Ok(self)
        } else {
            Err(CoreError::CommandFailed {
                program: program.to_owned(),
                code: self.exit_code,
                stderr: self.stderr,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_produces_expected_spec() {
        let spec = CommandSpec::new("systemctl")
            .arg("restart")
            .arg("nginx.service")
            .privileged();
        assert_eq!(spec.program, "systemctl");
        assert_eq!(spec.args, ["restart", "nginx.service"]);
        assert!(spec.requires_privilege);
    }

    #[test]
    fn display_is_shell_like_but_not_shell() {
        let spec = CommandSpec::new("ss").args(["-tulpn"]);
        assert_eq!(spec.to_string(), "ss -tulpn");
    }

    #[test]
    fn success_and_into_result() {
        let ok = CommandOutput {
            exit_code: Some(0),
            stdout: "a\nb\n".into(),
            stderr: String::new(),
            duration: Duration::from_millis(5),
        };
        assert!(ok.success());
        assert_eq!(ok.stdout_lines().collect::<Vec<_>>(), ["a", "b"]);
        assert!(ok.into_result("ss").is_ok());

        let bad = CommandOutput {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "boom".into(),
            duration: Duration::from_millis(1),
        };
        assert!(bad.into_result("ss").is_err());
    }
}
