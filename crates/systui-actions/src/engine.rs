//! The action engine: the single safety pipeline every mutation passes through.
//!
//! ```text
//! guardrail → read-only/permission → risk → preview → confirmation
//!   → backup → execute → verify
//! ```
//!
//! The engine is transport-agnostic and does not touch the UI: callers `plan` an
//! action to learn whether it is allowed and what confirmation it needs, then
//! `execute` it with the confirmation the user supplied. Audit persistence is
//! layered on in S2.6.

use std::time::Instant;

use systui_core::{
    Action, ActionOutcome, ActionPreview, AuditContext, AuditRecord, AuditStatus, CoreError,
    ExecutionMode, Result, RiskLevel, Transport,
};

/// The result of planning an action against the current mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionDecision {
    /// Hard-blocked (guardrail) or not permitted in the current mode.
    Rejected(String),
    /// Permitted but requires the user to type the given confirmation phrase.
    NeedsConfirmation {
        preview: ActionPreview,
        phrase: String,
    },
    /// Permitted with no typed confirmation (low risk).
    Ready { preview: ActionPreview },
}

/// Drives actions through the safety pipeline for a given [`ExecutionMode`].
#[derive(Debug, Clone, Copy)]
pub struct ActionEngine {
    mode: ExecutionMode,
}

impl ActionEngine {
    pub fn new(mode: ExecutionMode) -> Self {
        Self { mode }
    }

    /// Risk at or above which a typed confirmation phrase is required.
    const CONFIRM_AT: RiskLevel = RiskLevel::Medium;

    /// Plan an action: check guardrails and mode, build the preview, and decide
    /// what confirmation (if any) is required.
    pub async fn plan(
        &self,
        action: &dyn Action,
        transport: &dyn Transport,
    ) -> Result<ActionDecision> {
        if let Some(reason) = action.guardrail() {
            return Ok(ActionDecision::Rejected(format!("blocked: {reason}")));
        }
        if !self.mode.allows(action.risk()) {
            return Ok(ActionDecision::Rejected(format!(
                "{:?} actions are not allowed in {} mode",
                action.risk(),
                self.mode
            )));
        }

        let preview = action.preview(transport).await?;
        if action.risk() >= Self::CONFIRM_AT {
            let phrase = preview.summary.clone();
            Ok(ActionDecision::NeedsConfirmation { preview, phrase })
        } else {
            Ok(ActionDecision::Ready { preview })
        }
    }

    /// Execute an action after re-checking guardrails, mode and confirmation.
    ///
    /// `confirmation` is the phrase the user typed; it must match the action's
    /// preview summary (case-insensitive) for risky actions.
    pub async fn execute(
        &self,
        action: &dyn Action,
        transport: &dyn Transport,
        confirmation: Option<&str>,
    ) -> Result<ActionOutcome> {
        if let Some(reason) = action.guardrail() {
            return Err(CoreError::InvalidInput(reason));
        }

        let preview = action.preview(transport).await?;

        if !self.mode.allows(action.risk()) {
            return Err(CoreError::ModeForbidden {
                mode: self.mode,
                action: preview.summary,
            });
        }

        if action.risk() >= Self::CONFIRM_AT {
            let confirmed =
                confirmation.is_some_and(|c| c.trim().eq_ignore_ascii_case(preview.summary.trim()));
            if !confirmed {
                return Err(CoreError::InvalidInput(format!(
                    "confirmation did not match; type: {}",
                    preview.summary
                )));
            }
        }

        // Backup step is a no-op for service/signal actions (nothing to back up);
        // cron/config edits will create backups here in later phases.

        action.execute(transport).await
    }

    /// Execute an action and produce an [`AuditRecord`] for it.
    ///
    /// Returns the execution result alongside the record so the caller can both
    /// react to the outcome and persist the audit entry (via `systui-storage`).
    pub async fn run(
        &self,
        action: &dyn Action,
        transport: &dyn Transport,
        confirmation: Option<&str>,
        ctx: &AuditContext,
    ) -> (Result<ActionOutcome>, AuditRecord) {
        let start = Instant::now();
        let result = self.execute(action, transport, confirmation).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let status = match &result {
            Ok(outcome) if outcome.success => AuditStatus::Success,
            Ok(_) => AuditStatus::Failure,
            Err(_) => AuditStatus::Rejected,
        };
        let action_label = action
            .preview(transport)
            .await
            .map(|p| p.summary)
            .unwrap_or_else(|_| action.target());

        let record = AuditRecord {
            timestamp: chrono::Utc::now().to_rfc3339(),
            host: ctx.host.clone(),
            user: ctx.user.clone(),
            module: action.module(),
            action: action_label,
            target: action.target(),
            status,
            duration_ms,
        };
        (result, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ServiceAction, ServiceOp, Signal, SignalAction};
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

    #[tokio::test]
    async fn read_only_mode_rejects_mutations() {
        let engine = ActionEngine::new(ExecutionMode::ReadOnly);
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");
        let decision = engine.plan(&action, &MockTransport::new()).await.unwrap();
        assert!(matches!(decision, ActionDecision::Rejected(_)));

        let err = engine
            .execute(
                &action,
                &MockTransport::new(),
                Some("Restart nginx.service"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::ModeForbidden { .. }));
    }

    #[tokio::test]
    async fn medium_risk_needs_matching_confirmation() {
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");

        match engine.plan(&action, &MockTransport::new()).await.unwrap() {
            ActionDecision::NeedsConfirmation { phrase, .. } => {
                assert_eq!(phrase, "Restart nginx.service");
            }
            other => panic!("expected confirmation, got {other:?}"),
        }

        // Wrong confirmation is refused.
        let err = engine
            .execute(&action, &MockTransport::new(), Some("nope"))
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput(_)));

        // Correct confirmation executes.
        let transport = MockTransport::new()
            .with_command("systemctl restart nginx.service", ok(""))
            .with_command("systemctl is-active nginx.service", ok("active\n"));
        let outcome = engine
            .execute(&action, &transport, Some("restart nginx.service"))
            .await
            .unwrap();
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn low_risk_is_ready_without_confirmation() {
        let engine = ActionEngine::new(ExecutionMode::SafeActions);
        let action = ServiceAction::new(ServiceOp::Reload, "nginx.service");

        assert!(matches!(
            engine.plan(&action, &MockTransport::new()).await.unwrap(),
            ActionDecision::Ready { .. }
        ));

        let transport = MockTransport::new()
            .with_command("systemctl reload nginx.service", ok(""))
            .with_command("systemctl is-active nginx.service", ok("active\n"));
        let outcome = engine.execute(&action, &transport, None).await.unwrap();
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn safe_mode_blocks_high_risk() {
        let engine = ActionEngine::new(ExecutionMode::SafeActions);
        let action = ServiceAction::new(ServiceOp::Stop, "nginx.service");
        assert!(matches!(
            engine.plan(&action, &MockTransport::new()).await.unwrap(),
            ActionDecision::Rejected(_)
        ));
    }

    #[tokio::test]
    async fn guardrail_blocks_protected_pid() {
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = SignalAction::new(Signal::Kill, 1, "init");
        assert!(matches!(
            engine.plan(&action, &MockTransport::new()).await.unwrap(),
            ActionDecision::Rejected(_)
        ));
    }

    fn ctx() -> systui_core::AuditContext {
        systui_core::AuditContext {
            host: "local".to_owned(),
            user: "admin".to_owned(),
        }
    }

    #[tokio::test]
    async fn run_produces_success_audit_record() {
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");
        let transport = MockTransport::new()
            .with_command("systemctl restart nginx.service", ok(""))
            .with_command("systemctl is-active nginx.service", ok("active\n"));

        let (result, record) = engine
            .run(&action, &transport, Some("restart nginx.service"), &ctx())
            .await;
        assert!(result.unwrap().success);
        assert_eq!(record.status, systui_core::AuditStatus::Success);
        assert_eq!(record.action, "Restart nginx.service");
        assert_eq!(record.target, "nginx.service");
        assert_eq!(record.user, "admin");
    }

    #[tokio::test]
    async fn run_records_rejection_in_read_only() {
        let engine = ActionEngine::new(ExecutionMode::ReadOnly);
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");
        let (result, record) = engine
            .run(&action, &MockTransport::new(), None, &ctx())
            .await;
        assert!(result.is_err());
        assert_eq!(record.status, systui_core::AuditStatus::Rejected);
    }

    #[tokio::test]
    async fn run_records_failure_when_execution_does_not_succeed() {
        // Start is Low risk, so it is Ready without confirmation; the command
        // itself fails (non-zero exit) so the outcome is an unsuccessful run,
        // which must be audited as Failure, distinct from a Rejected gate.
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = ServiceAction::new(ServiceOp::Start, "missing.service");
        let transport = MockTransport::new().with_command(
            "systemctl start missing.service",
            CommandOutput {
                exit_code: Some(5),
                stdout: String::new(),
                stderr: "Unit missing.service not found.".to_owned(),
                duration: std::time::Duration::ZERO,
            },
        );

        let (result, record) = engine.run(&action, &transport, None, &ctx()).await;
        assert!(!result.unwrap().success);
        assert_eq!(record.status, systui_core::AuditStatus::Failure);
    }

    #[tokio::test]
    async fn execute_rejects_guardrailed_action() {
        // The guardrail must also hold on the execute path, not only on plan.
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = SignalAction::new(Signal::Kill, 1, "init");
        let err = engine
            .execute(&action, &MockTransport::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn confirmation_is_trimmed_and_case_insensitive() {
        let engine = ActionEngine::new(ExecutionMode::Privileged);
        let action = ServiceAction::new(ServiceOp::Restart, "nginx.service");
        let transport = MockTransport::new()
            .with_command("systemctl restart nginx.service", ok(""))
            .with_command("systemctl is-active nginx.service", ok("active\n"));

        let outcome = engine
            .execute(&action, &transport, Some("  RESTART nginx.service  "))
            .await
            .unwrap();
        assert!(outcome.success);
    }
}
