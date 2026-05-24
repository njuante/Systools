//! Execution modes and risk classification.
//!
//! SysTUI is *secure by default*: the default [`ExecutionMode`] is
//! [`ExecutionMode::ReadOnly`].

use std::fmt;

use serde::{Deserialize, Serialize};

/// How much SysTUI is allowed to do against a host.
///
/// See `Product.md` §3. The mode gates every action through the action engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    /// Inspect only. Never modifies anything.
    #[default]
    ReadOnly,
    /// Allows reversible, low-risk actions (export, list, ping, ...).
    SafeActions,
    /// Allows privileged actions; each still requires explicit confirmation.
    Privileged,
}

impl ExecutionMode {
    /// Whether an action of the given [`RiskLevel`] is permitted in this mode.
    ///
    /// Confirmation is handled separately by the action engine; this only
    /// answers "is it allowed at all".
    pub fn allows(self, risk: RiskLevel) -> bool {
        match self {
            Self::ReadOnly => risk == RiskLevel::None,
            Self::SafeActions => risk <= RiskLevel::Low,
            Self::Privileged => true,
        }
    }

    /// `true` only for [`ExecutionMode::ReadOnly`].
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::ReadOnly)
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ReadOnly => "read-only",
            Self::SafeActions => "safe-actions",
            Self::Privileged => "privileged",
        };
        f.write_str(s)
    }
}

/// Risk of an action, used to decide whether a mode permits it and how strong a
/// confirmation to require.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum RiskLevel {
    /// Pure inspection; no side effects.
    #[default]
    None,
    /// Reversible and low impact.
    Low,
    /// Service-affecting but recoverable (restart/reload).
    Medium,
    /// Potential downtime or data exposure (stop, kill, firewall change).
    High,
    /// Likely destructive or hard to reverse (delete, reboot, wipe logs).
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_is_the_default() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::ReadOnly);
        assert!(ExecutionMode::default().is_read_only());
    }

    #[test]
    fn read_only_allows_only_inspection() {
        let m = ExecutionMode::ReadOnly;
        assert!(m.allows(RiskLevel::None));
        assert!(!m.allows(RiskLevel::Low));
        assert!(!m.allows(RiskLevel::Critical));
    }

    #[test]
    fn safe_actions_allows_up_to_low() {
        let m = ExecutionMode::SafeActions;
        assert!(m.allows(RiskLevel::None));
        assert!(m.allows(RiskLevel::Low));
        assert!(!m.allows(RiskLevel::Medium));
    }

    #[test]
    fn privileged_allows_everything() {
        let m = ExecutionMode::Privileged;
        assert!(m.allows(RiskLevel::Critical));
    }

    #[test]
    fn risk_levels_are_ordered() {
        assert!(RiskLevel::None < RiskLevel::Low);
        assert!(RiskLevel::High < RiskLevel::Critical);
    }
}
