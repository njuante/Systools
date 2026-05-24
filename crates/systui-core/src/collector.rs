//! The collector contract: read-only readers of host state.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::transport::Transport;

/// Identifies a functional module. Used to tag collectors, actions, findings
/// and audit entries so they can be correlated across the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ModuleId {
    Dashboard,
    System,
    Processes,
    Services,
    Logs,
    Network,
    Docker,
    Crons,
    Databases,
    Security,
    Firewall,
    Certificates,
    Backups,
    Packages,
    Fleet,
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Dashboard => "dashboard",
            Self::System => "system",
            Self::Processes => "processes",
            Self::Services => "services",
            Self::Logs => "logs",
            Self::Network => "network",
            Self::Docker => "docker",
            Self::Crons => "crons",
            Self::Databases => "databases",
            Self::Security => "security",
            Self::Firewall => "firewall",
            Self::Certificates => "certificates",
            Self::Backups => "backups",
            Self::Packages => "packages",
            Self::Fleet => "fleet",
        };
        f.write_str(s)
    }
}

/// A read-only reader of host state. Collectors must never mutate the host.
///
/// Missing permissions should surface as a typed error (e.g.
/// [`crate::CoreError::PermissionDenied`]) so the UI can show partial data
/// instead of crashing.
#[async_trait]
pub trait Collector: Send + Sync {
    /// The typed snapshot this collector produces.
    type Output;

    /// Which module this collector belongs to.
    fn module(&self) -> ModuleId;

    /// Read the current state through the given transport.
    async fn collect(&self, transport: &dyn Transport) -> Result<Self::Output>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_id_display_roundtrips_via_serde() {
        let id = ModuleId::Services;
        assert_eq!(id.to_string(), "services");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"services\"");
    }
}
