//! Database discovery: detect common database engines from systemd services,
//! listening sockets and optional local CLIs. This is intentionally not a SQL
//! client; it is the credential-free discovery layer for v0.7.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

use crate::exposure::BindScope;
use crate::logs::{LogEntry, LogQuery, LogsCollector};
use crate::network::{Connection, Listener, NetworkCollector, NetworkSnapshot, ProcessRef};
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
    pub operational: DatabaseOperational,
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

/// Best-effort operational signals for one database instance. Every field is
/// optional because credentials, CLIs and permissions vary widely by engine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseOperational {
    pub connection_summary: Option<String>,
    pub size_summary: Option<String>,
    pub replication_summary: Option<String>,
    pub lock_summary: Option<String>,
    pub recent_errors: Vec<LogEntry>,
    pub notes: Vec<String>,
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
        let network = NetworkCollector::new().collect(transport).await.ok();
        let listeners = network
            .as_ref()
            .map(|net| net.listeners.as_slice())
            .unwrap_or(&[]);
        let mut snapshot = detect_database_instances(&services, listeners, &BTreeMap::new());
        let engines = snapshot
            .instances
            .iter()
            .map(|i| i.engine)
            .collect::<BTreeSet<_>>();
        let versions = collect_versions(transport, &engines).await;
        apply_versions(&mut snapshot, &versions);
        enrich_operational(transport, &mut snapshot, network.as_ref()).await;
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
            operational: DatabaseOperational::default(),
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
                operational: DatabaseOperational::default(),
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

async fn enrich_operational(
    transport: &dyn Transport,
    snapshot: &mut DatabaseSnapshot,
    network: Option<&NetworkSnapshot>,
) {
    for instance in &mut snapshot.instances {
        if let Some(network) = network
            && let Some(summary) = connection_summary(instance, &network.connections)
        {
            instance.operational.connection_summary = Some(summary);
        }
        instance.operational.recent_errors = collect_recent_errors(transport, instance).await;
        if instance.operational.recent_errors.is_empty() && instance.service.is_none() {
            instance
                .operational
                .notes
                .push("No systemd unit available for recent error lookup.".to_owned());
        }
        match instance.engine {
            DatabaseEngine::Redis => enrich_redis(transport, instance).await,
            DatabaseEngine::PostgreSql => enrich_postgres(transport, instance).await,
            DatabaseEngine::MySql => enrich_mysql(transport, instance).await,
            DatabaseEngine::MongoDb => enrich_mongo(transport, instance).await,
        }
    }
}

fn connection_summary(instance: &DatabaseInstance, connections: &[Connection]) -> Option<String> {
    let listener = instance.listener.as_ref()?;
    let mut states = BTreeMap::<String, usize>::new();
    for conn in connections
        .iter()
        .filter(|conn| conn.local_port == listener.port)
    {
        *states.entry(conn.state.clone()).or_insert(0) += 1;
    }
    if states.is_empty() {
        return Some("0 active TCP connections".to_owned());
    }
    let total: usize = states.values().sum();
    let parts = states
        .iter()
        .map(|(state, count)| format!("{state} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{total} active TCP connections ({parts})"))
}

async fn collect_recent_errors(
    transport: &dyn Transport,
    instance: &DatabaseInstance,
) -> Vec<LogEntry> {
    let Some(service) = &instance.service else {
        return Vec::new();
    };
    LogsCollector::with_query(LogQuery {
        min_priority: 3,
        unit: Some(service.unit.clone()),
        since: Some("24 hours ago".to_owned()),
        lines: 20,
    })
    .collect(transport)
    .await
    .unwrap_or_default()
}

async fn enrich_redis(transport: &dyn Transport, instance: &mut DatabaseInstance) {
    let Some((host, port)) = probe_host_port(instance) else {
        instance
            .operational
            .notes
            .push("Redis INFO skipped because no listener was detected.".to_owned());
        return;
    };
    let spec = CommandSpec::new("redis-cli")
        .args([
            "-h",
            host.as_str(),
            "-p",
            &port.to_string(),
            "--no-auth-warning",
            "INFO",
        ])
        .timeout(Duration::from_secs(3));
    match transport.run(&spec).await {
        Ok(out) if out.success() => apply_redis_info(instance, &parse_redis_info(&out.stdout)),
        Ok(out) => instance.operational.notes.push(format!(
            "redis-cli INFO unavailable: {}",
            trim_stderr(&out.stderr)
        )),
        Err(_) => instance
            .operational
            .notes
            .push("redis-cli unavailable or timed out.".to_owned()),
    }
}

async fn enrich_postgres(transport: &dyn Transport, instance: &mut DatabaseInstance) {
    let Some((host, port)) = probe_host_port(instance) else {
        instance
            .operational
            .notes
            .push("PostgreSQL stats skipped because no listener was detected.".to_owned());
        return;
    };
    let query = "select 'connections='||count(*) from pg_stat_activity union all select 'locks_waiting='||count(*) from pg_locks where not granted union all select 'replicas='||count(*) from pg_stat_replication union all select 'size='||pg_size_pretty(sum(pg_database_size(datname))) from pg_database";
    let spec = CommandSpec::new("psql")
        .args([
            "-w",
            "-AtX",
            "-h",
            host.as_str(),
            "-p",
            &port.to_string(),
            "-c",
            query,
        ])
        .timeout(Duration::from_secs(3));
    match transport.run(&spec).await {
        Ok(out) if out.success() => {
            apply_postgres_stats(instance, &parse_key_value_lines(&out.stdout))
        }
        Ok(out) => instance.operational.notes.push(format!(
            "psql stats unavailable: {}",
            trim_stderr(&out.stderr)
        )),
        Err(_) => instance
            .operational
            .notes
            .push("psql unavailable or timed out.".to_owned()),
    }
}

async fn enrich_mysql(transport: &dyn Transport, instance: &mut DatabaseInstance) {
    let Some((host, port)) = probe_host_port(instance) else {
        instance
            .operational
            .notes
            .push("MySQL stats skipped because no listener was detected.".to_owned());
        return;
    };
    let query = "SHOW GLOBAL STATUS WHERE Variable_name IN ('Threads_connected','Max_used_connections'); SELECT 'Database_size', CONCAT(ROUND(SUM(data_length+index_length)/1024/1024,1),' MiB') FROM information_schema.tables";
    let spec = CommandSpec::new("mysql")
        .args([
            "--batch",
            "--skip-column-names",
            "--connect-timeout=3",
            "-h",
            host.as_str(),
            "-P",
            &port.to_string(),
            "-e",
            query,
        ])
        .timeout(Duration::from_secs(3));
    match transport.run(&spec).await {
        Ok(out) if out.success() => apply_mysql_stats(instance, &parse_mysql_status(&out.stdout)),
        Ok(out) => instance.operational.notes.push(format!(
            "mysql stats unavailable: {}",
            trim_stderr(&out.stderr)
        )),
        Err(_) => instance
            .operational
            .notes
            .push("mysql unavailable or timed out.".to_owned()),
    }
}

async fn enrich_mongo(transport: &dyn Transport, instance: &mut DatabaseInstance) {
    let Some((host, port)) = probe_host_port(instance) else {
        instance
            .operational
            .notes
            .push("MongoDB serverStatus skipped because no listener was detected.".to_owned());
        return;
    };
    let spec = CommandSpec::new("mongosh")
        .args([
            "--quiet",
            "--host",
            host.as_str(),
            "--port",
            &port.to_string(),
            "--eval",
            "JSON.stringify(db.serverStatus())",
        ])
        .timeout(Duration::from_secs(3));
    match transport.run(&spec).await {
        Ok(out) if out.success() => apply_mongo_status(instance, &out.stdout),
        Ok(out) => instance.operational.notes.push(format!(
            "mongosh serverStatus unavailable: {}",
            trim_stderr(&out.stderr)
        )),
        Err(_) => instance
            .operational
            .notes
            .push("mongosh unavailable or timed out.".to_owned()),
    }
}

fn probe_host_port(instance: &DatabaseInstance) -> Option<(String, u16)> {
    let listener = instance.listener.as_ref()?;
    let host = match listener.local_ip.as_str() {
        "0.0.0.0" | "*" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    Some((host.to_owned(), listener.port))
}

fn trim_stderr(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        "command failed".to_owned()
    } else {
        trimmed.lines().next().unwrap_or(trimmed).to_owned()
    }
}

fn parse_redis_info(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            Some((key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn apply_redis_info(instance: &mut DatabaseInstance, info: &BTreeMap<String, String>) {
    if let Some(clients) = info.get("connected_clients") {
        instance.operational.connection_summary = Some(format!("{clients} connected clients"));
    }
    let mut size = Vec::new();
    if let Some(memory) = info.get("used_memory_human") {
        size.push(format!("{memory} memory"));
    }
    let keys = info
        .iter()
        .filter_map(|(key, value)| key.starts_with("db").then(|| redis_keys(value)).flatten())
        .sum::<u64>();
    if keys > 0 {
        size.push(format!("{keys} keys"));
    }
    if !size.is_empty() {
        instance.operational.size_summary = Some(size.join(", "));
    }
    if let Some(blocked) = info.get("blocked_clients") {
        instance.operational.lock_summary = Some(format!("{blocked} blocked clients"));
    }
    let role = info.get("role").map(String::as_str).unwrap_or("unknown");
    let repl = match role {
        "master" => info
            .get("connected_slaves")
            .map(|slaves| format!("master with {slaves} replicas")),
        "slave" => info
            .get("master_link_status")
            .map(|status| format!("replica, master link {status}")),
        _ => Some(format!("role {role}")),
    };
    instance.operational.replication_summary = repl;
}

fn redis_keys(value: &str) -> Option<u64> {
    value.split(',').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "keys").then(|| value.parse().ok()).flatten()
    })
}

fn parse_key_value_lines(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn apply_postgres_stats(instance: &mut DatabaseInstance, stats: &BTreeMap<String, String>) {
    if let Some(connections) = stats.get("connections") {
        instance.operational.connection_summary = Some(format!("{connections} SQL sessions"));
    }
    if let Some(size) = stats.get("size") {
        instance.operational.size_summary = Some(size.clone());
    }
    if let Some(replicas) = stats.get("replicas") {
        instance.operational.replication_summary = Some(format!("{replicas} streaming replicas"));
    }
    if let Some(locks) = stats.get("locks_waiting") {
        instance.operational.lock_summary = Some(format!("{locks} waiting locks"));
    }
}

fn parse_mysql_status(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let value = parts.collect::<Vec<_>>().join(" ");
            (!value.is_empty()).then(|| (key.to_owned(), value))
        })
        .collect()
}

fn apply_mysql_stats(instance: &mut DatabaseInstance, stats: &BTreeMap<String, String>) {
    if let Some(threads) = stats.get("Threads_connected") {
        let max = stats
            .get("Max_used_connections")
            .map(|m| format!("; max used {m}"))
            .unwrap_or_default();
        instance.operational.connection_summary = Some(format!("{threads} connected threads{max}"));
    }
    if let Some(size) = stats.get("Database_size") {
        instance.operational.size_summary = Some(size.clone());
    }
    let io = stats.get("Replica_IO_Running");
    let sql = stats.get("Replica_SQL_Running");
    if io.is_some() || sql.is_some() {
        instance.operational.replication_summary = Some(format!(
            "replica IO {}, SQL {}",
            io.map(String::as_str).unwrap_or("unknown"),
            sql.map(String::as_str).unwrap_or("unknown")
        ));
    }
}

fn apply_mongo_status(instance: &mut DatabaseInstance, stdout: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        instance
            .operational
            .notes
            .push("mongosh returned non-JSON serverStatus output.".to_owned());
        return;
    };
    if let Some(current) = value
        .pointer("/connections/current")
        .and_then(|v| v.as_u64())
    {
        instance.operational.connection_summary = Some(format!("{current} current connections"));
    }
    if let Some(resident) = value.pointer("/mem/resident").and_then(|v| v.as_u64()) {
        instance.operational.size_summary = Some(format!("{resident} MiB resident memory"));
    }
    if let Some(set) = value.pointer("/repl/setName").and_then(|v| v.as_str()) {
        let state = value
            .pointer("/repl/myState")
            .and_then(|v| v.as_i64())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        instance.operational.replication_summary =
            Some(format!("replica set {set}, state {state}"));
    }
    if let Some(queue) = value
        .pointer("/globalLock/currentQueue/total")
        .and_then(|v| v.as_u64())
    {
        instance.operational.lock_summary = Some(format!("{queue} operations queued"));
    }
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

    #[test]
    fn parses_redis_info_operational_signals() {
        let mut instance = empty_instance(DatabaseEngine::Redis);
        apply_redis_info(
            &mut instance,
            &parse_redis_info(include_str!("../fixtures/redis-info.txt")),
        );

        assert_eq!(
            instance.operational.connection_summary.as_deref(),
            Some("12 connected clients")
        );
        assert_eq!(
            instance.operational.size_summary.as_deref(),
            Some("10.40M memory, 1200 keys")
        );
        assert_eq!(
            instance.operational.lock_summary.as_deref(),
            Some("1 blocked clients")
        );
        assert_eq!(
            instance.operational.replication_summary.as_deref(),
            Some("master with 2 replicas")
        );
    }

    #[test]
    fn parses_sql_and_mongo_operational_signals() {
        let mut pg = empty_instance(DatabaseEngine::PostgreSql);
        apply_postgres_stats(
            &mut pg,
            &parse_key_value_lines(include_str!("../fixtures/psql-operational.txt")),
        );
        assert_eq!(
            pg.operational.connection_summary.as_deref(),
            Some("14 SQL sessions")
        );
        assert_eq!(pg.operational.size_summary.as_deref(), Some("42 MB"));
        assert_eq!(
            pg.operational.lock_summary.as_deref(),
            Some("2 waiting locks")
        );
        assert_eq!(
            pg.operational.replication_summary.as_deref(),
            Some("1 streaming replicas")
        );

        let mut mysql = empty_instance(DatabaseEngine::MySql);
        apply_mysql_stats(
            &mut mysql,
            &parse_mysql_status(include_str!("../fixtures/mysql-operational.txt")),
        );
        assert_eq!(
            mysql.operational.connection_summary.as_deref(),
            Some("8 connected threads; max used 24")
        );
        assert_eq!(mysql.operational.size_summary.as_deref(), Some("128.0 MiB"));
        assert_eq!(
            mysql.operational.replication_summary.as_deref(),
            Some("replica IO Yes, SQL No")
        );

        let mut mongo = empty_instance(DatabaseEngine::MongoDb);
        apply_mongo_status(
            &mut mongo,
            include_str!("../fixtures/mongosh-server-status.json"),
        );
        assert_eq!(
            mongo.operational.connection_summary.as_deref(),
            Some("17 current connections")
        );
        assert_eq!(
            mongo.operational.size_summary.as_deref(),
            Some("256 MiB resident memory")
        );
        assert_eq!(
            mongo.operational.lock_summary.as_deref(),
            Some("3 operations queued")
        );
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

    fn empty_instance(engine: DatabaseEngine) -> DatabaseInstance {
        DatabaseInstance {
            engine,
            service: None,
            listener: None,
            version: None,
            exposure: None,
            operational: DatabaseOperational::default(),
            detected_by: Vec::new(),
        }
    }
}
