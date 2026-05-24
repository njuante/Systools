//! Database exposure findings built from the v0.7 discovery snapshot.

use systui_collectors::{BindScope, DatabaseEngine, DatabaseInstance, DatabaseSnapshot};
use systui_core::{Finding, ModuleId, Severity};

/// Convert detected database exposure into prioritized findings.
pub fn database_findings(snapshot: &DatabaseSnapshot) -> Vec<Finding> {
    let mut findings = Vec::new();
    for instance in &snapshot.instances {
        if instance.is_externally_reachable() {
            findings.push(exposure_finding(instance));
            if instance.engine == DatabaseEngine::Redis && instance.credential_sources.is_empty() {
                findings.push(redis_auth_finding(instance));
            }
        }

        if let Some(finding) = blocked_work_finding(instance) {
            findings.push(finding);
        }
        if let Some(finding) = replication_finding(instance) {
            findings.push(finding);
        }
        if let Some(finding) = recent_errors_finding(instance) {
            findings.push(finding);
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

fn blocked_work_finding(instance: &DatabaseInstance) -> Option<Finding> {
    let summary = instance.operational.lock_summary.as_deref()?;
    let count = first_number(summary)?;
    if count == 0 {
        return None;
    }
    Some(
        Finding::new(
            format!("db.blocked.{}", instance.engine.id()),
            Severity::Medium,
            ModuleId::Databases,
            format!("{} has blocked work", instance.engine.label()),
        )
        .evidence(evidence(instance))
        .impact(
            "Blocked clients, queued operations or waiting locks can delay application traffic.",
        )
        .recommendation(
            "Inspect the database workload and resolve the blocking query or operation.",
        ),
    )
}

fn replication_finding(instance: &DatabaseInstance) -> Option<Finding> {
    let summary = instance.operational.replication_summary.as_deref()?;
    let lower = summary.to_lowercase();
    let broken = lower.contains("down")
        || lower.contains("no")
        || lower.contains("fail")
        || lower.contains("broken")
        || lower.contains("master link err");
    if !broken {
        return None;
    }
    Some(
        Finding::new(
            format!("db.replication.{}", instance.engine.id()),
            Severity::High,
            ModuleId::Databases,
            format!("{} replication may be unhealthy", instance.engine.label()),
        )
        .evidence(evidence(instance))
        .impact(
            "A broken replica can increase recovery time and cause stale reads or failover risk.",
        )
        .recommendation(
            "Check replication status with the native database CLI and restore the replica link.",
        ),
    )
}

fn recent_errors_finding(instance: &DatabaseInstance) -> Option<Finding> {
    if instance.operational.recent_errors.is_empty() {
        return None;
    }
    Some(
        Finding::new(
            format!("db.recent-errors.{}", instance.engine.id()),
            Severity::Medium,
            ModuleId::Databases,
            format!("{} has recent error logs", instance.engine.label()),
        )
        .evidence(evidence(instance))
        .impact("Recent database errors can indicate failed persistence, authentication issues or application impact.")
        .recommendation("Review the database unit logs and correlate the errors with application symptoms."),
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
    if !instance.credential_sources.is_empty() {
        lines.push(format!(
            "credential sources: {}",
            instance
                .credential_sources
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(connections) = &instance.operational.connection_summary {
        lines.push(format!("connections {connections}"));
    }
    if let Some(locks) = &instance.operational.lock_summary {
        lines.push(format!("locks {locks}"));
    }
    if let Some(replication) = &instance.operational.replication_summary {
        lines.push(format!("replication {replication}"));
    }
    if let Some(error) = instance.operational.recent_errors.first() {
        lines.push(format!(
            "recent error {} {}",
            error.priority_label(),
            error.message
        ));
    }
    lines.extend(instance.detected_by.iter().cloned());
    lines
}

fn first_number(s: &str) -> Option<u64> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{
        DatabaseCredentialKind, DatabaseCredentialSource, DatabaseEngine, DatabaseInstance,
        DatabaseOperational, DatabaseSnapshot, Listener, LogEntry, ProcessRef, Protocol,
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
            credential_sources: Vec::new(),
            operational: Default::default(),
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

    #[test]
    fn redis_auth_risk_is_suppressed_when_source_exists() {
        let mut redis = instance(DatabaseEngine::Redis, 6379);
        redis.credential_sources = vec![DatabaseCredentialSource {
            kind: DatabaseCredentialKind::Environment,
            label: "REDISCLI_AUTH environment variable (value redacted)".to_owned(),
        }];

        let findings = database_findings(&DatabaseSnapshot {
            instances: vec![redis],
        });
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"db.exposed.redis.6379"));
        assert!(!ids.contains(&"db.redis.auth-unknown"));
    }

    #[test]
    fn flags_operational_database_health_signals() {
        let mut pg = instance(DatabaseEngine::PostgreSql, 5432);
        pg.listener.as_mut().unwrap().local_ip = "127.0.0.1".to_owned();
        pg.exposure = Some(BindScope::Loopback);
        pg.operational = DatabaseOperational {
            lock_summary: Some("2 waiting locks".to_owned()),
            replication_summary: Some("replica IO Yes, SQL No".to_owned()),
            recent_errors: vec![LogEntry {
                time: "09:00:00".to_owned(),
                priority: 3,
                identifier: "postgres".to_owned(),
                message: "could not write to log file".to_owned(),
            }],
            ..Default::default()
        };

        let findings = database_findings(&DatabaseSnapshot {
            instances: vec![pg],
        });
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"db.blocked.postgresql"));
        assert!(ids.contains(&"db.replication.postgresql"));
        assert!(ids.contains(&"db.recent-errors.postgresql"));
    }
}
