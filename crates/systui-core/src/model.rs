//! Shared domain types used across modules. Module-specific models live in the
//! crates that own them; only the genuinely shared ones live here.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable identifier for a host (the inventory key, e.g. `"prod-01"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HostId(pub String);

impl HostId {
    /// The identifier used for the current local machine.
    pub const LOCAL: &'static str = "local";

    /// Borrow the inner string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for HostId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for HostId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Severity shared by checks and findings. Ordered from least to most severe, so
/// sorting descending yields a "worst first" list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_worst_last() {
        let mut s = [
            Severity::High,
            Severity::Info,
            Severity::Critical,
            Severity::Low,
        ];
        s.sort();
        assert_eq!(
            s,
            [
                Severity::Info,
                Severity::Low,
                Severity::High,
                Severity::Critical
            ]
        );
    }

    #[test]
    fn host_id_from_str() {
        let h: HostId = "prod-01".into();
        assert_eq!(h.as_str(), "prod-01");
        assert_eq!(h.to_string(), "prod-01");
    }
}
