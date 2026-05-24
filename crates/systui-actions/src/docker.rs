//! Docker lifecycle actions (start/stop/restart/remove).
//!
//! Each implements the core [`Action`] contract; the engine wraps them with the
//! read-only/permission/risk/confirmation/audit pipeline. Reading containers,
//! stats, inspect and logs is done by `systui-collectors` — this module only
//! mutates.

use async_trait::async_trait;
use systui_core::{
    Action, ActionOutcome, ActionPreview, CommandSpec, ModuleId, Result, RiskLevel, Transport,
};

/// A lifecycle operation on a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerOp {
    Start,
    Stop,
    Restart,
    Remove,
}

impl DockerOp {
    /// The `docker` subcommand.
    pub fn subcommand(self) -> &'static str {
        match self {
            DockerOp::Start => "start",
            DockerOp::Stop => "stop",
            DockerOp::Restart => "restart",
            DockerOp::Remove => "rm",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            DockerOp::Start => "Start",
            DockerOp::Stop => "Stop",
            DockerOp::Restart => "Restart",
            DockerOp::Remove => "Remove",
        }
    }

    fn risk(self) -> RiskLevel {
        match self {
            DockerOp::Start => RiskLevel::Low,
            DockerOp::Restart => RiskLevel::Medium,
            DockerOp::Stop => RiskLevel::High,
            DockerOp::Remove => RiskLevel::High,
        }
    }

    fn impact(self) -> &'static str {
        match self {
            DockerOp::Start => "Starts the container.",
            DockerOp::Stop => "Stops the container; its services go down.",
            DockerOp::Restart => "Restarts the container; it will be briefly unavailable.",
            DockerOp::Remove => {
                "Removes the container. This cannot be undone; a running container will not be removed unless it is stopped first."
            }
        }
    }
}

/// An action performing a [`DockerOp`] on a container (by name or id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerAction {
    pub container: String,
    pub op: DockerOp,
}

impl DockerAction {
    pub fn new(op: DockerOp, container: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            op,
        }
    }

    fn command(&self) -> CommandSpec {
        CommandSpec::new("docker")
            .arg(self.op.subcommand())
            .arg(&self.container)
            .privileged()
    }

    fn summary(&self) -> String {
        format!("{} container {}", self.op.verb(), self.container)
    }

    /// Report the resulting state (best-effort, never errors). For `Remove`,
    /// a failed inspect means the container is gone — the desired outcome.
    async fn verify(&self, transport: &dyn Transport) -> String {
        let spec = CommandSpec::new("docker").args([
            "inspect",
            "-f",
            "{{.State.Status}}",
            &self.container,
        ]);
        match transport.run(&spec).await {
            Ok(output) if output.success() => {
                format!("{} is {}", self.container, output.stdout.trim())
            }
            _ if self.op == DockerOp::Remove => format!("{} removed", self.container),
            _ => format!("{} state unknown", self.container),
        }
    }
}

#[async_trait]
impl Action for DockerAction {
    fn module(&self) -> ModuleId {
        ModuleId::Docker
    }

    fn risk(&self) -> RiskLevel {
        self.op.risk()
    }

    fn requires_privilege(&self) -> bool {
        true
    }

    fn target(&self) -> String {
        self.container.clone()
    }

    async fn preview(&self, _transport: &dyn Transport) -> Result<ActionPreview> {
        Ok(ActionPreview {
            summary: self.summary(),
            details: vec![self.op.impact().to_owned()],
            command: Some(self.command()),
            reversible: self.op != DockerOp::Remove,
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
    fn risk_is_classified_per_op() {
        assert_eq!(
            DockerAction::new(DockerOp::Start, "c").risk(),
            RiskLevel::Low
        );
        assert_eq!(
            DockerAction::new(DockerOp::Restart, "c").risk(),
            RiskLevel::Medium
        );
        assert_eq!(
            DockerAction::new(DockerOp::Stop, "c").risk(),
            RiskLevel::High
        );
        assert_eq!(
            DockerAction::new(DockerOp::Remove, "c").risk(),
            RiskLevel::High
        );
    }

    #[tokio::test]
    async fn preview_describes_command_and_reversibility() {
        let restart = DockerAction::new(DockerOp::Restart, "redis");
        let preview = restart.preview(&MockTransport::new()).await.unwrap();
        assert_eq!(preview.summary, "Restart container redis");
        assert_eq!(preview.command.unwrap().to_string(), "docker restart redis");
        assert!(preview.reversible);

        let remove = DockerAction::new(DockerOp::Remove, "redis");
        assert!(
            !remove
                .preview(&MockTransport::new())
                .await
                .unwrap()
                .reversible
        );
    }

    #[tokio::test]
    async fn execute_runs_and_verifies_state() {
        let transport = MockTransport::new()
            .with_command("docker restart redis", ok(""))
            .with_command("docker inspect -f {{.State.Status}} redis", ok("running\n"));
        let outcome = DockerAction::new(DockerOp::Restart, "redis")
            .execute(&transport)
            .await
            .unwrap();
        assert!(outcome.success);
        assert!(outcome.message.contains("redis is running"));
    }

    #[tokio::test]
    async fn remove_reports_gone_when_inspect_fails() {
        // After rm, inspect fails (no such container) — that is the success state.
        let transport = MockTransport::new().with_command("docker rm redis", ok(""));
        let outcome = DockerAction::new(DockerOp::Remove, "redis")
            .execute(&transport)
            .await
            .unwrap();
        assert!(outcome.success);
        assert!(outcome.message.contains("redis removed"));
    }

    #[tokio::test]
    async fn execute_reports_failure() {
        let transport = MockTransport::new().with_command(
            "docker stop redis",
            CommandOutput {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "No such container: redis".to_owned(),
                duration: std::time::Duration::ZERO,
            },
        );
        let outcome = DockerAction::new(DockerOp::Stop, "redis")
            .execute(&transport)
            .await
            .unwrap();
        assert!(!outcome.success);
        assert!(outcome.message.contains("No such container"));
    }
}
