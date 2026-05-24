//! The shared `Finding` model: a prioritized, evidence-based security or
//! exposure issue (`Product.md` §9). Findings are produced by the security and
//! network modules and rendered in the Security panel and reports.
//!
//! A finding always carries the *evidence* behind it and a *recommendation*, so
//! the UI can explain "why" rather than asserting "X is insecure" (§7.4). The
//! `id` is a stable, dotted identifier (e.g. `ssh.password-auth`) so later
//! phases can attach policy exceptions to a finding without ambiguity.

use serde::{Deserialize, Serialize};

use crate::collector::ModuleId;
use crate::model::Severity;

/// Lifecycle state of a finding. The workflow (accept/ignore/exception) lands
/// in a later phase; for now everything is reported as [`FindingStatus::Open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingStatus {
    #[default]
    Open,
    Accepted,
    Ignored,
    Fixed,
    FalsePositive,
}

/// A single prioritized issue with the evidence and remediation behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable dotted identifier, e.g. `ssh.root-login` or `net.sensitive-port.6379`.
    pub id: String,
    pub severity: Severity,
    /// The module that produced the finding.
    pub module: ModuleId,
    pub title: String,
    /// Concrete proof, e.g. the offending config line or command output.
    pub evidence: Vec<String>,
    /// What an attacker or operator could do because of this.
    pub impact: String,
    /// The suggested remediation.
    pub recommendation: String,
    pub status: FindingStatus,
}

impl Finding {
    /// Start a finding. Evidence, impact and recommendation are filled via the
    /// builder methods; status defaults to [`FindingStatus::Open`].
    pub fn new(
        id: impl Into<String>,
        severity: Severity,
        module: ModuleId,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            severity,
            module,
            title: title.into(),
            evidence: Vec::new(),
            impact: String::new(),
            recommendation: String::new(),
            status: FindingStatus::default(),
        }
    }

    /// Append a single line of evidence.
    #[must_use]
    pub fn with_evidence(mut self, line: impl Into<String>) -> Self {
        self.evidence.push(line.into());
        self
    }

    /// Replace the evidence with the given lines.
    #[must_use]
    pub fn evidence<I, S>(mut self, lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.evidence = lines.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn impact(mut self, impact: impl Into<String>) -> Self {
        self.impact = impact.into();
        self
    }

    #[must_use]
    pub fn recommendation(mut self, recommendation: impl Into<String>) -> Self {
        self.recommendation = recommendation.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_finding_with_evidence() {
        let f = Finding::new(
            "ssh.password-auth",
            Severity::High,
            ModuleId::Security,
            "SSH password authentication is enabled",
        )
        .with_evidence("/etc/ssh/sshd_config: PasswordAuthentication yes")
        .impact("Allows brute-force attempts if SSH is exposed.")
        .recommendation("Disable password auth and use key-based login.");

        assert_eq!(f.status, FindingStatus::Open);
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.evidence.len(), 1);
        assert!(!f.recommendation.is_empty());
    }

    #[test]
    fn status_serializes_kebab_case() {
        let json = serde_json::to_string(&FindingStatus::FalsePositive).unwrap();
        assert_eq!(json, "\"false-positive\"");
    }
}
