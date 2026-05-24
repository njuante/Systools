//! Process signal actions (SIGTERM / SIGKILL / SIGHUP) with guardrails.
//!
//! Guardrails are hard blocks: PID 1 and the SysTUI process itself are never
//! signaled. Signaling other critical processes (sshd, systemd, …) is left to the
//! engine's strong-confirmation step (S2.7); this layer only protects against the
//! always-wrong cases.

use async_trait::async_trait;
use systui_core::{
    Action, ActionOutcome, ActionPreview, CommandSpec, CoreError, ModuleId, Result, RiskLevel,
    Transport,
};

/// A POSIX signal SysTUI can send to a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Graceful termination (SIGTERM).
    Term,
    /// Forceful kill (SIGKILL).
    Kill,
    /// Hangup / reload (SIGHUP).
    Hup,
}

impl Signal {
    /// The signal name passed to `kill -s`.
    pub fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Kill => "KILL",
            Signal::Hup => "HUP",
        }
    }

    fn risk(self) -> RiskLevel {
        match self {
            Signal::Hup => RiskLevel::Low,
            Signal::Term => RiskLevel::Medium,
            Signal::Kill => RiskLevel::High,
        }
    }

    /// Whether the signal is expected to terminate the process.
    fn terminates(self) -> bool {
        matches!(self, Signal::Term | Signal::Kill)
    }
}

/// An action that sends a [`Signal`] to a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalAction {
    pub pid: u32,
    pub signal: Signal,
    pub name: String,
}

impl SignalAction {
    pub fn new(signal: Signal, pid: u32, name: impl Into<String>) -> Self {
        Self {
            pid,
            signal,
            name: name.into(),
        }
    }

    fn target(&self) -> String {
        if self.name.is_empty() {
            format!("PID {}", self.pid)
        } else {
            format!("{} (PID {})", self.name, self.pid)
        }
    }

    fn command(&self) -> CommandSpec {
        CommandSpec::new("kill")
            .arg("-s")
            .arg(self.signal.name())
            .arg(self.pid.to_string())
    }
}

#[async_trait]
impl Action for SignalAction {
    fn module(&self) -> ModuleId {
        ModuleId::Processes
    }

    fn risk(&self) -> RiskLevel {
        self.signal.risk()
    }

    fn requires_privilege(&self) -> bool {
        // Signaling your own processes needs no privilege; signaling others fails
        // with a permission error, which is surfaced as-is.
        false
    }

    fn guardrail(&self) -> Option<String> {
        if self.pid == 1 {
            Some("PID 1 (init) is protected and cannot be signaled".to_owned())
        } else if self.pid == std::process::id() {
            Some("refusing to signal the SysTUI process itself".to_owned())
        } else {
            None
        }
    }

    async fn preview(&self, _transport: &dyn Transport) -> Result<ActionPreview> {
        let mut details = vec![format!(
            "Sends SIG{} to {}.",
            self.signal.name(),
            self.target()
        )];
        if let Some(reason) = self.guardrail() {
            details.push(format!("BLOCKED: {reason}"));
        }
        Ok(ActionPreview {
            summary: format!("Send SIG{} to {}", self.signal.name(), self.target()),
            details,
            command: Some(self.command()),
            reversible: false,
            creates_backup: false,
        })
    }

    async fn execute(&self, transport: &dyn Transport) -> Result<ActionOutcome> {
        if let Some(reason) = self.guardrail() {
            return Err(CoreError::InvalidInput(reason));
        }

        let output = transport.run(&self.command()).await?;
        if !output.success() {
            return Ok(ActionOutcome {
                success: false,
                message: format!(
                    "failed to signal {}: {}",
                    self.target(),
                    output.stderr.trim()
                ),
            });
        }

        let state = if self.signal.terminates() {
            self.verify_gone(transport).await
        } else {
            "signal delivered".to_owned()
        };
        Ok(ActionOutcome {
            success: true,
            message: format!(
                "SIG{} sent to {} — {state}",
                self.signal.name(),
                self.target()
            ),
        })
    }
}

impl SignalAction {
    /// Check whether the process is gone, via `kill -0` (best-effort).
    async fn verify_gone(&self, transport: &dyn Transport) -> String {
        let spec = CommandSpec::new("kill").arg("-0").arg(self.pid.to_string());
        match transport.run(&spec).await {
            Ok(output) if output.success() => "still running".to_owned(),
            _ => "process exited".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    fn out(code: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(code),
            stdout: String::new(),
            stderr: stderr.to_owned(),
            duration: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn risk_classification() {
        assert_eq!(
            SignalAction::new(Signal::Hup, 10, "x").risk(),
            RiskLevel::Low
        );
        assert_eq!(
            SignalAction::new(Signal::Kill, 10, "x").risk(),
            RiskLevel::High
        );
        assert!(!SignalAction::new(Signal::Term, 10, "x").requires_privilege());
    }

    #[test]
    fn guardrail_blocks_pid_1_and_self() {
        assert!(
            SignalAction::new(Signal::Term, 1, "init")
                .guardrail()
                .is_some()
        );
        let me = std::process::id();
        assert!(
            SignalAction::new(Signal::Kill, me, "systui")
                .guardrail()
                .is_some()
        );
        assert!(
            SignalAction::new(Signal::Term, 4410, "bash")
                .guardrail()
                .is_none()
        );
    }

    #[tokio::test]
    async fn execute_refuses_protected_pid() {
        let action = SignalAction::new(Signal::Kill, 1, "init");
        let err = action.execute(&MockTransport::new()).await.unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn execute_terminates_and_verifies() {
        let transport = MockTransport::new()
            .with_command("kill -s TERM 4410", out(0, ""))
            .with_command("kill -0 4410", out(1, "")); // gone
        let action = SignalAction::new(Signal::Term, 4410, "bash");
        let outcome = action.execute(&transport).await.unwrap();
        assert!(outcome.success);
        assert!(outcome.message.contains("process exited"));
    }

    #[tokio::test]
    async fn execute_reports_permission_failure() {
        let transport = MockTransport::new()
            .with_command("kill -s TERM 999", out(1, "Operation not permitted"));
        let action = SignalAction::new(Signal::Term, 999, "root-proc");
        let outcome = action.execute(&transport).await.unwrap();
        assert!(!outcome.success);
        assert!(outcome.message.contains("not permitted"));
    }
}
