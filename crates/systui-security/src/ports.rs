//! Network exposure findings: bridge the collectors' exposure map into security
//! findings. Externally reachable sensitive ports become high/critical findings;
//! externally reachable listeners with no identified process are flagged too.

use systui_collectors::{BindScope, ExposureEntry};
use systui_core::{Finding, ModuleId, Severity};

/// Turn the ranked exposure map into findings. Only externally reachable
/// sensitive ports (already classified `High`/`Critical`) and unattributed
/// external listeners are surfaced here; benign exposures stay in the map.
pub fn check_exposed_ports(entries: &[ExposureEntry]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for entry in entries {
        let port = entry.listener.port;
        if entry.severity >= Severity::High {
            let service = entry.sensitive_service.unwrap_or("service");
            findings.push(
                Finding::new(
                    format!("net.sensitive-port.{port}"),
                    entry.severity,
                    ModuleId::Network,
                    format!("Sensitive service exposed: {service} on port {port}"),
                )
                .with_evidence(entry.evidence.clone())
                .impact("The service is reachable from other hosts and is a high-value target.")
                .recommendation(
                    "Bind it to loopback, restrict it with a firewall, or require authentication.",
                ),
            );
        }

        if entry.scope == BindScope::External && entry.listener.process.is_none() {
            findings.push(
                Finding::new(
                    format!("net.unidentified-listener.{port}"),
                    Severity::Low,
                    ModuleId::Network,
                    format!("Externally reachable port {port} has no identified process"),
                )
                .with_evidence(entry.evidence.clone())
                .impact("An exposed listener that cannot be attributed may be unmonitored or malicious.")
                .recommendation("Re-run with privileges to attribute the socket, then verify it is expected."),
            );
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{Listener, ProcessRef, Protocol, exposure_map};

    fn listener(ip: &str, port: u16, process: Option<&str>) -> Listener {
        Listener {
            protocol: Protocol::Tcp,
            local_ip: ip.to_owned(),
            port,
            process: process.map(|name| ProcessRef {
                pid: 1,
                name: name.to_owned(),
            }),
            unit: None,
        }
    }

    #[test]
    fn surfaces_sensitive_ports_and_unidentified_listeners() {
        let listeners = vec![
            listener("0.0.0.0", 6379, Some("redis-server")),
            listener("0.0.0.0", 22, Some("sshd")),
            listener("127.0.0.1", 5432, Some("postgres")),
            listener("0.0.0.0", 111, None),
        ];
        let findings = check_exposed_ports(&exposure_map(&listeners));
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        // redis on 0.0.0.0:6379 is critical; ssh on 0.0.0.0:22 is high.
        assert!(ids.contains(&"net.sensitive-port.6379"));
        assert!(ids.contains(&"net.sensitive-port.22"));
        // The processless rpcbind listener on 0.0.0.0:111 is flagged.
        assert!(ids.contains(&"net.unidentified-listener.111"));
        // Loopback postgres (5432) is not surfaced as a sensitive-port finding.
        assert!(!ids.contains(&"net.sensitive-port.5432"));

        let redis = findings
            .iter()
            .find(|f| f.id == "net.sensitive-port.6379")
            .unwrap();
        assert_eq!(redis.severity, Severity::Critical);
    }
}
