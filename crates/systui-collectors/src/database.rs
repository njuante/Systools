//! Database discovery: detect common database engines from systemd services,
//! listening sockets and optional local CLIs. This is intentionally not a SQL
//! client; it is the credential-free discovery layer for v0.7.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

use crate::exposure::BindScope;
use crate::network::{Listener, NetworkCollector, ProcessRef};
use crate::service::{ServiceCollector, ServiceUnit};

/// Supported database engines in v0.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseEngine {
    PostgreSql,
    Redis,
    MySql,
    MongoDb,
}

impl DatabaseEngine {
    pub fn id(self) -> &'static str {
        match self {
            Self::PostgreSql => "postgresql",
            Self::Redis => "redis",
            Self::MySql => "mysql",
            Self::MongoDb => "mongodb",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PostgreSql => "PostgreSQL",
            Self::Redis => "Redis",
            Self::MySql => "MySQL/MariaDB",
            Self::MongoDb => "MongoDB",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::PostgreSql => 5432,
            Self::Redis => 6379,
            Self::MySql => 3306,
            Self::MongoDb => 27017,
        }
    }

    fn version_probes(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Self::PostgreSql => &[("postgres", &["--version"]), ("psql", &["--version"])],
            Self::Redis => &[
                ("redis-server", &["--version"]),
                ("redis-cli", &["--version"]),
            ],
            Self::MySql => &[
                ("mysqld", &["--version"]),
                ("mariadbd", &["--version"]),
                ("mysql", &["--version"]),
                ("mariadb", &["--version"]),
            ],
            Self::MongoDb => &[
                ("mongod", &["--version"]),
                ("mongosh", &["--version"]),
                ("mongo", &["--version"]),
            ],
        }
    }
}

/// The systemd service evidence associated with a detected engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseService {
    pub unit: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

impl From<&ServiceUnit> for DatabaseService {
    fn from(unit: &ServiceUnit) -> Self {
        Self {
            unit: unit.name.clone(),
            active: unit.active.clone(),
            sub: unit.sub.clone(),
            description: unit.description.clone(),
        }
    }
}

/// One detected database endpoint or service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseInstance {
    pub engine: DatabaseEngine,
    pub service: Option<DatabaseService>,
    pub listener: Option<Listener>,
    pub version: Option<String>,
    pub exposure: Option<BindScope>,
    pub detected_by: Vec<String>,
}

impl DatabaseInstance {
    pub fn endpoint(&self) -> Option<String> {
        self.listener
            .as_ref()
            .map(|l| format!("{}:{}", l.local_ip, l.port))
    }

    pub fn process(&self) -> Option<&ProcessRef> {
        self.listener.as_ref().and_then(|l| l.process.as_ref())
    }

    pub fn is_externally_reachable(&self) -> bool {
        self.exposure == Some(BindScope::External)
    }
}

/// Point-in-time database discovery snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    pub instances: Vec<DatabaseInstance>,
}

/// Reads database discovery data from the host.
#[derive(Debug, Default, Clone, Copy)]
pub struct DatabaseCollector;

impl DatabaseCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for DatabaseCollector {
    type Output = DatabaseSnapshot;

    fn module(&self) -> ModuleId {
        ModuleId::Databases
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<DatabaseSnapshot> {
        let services = ServiceCollector::new()
            .collect(transport)
            .await
            .unwrap_or_default();
        let listeners = NetworkCollector::new()
            .collect(transport)
            .await
            .map(|net| net.listeners)
            .unwrap_or_default();
        let mut snapshot = detect_database_instances(&services, &listeners, &BTreeMap::new());
        let engines = snapshot
            .instances
            .iter()
            .map(|i| i.engine)
            .collect::<BTreeSet<_>>();
        let versions = collect_versions(transport, &engines).await;
        apply_versions(&mut snapshot, &versions);
        Ok(snapshot)
    }
}

/// Pure detection over already-collected services/listeners.
pub fn detect_database_instances(
    services: &[ServiceUnit],
    listeners: &[Listener],
    versions: &BTreeMap<DatabaseEngine, String>,
) -> DatabaseSnapshot {
    let service_matches = match_services(services);
    let mut used_services = BTreeSet::new();
    let mut instances = Vec::new();

    for listener in listeners {
        let Some(engine) = engine_for_listener(listener) else {
            continue;
        };
        let service = service_for_listener(listener, engine, &service_matches).or_else(|| {
            service_matches
                .get(&engine)
                .and_then(|v| v.first().copied())
        });
        if let Some(service) = service {
            used_services.insert(service.name.clone());
        }

        let mut detected_by = vec![format!("listener {}:{}", listener.local_ip, listener.port)];
        if listener.port == engine.default_port() {
            detected_by.push(format!("default port {}", engine.default_port()));
        }
        if let Some(process) = &listener.process {
            detected_by.push(format!("process {} pid {}", process.name, process.pid));
        }
        if let Some(unit) = &listener.unit {
            detected_by.push(format!("unit {unit}"));
        }

        instances.push(DatabaseInstance {
            engine,
            service: service.map(DatabaseService::from),
            listener: Some(listener.clone()),
            version: versions.get(&engine).cloned(),
            exposure: Some(bind_scope(&listener.local_ip)),
            detected_by,
        });
    }

    for (engine, units) in service_matches {
        for unit in units {
            if used_services.contains(&unit.name) {
                continue;
            }
            instances.push(DatabaseInstance {
                engine,
                service: Some(DatabaseService::from(unit)),
                listener: None,
                version: versions.get(&engine).cloned(),
                exposure: None,
                detected_by: vec![format!("systemd unit {}", unit.name)],
            });
        }
    }

    instances.sort_by(|a, b| {
        a.engine
            .cmp(&b.engine)
            .then_with(|| a.endpoint().cmp(&b.endpoint()))
            .then_with(|| {
                a.service
                    .as_ref()
                    .map(|s| &s.unit)
                    .cmp(&b.service.as_ref().map(|s| &s.unit))
            })
    });
    DatabaseSnapshot { instances }
}

fn match_services(services: &[ServiceUnit]) -> BTreeMap<DatabaseEngine, Vec<&ServiceUnit>> {
    let mut matches: BTreeMap<DatabaseEngine, Vec<&ServiceUnit>> = BTreeMap::new();
    for service in services {
        for engine in [
            DatabaseEngine::PostgreSql,
            DatabaseEngine::Redis,
            DatabaseEngine::MySql,
            DatabaseEngine::MongoDb,
        ] {
            if service_matches_engine(service, engine) {
                matches.entry(engine).or_default().push(service);
            }
        }
    }
    matches
}

fn service_matches_engine(service: &ServiceUnit, engine: DatabaseEngine) -> bool {
    let text = format!("{} {}", service.name, service.description).to_lowercase();
    match engine {
        DatabaseEngine::PostgreSql => text.contains("postgresql") || text.contains("postgres"),
        DatabaseEngine::Redis => text.contains("redis"),
        DatabaseEngine::MySql => {
            text.contains("mysql") || text.contains("mariadb") || text.contains("mysqld")
        }
        DatabaseEngine::MongoDb => text.contains("mongodb") || text.contains("mongod"),
    }
}

fn service_for_listener<'a>(
    listener: &Listener,
    engine: DatabaseEngine,
    services: &'a BTreeMap<DatabaseEngine, Vec<&'a ServiceUnit>>,
) -> Option<&'a ServiceUnit> {
    let unit = listener.unit.as_deref()?;
    services
        .get(&engine)?
        .iter()
        .copied()
        .find(|service| service.name == unit)
}

fn engine_for_listener(listener: &Listener) -> Option<DatabaseEngine> {
    if let Some(unit) = &listener.unit
        && let Some(engine) = engine_from_text(unit)
    {
        return Some(engine);
    }
    if let Some(process) = &listener.process
        && let Some(engine) = engine_from_text(&process.name)
    {
        return Some(engine);
    }
    [
        DatabaseEngine::PostgreSql,
        DatabaseEngine::Redis,
        DatabaseEngine::MySql,
        DatabaseEngine::MongoDb,
    ]
    .into_iter()
    .find(|engine| listener.port == engine.default_port())
}

fn engine_from_text(text: &str) -> Option<DatabaseEngine> {
    let text = text.to_lowercase();
    if text.contains("postgres") {
        Some(DatabaseEngine::PostgreSql)
    } else if text.contains("redis") {
        Some(DatabaseEngine::Redis)
    } else if text.contains("mysql") || text.contains("mariadb") || text.contains("mysqld") {
        Some(DatabaseEngine::MySql)
    } else if text.contains("mongo") {
        Some(DatabaseEngine::MongoDb)
    } else {
        None
    }
}

fn bind_scope(ip: &str) -> BindScope {
    if ip == "::1" || ip.starts_with("127.") {
        BindScope::Loopback
    } else {
        BindScope::External
    }
}

async fn collect_versions(
    transport: &dyn Transport,
    engines: &BTreeSet<DatabaseEngine>,
) -> BTreeMap<DatabaseEngine, String> {
    let mut versions = BTreeMap::new();
    for engine in engines {
        if let Some(version) = collect_version(transport, *engine).await {
            versions.insert(*engine, version);
        }
    }
    versions
}

async fn collect_version(transport: &dyn Transport, engine: DatabaseEngine) -> Option<String> {
    for (program, args) in engine.version_probes() {
        let spec = CommandSpec::new(*program).args(args.iter().copied());
        let Ok(output) = transport.run(&spec).await else {
            continue;
        };
        if output.success()
            && let Some(version) = normalize_version(&output.stdout)
        {
            return Some(version);
        }
    }
    None
}

fn apply_versions(snapshot: &mut DatabaseSnapshot, versions: &BTreeMap<DatabaseEngine, String>) {
    for instance in &mut snapshot.instances {
        instance.version = versions.get(&instance.engine).cloned();
    }
}

fn normalize_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::parse_ss_listeners;
    use systui_core::Collector;
    use systui_transport::MockTransport;

    fn services() -> Vec<ServiceUnit> {
        vec![
            ServiceUnit {
                name: "postgresql.service".to_owned(),
                load: "loaded".to_owned(),
                active: "active".to_owned(),
                sub: "exited".to_owned(),
                description: "PostgreSQL RDBMS".to_owned(),
            },
            ServiceUnit {
                name: "redis-server.service".to_owned(),
                load: "loaded".to_owned(),
                active: "active".to_owned(),
                sub: "running".to_owned(),
                description: "Advanced key-value store".to_owned(),
            },
            ServiceUnit {
                name: "mariadb.service".to_owned(),
                load: "loaded".to_owned(),
                active: "inactive".to_owned(),
                sub: "dead".to_owned(),
                description: "MariaDB database server".to_owned(),
            },
        ]
    }

    #[test]
    fn detects_database_instances_from_services_and_listeners() {
        let listeners = parse_ss_listeners(include_str!("../fixtures/ss-databases.txt"));
        let mut versions = BTreeMap::new();
        versions.insert(DatabaseEngine::Redis, "Redis server v=7.0.15".to_owned());

        let snapshot = detect_database_instances(&services(), &listeners, &versions);

        assert_eq!(snapshot.instances.len(), 4);
        let redis = snapshot
            .instances
            .iter()
            .find(|i| i.engine == DatabaseEngine::Redis)
            .unwrap();
        assert_eq!(redis.endpoint().as_deref(), Some("0.0.0.0:6379"));
        assert_eq!(redis.exposure, Some(BindScope::External));
        assert_eq!(redis.service.as_ref().unwrap().unit, "redis-server.service");
        assert_eq!(redis.version.as_deref(), Some("Redis server v=7.0.15"));

        let pg = snapshot
            .instances
            .iter()
            .find(|i| i.engine == DatabaseEngine::PostgreSql)
            .unwrap();
        assert_eq!(pg.exposure, Some(BindScope::Loopback));

        let mariadb = snapshot
            .instances
            .iter()
            .find(|i| i.engine == DatabaseEngine::MySql)
            .unwrap();
        assert!(mariadb.listener.is_none());
        assert_eq!(mariadb.service.as_ref().unwrap().active, "inactive");
    }

    #[test]
    fn text_matching_identifies_engine_names() {
        assert_eq!(
            engine_from_text("postgresql@15-main.service"),
            Some(DatabaseEngine::PostgreSql)
        );
        assert_eq!(engine_from_text("mariadbd"), Some(DatabaseEngine::MySql));
        assert_eq!(engine_from_text("mongod"), Some(DatabaseEngine::MongoDb));
        assert_eq!(engine_from_text("nginx"), None);
    }

    #[tokio::test]
    async fn collector_degrades_and_collects_versions() {
        let transport = MockTransport::new()
            .with_stdout(
                "systemctl list-units --type=service --all --no-legend --plain --no-pager",
                include_str!("../fixtures/systemctl-db-list-units.txt"),
            )
            .with_stdout("ss -tulpn", include_str!("../fixtures/ss-databases.txt"))
            .with_stdout(
                "redis-server --version",
                "Redis server v=7.0.15 sha=000:0 malloc=libc",
            )
            .with_stdout("postgres --version", "postgres (PostgreSQL) 15.5");

        let snapshot = DatabaseCollector::new().collect(&transport).await.unwrap();
        assert_eq!(snapshot.instances.len(), 4);
        assert!(
            snapshot
                .instances
                .iter()
                .any(|i| i.version.as_deref() == Some("postgres (PostgreSQL) 15.5"))
        );
    }
}
