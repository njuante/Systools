//! Database exposure findings built from the v0.7 discovery snapshot.

use systui_collectors::{BindScope, DatabaseEngine, DatabaseInstance, DatabaseSnapshot};
use systui_core::{Finding, ModuleId, Severity};

/// Convert detected database exposure into prioritized findings.
pub fn database_findings(snapshot: &DatabaseSnapshot) -> Vec<Finding> {
    let mut findings = Vec::new();
    for instance in &snapshot.instances {
        if !instance.is_externally_reachable() {
            continue;
        }
        findings.push(exposure_finding(instance));
        if instance.engine == DatabaseEngine::Redis {
            findings.push(redis_auth_finding(instance));
        }
    }
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}

fn exposure_finding(instance: &DatabaseInstance) -> Finding {
    let port = instance
        .listener
        .as_ref()
        .map_or(instance.engine.default_port(), |l| l.port);
    let severity = match instance.engine {
        DatabaseEngine::Redis | DatabaseEngine::MongoDb => Severity::Critical,
        DatabaseEngine::PostgreSql | DatabaseEngine::MySql => Severity::High,
    };
    Finding::new(
        format!("db.exposed.{}.{}", instance.engine.id(), port),
        severity,
        ModuleId::Databases,
        format!("{} is reachable on a non-loopback address", instance.engine.label()),
    )
    .evidence(evidence(instance))
    .impact("The database listener is reachable from other hosts and is a high-value target.")
    .recommendation(
        "Bind the database to loopback or a private interface, restrict access with a firewall, and require authentication.",
    )
}

fn redis_auth_finding(instance: &DatabaseInstance) -> Finding {
    Finding::new(
        "db.redis.auth-unknown",
        Severity::Critical,
        ModuleId::Databases,
        "Redis is exposed without authentication evidence",
    )
    .evidence(evidence(instance))
    .impact("Exposed Redis instances are frequently abused for data theft, persistence and remote code execution paths.")
    .recommendation(
        "Require Redis authentication, bind it to loopback/private networks, and block port 6379 at the firewall.",
    )
}

fn evidence(instance: &DatabaseInstance) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(endpoint) = instance.endpoint() {
        lines.push(format!(
            "listener {endpoint} ({:?})",
            instance.exposure.unwrap_or(BindScope::External)
        ));
    }
    if let Some(service) = &instance.service {
        lines.push(format!(
            "service {} active={} sub={}",
            service.unit, service.active, service.sub
        ));
    }
    if let Some(process) = instance.process() {
        lines.push(format!("process {} pid {}", process.name, process.pid));
    }
    if let Some(version) = &instance.version {
        lines.push(format!("version {version}"));
    }
    lines.extend(instance.detected_by.iter().cloned());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{
        DatabaseEngine, DatabaseInstance, DatabaseSnapshot, Listener, ProcessRef, Protocol,
    };

    fn instance(engine: DatabaseEngine, port: u16) -> DatabaseInstance {
        DatabaseInstance {
            engine,
            service: None,
            listener: Some(Listener {
                protocol: Protocol::Tcp,
                local_ip: "0.0.0.0".to_owned(),
                port,
                process: Some(ProcessRef {
                    pid: 42,
                    name: engine.id().to_owned(),
                }),
                unit: None,
            }),
            version: None,
            exposure: Some(BindScope::External),
            detected_by: vec![format!("default port {port}")],
        }
    }

    #[test]
    fn flags_external_database_listeners() {
        let snapshot = DatabaseSnapshot {
            instances: vec![
                instance(DatabaseEngine::PostgreSql, 5432),
                instance(DatabaseEngine::Redis, 6379),
            ],
        };
        let findings = database_findings(&snapshot);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();

        assert!(ids.contains(&"db.exposed.postgresql.5432"));
        assert!(ids.contains(&"db.exposed.redis.6379"));
        assert!(ids.contains(&"db.redis.auth-unknown"));

        let redis = findings
            .iter()
            .find(|f| f.id == "db.exposed.redis.6379")
            .unwrap();
        assert_eq!(redis.severity, Severity::Critical);
    }

    #[test]
    fn ignores_loopback_databases() {
        let mut pg = instance(DatabaseEngine::PostgreSql, 5432);
        pg.listener.as_mut().unwrap().local_ip = "127.0.0.1".to_owned();
        pg.exposure = Some(BindScope::Loopback);

        assert!(
            database_findings(&DatabaseSnapshot {
                instances: vec![pg]
            })
            .is_empty()
        );
    }
}
