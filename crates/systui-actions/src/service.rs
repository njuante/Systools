//! systemd service actions (start/stop/restart/reload/enable/disable/mask/unmask).
//!
//! Each implements the core [`Action`] contract: it describes itself (risk,
//! privilege, preview) and performs the raw execution + verification. The action
//! engine (S2.5) wraps these with read-only/permission/confirmation/audit.

use async_trait::async_trait;
use systui_core::{
    Action, ActionOutcome, ActionPreview, CommandSpec, ModuleId, Result, RiskLevel, Transport,
};

/// A systemd operation on a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceOp {
    Start,
    Stop,
    Restart,
    Reload,
    Enable,
    Disable,
    Mask,
    Unmask,
}

impl ServiceOp {
    /// The `systemctl` subcommand for this operation.
    pub fn subcommand(self) -> &'static str {
        match self {
            ServiceOp::Start => "start",
            ServiceOp::Stop => "stop",
            ServiceOp::Restart => "restart",
            ServiceOp::Reload => "reload",
            ServiceOp::Enable => "enable",
            ServiceOp::Disable => "disable",
            ServiceOp::Mask => "mask",
            ServiceOp::Unmask => "unmask",
        }
    }

    fn risk(self) -> RiskLevel {
        match self {
            ServiceOp::Start | ServiceOp::Reload | ServiceOp::Enable | ServiceOp::Unmask => {
                RiskLevel::Low
            }
            ServiceOp::Restart => RiskLevel::Medium,
            ServiceOp::Stop | ServiceOp::Disable | ServiceOp::Mask => RiskLevel::High,
        }
    }

    /// Whether the op changes the *running* state (vs. boot/enablement state).
    fn affects_runtime(self) -> bool {
        matches!(
            self,
            ServiceOp::Start | ServiceOp::Stop | ServiceOp::Restart | ServiceOp::Reload
        )
    }

    fn impact(self) -> &'static str {
        match self {
            ServiceOp::Start => "Starts the unit.",
            ServiceOp::Stop => "Stops the unit; active clients will be disconnected.",
            ServiceOp::Restart => "Restarts the unit; it will be briefly unavailable.",
            ServiceOp::Reload => "Reloads the unit's configuration without a full restart.",
            ServiceOp::Enable => "Enables the unit to start at boot.",
            ServiceOp::Disable => "Disables the unit from starting at boot.",
            ServiceOp::Mask => "Masks the unit; it cannot be started until unmasked.",
            ServiceOp::Unmask => "Unmasks the unit.",
        }
    }
}

/// An action that performs a [`ServiceOp`] on a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAction {
    pub unit: String,
    pub op: ServiceOp,
}

impl ServiceAction {
    pub fn new(op: ServiceOp, unit: impl Into<String>) -> Self {
        Self {
            unit: unit.into(),
            op,
        }
    }

    fn command(&self) -> CommandSpec {
        CommandSpec::new("systemctl")
            .arg(self.op.subcommand())
            .arg(&self.unit)
            .privileged()
    }

    fn summary(&self) -> String {
        let verb = self.op.subcommand();
        let mut chars = verb.chars();
        let titled = chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default();
        format!("{titled} {}", self.unit)
    }

    /// Query the resulting state after execution (best-effort, never errors).
    async fn verify(&self, transport: &dyn Transport) -> String {
        let query = if self.op.affects_runtime() {
            "is-active"
        } else {
            "is-enabled"
        };
        let spec = CommandSpec::new("systemctl").arg(query).arg(&self.unit);
        match transport.run(&spec).await {
            // is-active/is-enabled return non-zero for inactive/disabled, so read
            // stdout regardless of exit status.
            Ok(output) => format!("{} is {}", self.unit, output.stdout.trim()),
            Err(_) => format!("{} state unknown", self.unit),
        }
    }
}

#[async_trait]
impl Action for ServiceAction {
    fn module(&self) -> ModuleId {
        ModuleId::Services
    }

    fn risk(&self) -> RiskLevel {
        self.op.risk()
    }

    fn requires_privilege(&self) -> bool {
        true
    }

    async fn preview(&self, _transport: &dyn Transport) -> Result<ActionPreview> {
        Ok(ActionPreview {
            summary: self.summary(),
            details: vec![self.op.impact().to_owned()],
            command: Some(self.command()),
            reversible: matches!(
                self.op,
                ServiceOp::Start
                    | ServiceOp::Stop
                    | ServiceOp::Enable
                    | ServiceOp::Disable
                    | ServiceOp::Mask
                    | ServiceOp::Unmask
            ),
            creates_backup: false,
        })
    }

    async fn execute(&self, transport: &dyn Transport) -> Result<ActionOutcome> {
        let output = transport.run(&self.command()).await?;
        if !output.success() {
            return Ok(ActionOutcome {
                success: false,
                message: format!("{} failed: {}", self.summary(), output.stderr.trim()),
            });
        }
        let state = self.verify(transport).await;
        Ok(ActionOutcome {
            success: true,
            message: format!("{} — {state}", self.summary()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(0),
            stdout: stdout.to_owned(),
            stderr: String::new(),
            duration: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn risk_and_privilege_are_classified() {
        assert_eq!(
            ServiceAction::new(ServiceOp::Restart, "nginx.service").risk(),
            RiskLevel::Medium
        );
        assert_eq!(
            ServiceAction::new(ServiceOp::Stop, "nginx.service").risk(),
            RiskLevel::High
        );
        assert!(ServiceAction::new(ServiceOp::Start, "x").requires_privilege());
    }

    #[tokio::test]
    async fn preview_describes_the_action() {
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");
        let preview = action.preview(&MockTransport::new()).await.unwrap();
        assert_eq!(preview.summary, "Restart nginx.service");
        assert_eq!(
            preview.command.unwrap().to_string(),
            "systemctl restart nginx.service"
        );
    }

    #[tokio::test]
    async fn execute_runs_and_verifies() {
        let transport = MockTransport::new()
            .with_command("systemctl restart nginx.service", ok(""))
            .with_command("systemctl is-active nginx.service", ok("active\n"));
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");

        let outcome = action.execute(&transport).await.unwrap();
        assert!(outcome.success);
        assert!(outcome.message.contains("nginx.service is active"));
    }

    #[tokio::test]
    async fn execute_reports_failure() {
        let transport = MockTransport::new().with_command(
            "systemctl start missing.service",
            CommandOutput {
                exit_code: Some(5),
                stdout: String::new(),
                stderr: "Unit missing.service not found.".to_owned(),
                duration: std::time::Duration::ZERO,
            },
        );
        let action = ServiceAction::new(ServiceOp::Start, "missing.service");

        let outcome = action.execute(&transport).await.unwrap();
        assert!(!outcome.success);
        assert!(outcome.message.contains("not found"));
    }
}
