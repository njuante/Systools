//! Exposure map: classify listening sockets by bind scope (loopback vs.
//! externally reachable) and port sensitivity into a risk-ranked list with
//! human-readable evidence. This is the v0.3 differentiator (`Product.md` §4.6):
//! it answers "what is exposed on this host, by which process, and what should
//! I worry about first?".
//!
//! Classification is a pure function over the [`Listener`]s collected in S3.2/3.3,
//! so it is fully fixture-testable. The richer security `Finding` model (S3.6)
//! reuses [`Severity`]; the exposure map feeds it.

use serde::{Deserialize, Serialize};
use systui_core::Severity;

use crate::network::{Listener, Protocol};

/// Whether a socket binds the loopback interface only, or an externally
/// reachable address (a wildcard `0.0.0.0`/`::` or any non-loopback IP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindScope {
    Loopback,
    External,
}

/// A listener classified for exposure risk, carrying the evidence behind the
/// verdict so the UI and reports can explain *why*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureEntry {
    pub listener: Listener,
    pub scope: BindScope,
    /// The well-known sensitive service on this port, if any (e.g. `"redis"`),
    /// regardless of bind scope.
    pub sensitive_service: Option<&'static str>,
    pub severity: Severity,
    pub evidence: String,
}

/// Well-known sensitive ports and the severity an *external* exposure earns.
/// Unauthenticated-by-default datastores are `Critical`; authenticated remote
/// access services are `High`.
const SENSITIVE_PORTS: &[(u16, &str, Severity)] = &[
    (22, "ssh", Severity::High),
    (3306, "mysql", Severity::High),
    (5432, "postgresql", Severity::High),
    (6379, "redis", Severity::Critical),
    (9200, "elasticsearch", Severity::Critical),
    (11211, "memcached", Severity::Critical),
    (27017, "mongodb", Severity::Critical),
    (5984, "couchdb", Severity::Critical),
];

fn sensitive_entry(port: u16) -> Option<(&'static str, Severity)> {
    SENSITIVE_PORTS
        .iter()
        .find(|(p, _, _)| *p == port)
        .map(|(_, name, sev)| (*name, *sev))
}

fn bind_scope(ip: &str) -> BindScope {
    if ip == "::1" || ip.starts_with("127.") {
        BindScope::Loopback
    } else {
        BindScope::External
    }
}

/// Classify listeners into a risk-ranked exposure map, worst severity first
/// (ties broken by port for a stable order).
pub fn exposure_map(listeners: &[Listener]) -> Vec<ExposureEntry> {
    let mut entries: Vec<ExposureEntry> = listeners.iter().cloned().map(classify).collect();
    entries.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.listener.port.cmp(&b.listener.port))
    });
    entries
}

fn classify(listener: Listener) -> ExposureEntry {
    let scope = bind_scope(&listener.local_ip);
    let sensitive = sensitive_entry(listener.port);
    let sensitive_service = sensitive.map(|(name, _)| name);

    let severity = match scope {
        BindScope::Loopback => Severity::Info,
        BindScope::External => match sensitive {
            Some((_, sev)) => sev,
            None => Severity::Low,
        },
    };

    let evidence = build_evidence(&listener, scope, sensitive_service);
    ExposureEntry {
        listener,
        scope,
        sensitive_service,
        severity,
        evidence,
    }
}

fn build_evidence(
    listener: &Listener,
    scope: BindScope,
    sensitive_service: Option<&'static str>,
) -> String {
    let proto = match listener.protocol {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    };
    let mut text = match (scope, sensitive_service) {
        (BindScope::Loopback, _) => format!(
            "{proto} {}:{} bound to loopback; not reachable from other hosts",
            listener.local_ip, listener.port
        ),
        (BindScope::External, Some(service)) => format!(
            "{proto} {}:{} exposes {service} on a non-loopback address, reachable from other hosts",
            listener.local_ip, listener.port
        ),
        (BindScope::External, None) => format!(
            "{proto} {}:{} bound to a non-loopback address, reachable from other hosts",
            listener.local_ip, listener.port
        ),
    };
    match (&listener.process, &listener.unit) {
        (Some(process), Some(unit)) => {
            text.push_str(&format!(
                " (process {} pid {}, unit {unit})",
                process.name, process.pid
            ));
        }
        (Some(process), None) => {
            text.push_str(&format!(" (process {} pid {})", process.name, process.pid));
        }
        (None, _) => text.push_str(" (no owning process identified)"),
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::parse_ss_listeners;

    fn fixture_entries() -> Vec<ExposureEntry> {
        let listeners = parse_ss_listeners(include_str!("../fixtures/ss-tulpn.txt"));
        exposure_map(&listeners)
    }

    #[test]
    fn ranks_critical_and_high_first() {
        let entries = fixture_entries();
        // Worst-first: redis (Critical), then ssh (High), then the Low/Info tail.
        assert_eq!(entries[0].listener.port, 6379);
        assert_eq!(entries[0].severity, Severity::Critical);
        assert_eq!(entries[0].sensitive_service, Some("redis"));
        assert_eq!(entries[0].scope, BindScope::External);

        assert_eq!(entries[1].listener.port, 22);
        assert_eq!(entries[1].severity, Severity::High);
        assert_eq!(entries[1].sensitive_service, Some("ssh"));
    }

    #[test]
    fn loopback_sensitive_port_is_only_info() {
        let entries = fixture_entries();
        let pg = entries.iter().find(|e| e.listener.port == 5432).unwrap();
        assert_eq!(pg.scope, BindScope::Loopback);
        assert_eq!(pg.severity, Severity::Info);
        // The port is still recognised as sensitive, just not exposed.
        assert_eq!(pg.sensitive_service, Some("postgresql"));
        assert!(pg.evidence.contains("loopback"));
    }

    #[test]
    fn external_non_sensitive_is_low() {
        let entries = fixture_entries();
        let https = entries.iter().find(|e| e.listener.port == 443).unwrap();
        assert_eq!(https.scope, BindScope::External);
        assert_eq!(https.severity, Severity::Low);
        assert_eq!(https.sensitive_service, None);
    }

    #[test]
    fn evidence_notes_missing_process() {
        let entries = fixture_entries();
        let rpc = entries.iter().find(|e| e.listener.port == 111).unwrap();
        assert!(rpc.evidence.contains("no owning process identified"));
    }

    #[test]
    fn wildcard_ipv6_is_external() {
        assert_eq!(bind_scope("::"), BindScope::External);
        assert_eq!(bind_scope("0.0.0.0"), BindScope::External);
        assert_eq!(bind_scope("::1"), BindScope::Loopback);
        assert_eq!(bind_scope("127.0.0.1"), BindScope::Loopback);
        assert_eq!(bind_scope("192.168.1.10"), BindScope::External);
    }
}
