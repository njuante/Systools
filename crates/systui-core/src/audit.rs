//! Audit model: a record of every action SysTUI attempts (`Product.md` §3).
//!
//! Records are produced by the action engine and persisted by `systui-storage`.

use serde::{Deserialize, Serialize};

use crate::collector::ModuleId;

/// Outcome of an audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStatus {
    /// The action ran and achieved its goal.
    Success,
    /// The action ran but did not succeed.
    Failure,
    /// The action was blocked (mode, guardrail or failed confirmation).
    Rejected,
}

/// Who/where an action was attempted, supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditContext {
    pub host: String,
    pub user: String,
}

/// A single audit-log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// RFC3339 timestamp.
    pub timestamp: String,
    pub host: String,
    pub user: String,
    pub module: ModuleId,
    /// Human-readable action summary, e.g. `"Restart nginx.service"`.
    pub action: String,
    /// The action's target, e.g. `"nginx.service"` or `"PID 4410"`.
    pub target: String,
    pub status: AuditStatus,
    pub duration_ms: u64,
}
