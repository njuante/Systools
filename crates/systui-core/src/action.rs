//! The action contract.
//!
//! The UI never executes commands; it requests an [`Action`]. The action engine
//! (in `systui-actions`, phase 2) wraps this trait with the full safety pipeline:
//! permission → read-only → risk → preview → confirm → backup → execute → verify
//! → audit (`Product.md` §10). An `Action` implementation only describes itself
//! and performs the raw execution.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::collector::ModuleId;
use crate::command::CommandSpec;
use crate::error::Result;
use crate::mode::RiskLevel;
use crate::transport::Transport;

/// A human-facing description of what an action will do, shown before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPreview {
    /// One-line summary, e.g. `"Restart nginx.service on prod-01"`.
    pub summary: String,
    /// Extra context lines (impact, affected targets, diffs).
    pub details: Vec<String>,
    /// The equivalent command, when there is a single representative one.
    pub command: Option<CommandSpec>,
    /// Whether the action can be undone.
    pub reversible: bool,
    /// Whether the engine will create a backup before applying.
    pub creates_backup: bool,
}

/// The result of executing an [`Action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// Whether the action achieved its goal.
    pub success: bool,
    /// Human-readable result message.
    pub message: String,
}

/// Something that mutates host state. Always driven by the action engine.
#[async_trait]
pub trait Action: Send + Sync {
    /// Which module this action belongs to.
    fn module(&self) -> ModuleId;

    /// The risk class, used for mode gating and confirmation strength.
    fn risk(&self) -> RiskLevel;

    /// Whether elevated privileges are required.
    fn requires_privilege(&self) -> bool;

    /// A reason this action is hard-blocked (e.g. a protected target), if any.
    /// The engine rejects blocked actions before preview/execution.
    fn guardrail(&self) -> Option<String> {
        None
    }

    /// The action's target (a unit name, PID, ...), used for auditing.
    fn target(&self) -> String {
        String::new()
    }

    /// Describe the action for the confirmation/preview step. Read-only.
    async fn preview(&self, transport: &dyn Transport) -> Result<ActionPreview>;

    /// Perform the action. The engine guarantees that mode, permission and
    /// confirmation checks have already passed before this is called.
    async fn execute(&self, transport: &dyn Transport) -> Result<ActionOutcome>;
}
