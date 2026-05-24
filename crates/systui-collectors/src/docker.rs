//! Docker collectors: list containers, read a no-stream stats snapshot and an
//! inspect summary. All data comes from the `docker` CLI through a [`Transport`]
//! using machine-readable output (`{{json .}}` lines for `ps`/`stats`, the inspect
//! JSON array), so no shell parsing happens and the same code runs over SSH later.
//!
//! Parsers are pure functions covered by fixture tests. If `docker` is missing or
//! the daemon/socket is unreachable, the collector surfaces the transport error so
//! the UI can show an honest "Docker unavailable" message rather than "no
//! containers". The inspect summary feeds the v0.4 risk checks in `systui-security`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// Health reported for a container (parsed from `docker ps` status or inspect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerHealth {
    Healthy,
    Unhealthy,
    Starting,
}

impl ContainerHealth {
    fn from_status_text(status: &str) -> Option<Self> {
        let s = status.to_ascii_lowercase();
        if s.contains("(healthy)") {
            Some(Self::Healthy)
        } else if s.contains("(unhealthy)") {
            Some(Self::Unhealthy)
        } else if s.contains("health: starting") {
            Some(Self::Starting)
        } else {
            None
        }
    }

    /// Parse the bare status string from `docker inspect` (`State.Health.Status`).
    pub fn from_inspect(status: &str) -> Option<Self> {
        match status {
            "healthy" => Some(Self::Healthy),
            "unhealthy" => Some(Self::Unhealthy),
            "starting" => Some(Self::Starting),
            _ => None,
        }
    }
}

/// A container row from `docker ps -a`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// Lifecycle state: `running`, `exited`, `created`, `paused`, `restarting`, …
    pub state: String,
    /// Raw status text, e.g. `Up 2 days (healthy)`.
    pub status: String,
    pub health: Option<ContainerHealth>,
    /// Raw published-ports string as shown by `docker ps`.
    pub ports: String,
    pub created: String,
}

impl Container {
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }
}

/// A no-stream stats snapshot for one container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerStats {
    pub id: String,
    pub name: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub mem_usage: String,
}

/// A bind/volume mount on a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub source: String,
    pub destination: String,
    pub rw: bool,
}

/// A published (host-mapped) port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPort {
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

/// The fields of `docker inspect` that the UI and risk checks need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectSummary {
    pub id: String,
    pub name: String,
    pub image: String,
    pub privileged: bool,
    pub restart_policy: String,
    pub max_retry_count: u32,
    pub restart_count: u32,
    /// Memory limit in bytes; `0` means unlimited.
    pub memory_limit_bytes: u64,
    pub networks: Vec<String>,
    pub mounts: Vec<Mount>,
    pub published_ports: Vec<PublishedPort>,
    pub health: Option<ContainerHealth>,
}

impl InspectSummary {
    /// Whether the image reference uses the floating `latest` tag (or no tag).
    pub fn uses_latest_tag(&self) -> bool {
        match self.image.rsplit_once(':') {
            // A ':' in the digest/registry-port is not a tag; treat `repo@sha` and
            // untagged refs as latest.
            Some((_, tag)) => tag == "latest" || tag.contains('/'),
            None => true,
        }
    }
}

/// Lists all containers (running and stopped) via `docker ps -a`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DockerCollector;

impl DockerCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for DockerCollector {
    type Output = Vec<Container>;

    fn module(&self) -> ModuleId {
        ModuleId::Docker
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<Container>> {
        let spec =
            CommandSpec::new("docker").args(["ps", "-a", "--no-trunc", "--format", "{{json .}}"]);
        let output = transport.run(&spec).await?.into_result("docker")?;
        Ok(parse_ps(&output.stdout))
    }
}

/// Read a no-stream stats snapshot for all running containers.
pub async fn container_stats(transport: &dyn Transport) -> Result<Vec<ContainerStats>> {
    let spec = CommandSpec::new("docker").args(["stats", "--no-stream", "--format", "{{json .}}"]);
    let output = transport.run(&spec).await?.into_result("docker")?;
    Ok(parse_stats(&output.stdout))
}

/// Read the inspect summary for one container.
pub async fn inspect_container(
    transport: &dyn Transport,
    id: &str,
) -> Result<Option<InspectSummary>> {
    let spec = CommandSpec::new("docker").args(["inspect", id]);
    let output = transport.run(&spec).await?.into_result("docker")?;
    Ok(parse_inspect(&output.stdout))
}

/// Read the last `tail` log lines for a container (read-only snapshot, not a
/// live stream). Docker writes container stdout and stderr to the command's
/// stdout and stderr respectively; both are returned, stdout lines first.
pub async fn container_logs(transport: &dyn Transport, id: &str, tail: u32) -> Result<Vec<String>> {
    let tail = tail.to_string();
    let spec = CommandSpec::new("docker").args(["logs", "--tail", &tail, "--timestamps", id]);
    let output = transport.run(&spec).await?.into_result("docker")?;
    let mut lines: Vec<String> = output.stdout.lines().map(str::to_owned).collect();
    lines.extend(output.stderr.lines().map(str::to_owned));
    Ok(lines)
}

// --- docker ps -------------------------------------------------------------

#[derive(Deserialize)]
struct RawPs {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Names", default)]
    names: String,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Ports", default)]
    ports: String,
    #[serde(rename = "CreatedAt", default)]
    created_at: String,
}

fn parse_ps(s: &str) -> Vec<Container> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<RawPs>(line).ok())
        .map(|raw| Container {
            health: ContainerHealth::from_status_text(&raw.status),
            id: raw.id,
            name: raw.names,
            image: raw.image,
            state: raw.state,
            status: raw.status,
            ports: raw.ports,
            created: raw.created_at,
        })
        .collect()
}

// --- docker stats ----------------------------------------------------------

#[derive(Deserialize)]
struct RawStats {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "CPUPerc", default)]
    cpu_perc: String,
    #[serde(rename = "MemPerc", default)]
    mem_perc: String,
    #[serde(rename = "MemUsage", default)]
    mem_usage: String,
}

fn parse_stats(s: &str) -> Vec<ContainerStats> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<RawStats>(line).ok())
        .map(|raw| ContainerStats {
            id: raw.id,
            name: raw.name,
            cpu_percent: parse_percent(&raw.cpu_perc),
            mem_percent: parse_percent(&raw.mem_perc),
            mem_usage: raw.mem_usage,
        })
        .collect()
}

fn parse_percent(s: &str) -> f64 {
    s.trim().trim_end_matches('%').parse().unwrap_or(0.0)
}

// --- docker inspect --------------------------------------------------------

#[derive(Deserialize)]
struct RawInspect {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "RestartCount", default)]
    restart_count: u32,
    #[serde(rename = "Config", default)]
    config: RawConfig,
    #[serde(rename = "HostConfig", default)]
    host_config: RawHostConfig,
    #[serde(rename = "Mounts", default)]
    mounts: Vec<RawMount>,
    #[serde(rename = "State", default)]
    state: RawState,
    #[serde(rename = "NetworkSettings", default)]
    network_settings: RawNetworkSettings,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(rename = "Image", default)]
    image: String,
}

#[derive(Deserialize, Default)]
struct RawHostConfig {
    #[serde(rename = "Privileged", default)]
    privileged: bool,
    #[serde(rename = "Memory", default)]
    memory: u64,
    #[serde(rename = "RestartPolicy", default)]
    restart_policy: RawRestartPolicy,
}

#[derive(Deserialize, Default)]
struct RawRestartPolicy {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "MaximumRetryCount", default)]
    max_retry_count: u32,
}

#[derive(Deserialize, Default)]
struct RawMount {
    #[serde(rename = "Source", default)]
    source: String,
    #[serde(rename = "Destination", default)]
    destination: String,
    #[serde(rename = "RW", default)]
    rw: bool,
}

#[derive(Deserialize, Default)]
struct RawState {
    #[serde(rename = "Health", default)]
    health: Option<RawHealth>,
}

#[derive(Deserialize, Default)]
struct RawHealth {
    #[serde(rename = "Status", default)]
    status: String,
}

#[derive(Deserialize, Default)]
struct RawNetworkSettings {
    #[serde(rename = "Networks", default)]
    networks: std::collections::HashMap<String, serde_json::Value>,
    #[serde(rename = "Ports", default)]
    ports: std::collections::HashMap<String, Option<Vec<RawPortBinding>>>,
}

#[derive(Deserialize, Default)]
struct RawPortBinding {
    #[serde(rename = "HostIp", default)]
    host_ip: String,
    #[serde(rename = "HostPort", default)]
    host_port: String,
}

fn parse_inspect(s: &str) -> Option<InspectSummary> {
    let raw: RawInspect = serde_json::from_str::<Vec<RawInspect>>(s)
        .ok()?
        .into_iter()
        .next()?;

    let mut networks: Vec<String> = raw.network_settings.networks.into_keys().collect();
    networks.sort();

    let mut published_ports: Vec<PublishedPort> = raw
        .network_settings
        .ports
        .into_iter()
        .flat_map(|(spec, bindings)| {
            let (port_str, protocol) = spec.split_once('/').unwrap_or((spec.as_str(), "tcp"));
            let container_port = port_str.parse().unwrap_or(0);
            let protocol = protocol.to_owned();
            bindings
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |b| {
                    Some(PublishedPort {
                        host_ip: b.host_ip,
                        host_port: b.host_port.parse().ok()?,
                        container_port,
                        protocol: protocol.clone(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    published_ports.sort_by(|a, b| {
        a.host_port
            .cmp(&b.host_port)
            .then_with(|| a.host_ip.cmp(&b.host_ip))
    });

    Some(InspectSummary {
        id: raw.id,
        name: raw.name.trim_start_matches('/').to_owned(),
        image: raw.config.image,
        privileged: raw.host_config.privileged,
        restart_policy: raw.host_config.restart_policy.name,
        max_retry_count: raw.host_config.restart_policy.max_retry_count,
        restart_count: raw.restart_count,
        memory_limit_bytes: raw.host_config.memory,
        networks,
        mounts: raw
            .mounts
            .into_iter()
            .map(|m| Mount {
                source: m.source,
                destination: m.destination,
                rw: m.rw,
            })
            .collect(),
        published_ports,
        health: raw
            .state
            .health
            .and_then(|h| ContainerHealth::from_inspect(&h.status)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    const PS_CMD: &str = "docker ps -a --no-trunc --format {{json .}}";
    const STATS_CMD: &str = "docker stats --no-stream --format {{json .}}";

    #[test]
    fn parses_container_list() {
        let containers = parse_ps(include_str!("../fixtures/docker-ps.json"));
        assert_eq!(containers.len(), 4);
        let redis = &containers[0];
        assert_eq!(redis.name, "redis");
        assert_eq!(redis.image, "redis:latest");
        assert!(redis.is_running());
        assert_eq!(redis.health, None);

        let web = containers.iter().find(|c| c.name == "web").unwrap();
        assert_eq!(web.health, Some(ContainerHealth::Healthy));
        let db = containers.iter().find(|c| c.name == "db").unwrap();
        assert_eq!(db.health, Some(ContainerHealth::Unhealthy));
        let worker = containers.iter().find(|c| c.name == "worker").unwrap();
        assert!(!worker.is_running());
        assert_eq!(worker.state, "exited");
    }

    #[test]
    fn parses_stats_percentages() {
        let stats = parse_stats(include_str!("../fixtures/docker-stats.json"));
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].name, "redis");
        assert!((stats[0].cpu_percent - 0.15).abs() < f64::EPSILON);
        assert!((stats[0].mem_percent - 1.20).abs() < f64::EPSILON);
        assert_eq!(stats[0].mem_usage, "12MiB / 1GiB");
    }

    #[test]
    fn parses_inspect_summary() {
        let summary = parse_inspect(include_str!("../fixtures/docker-inspect.json")).unwrap();
        assert_eq!(summary.name, "redis");
        assert_eq!(summary.image, "redis:latest");
        assert!(summary.privileged);
        assert_eq!(summary.restart_policy, "always");
        assert_eq!(summary.restart_count, 7);
        assert_eq!(summary.memory_limit_bytes, 0);
        assert_eq!(summary.networks, ["bridge"]);
        assert_eq!(summary.health, Some(ContainerHealth::Unhealthy));

        assert!(summary.uses_latest_tag());
        assert!(
            summary
                .mounts
                .iter()
                .any(|m| m.source == "/var/run/docker.sock")
        );

        // 6379/tcp published on 0.0.0.0 and ::, sorted by port then ip.
        assert_eq!(summary.published_ports.len(), 2);
        assert_eq!(summary.published_ports[0].container_port, 6379);
        assert_eq!(summary.published_ports[0].host_port, 6379);
        assert_eq!(summary.published_ports[0].protocol, "tcp");
    }

    #[test]
    fn latest_tag_detection() {
        let make = |image: &str| InspectSummary {
            id: String::new(),
            name: String::new(),
            image: image.to_owned(),
            privileged: false,
            restart_policy: String::new(),
            max_retry_count: 0,
            restart_count: 0,
            memory_limit_bytes: 0,
            networks: Vec::new(),
            mounts: Vec::new(),
            published_ports: Vec::new(),
            health: None,
        };
        assert!(make("redis:latest").uses_latest_tag());
        assert!(make("redis").uses_latest_tag());
        assert!(make("registry:5000/app").uses_latest_tag());
        assert!(!make("nginx:1.25").uses_latest_tag());
        assert!(!make("registry:5000/app:1.0").uses_latest_tag());
    }

    #[tokio::test]
    async fn collector_and_helpers_read_docker() {
        let transport = MockTransport::new()
            .with_stdout(PS_CMD, include_str!("../fixtures/docker-ps.json"))
            .with_stdout(STATS_CMD, include_str!("../fixtures/docker-stats.json"))
            .with_stdout(
                "docker inspect abc123",
                include_str!("../fixtures/docker-inspect.json"),
            );
        assert_eq!(
            DockerCollector::new()
                .collect(&transport)
                .await
                .unwrap()
                .len(),
            4
        );
        assert_eq!(container_stats(&transport).await.unwrap().len(), 2);
        assert!(
            inspect_container(&transport, "abc123")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn reads_container_logs_merging_streams() {
        let transport = MockTransport::new().with_command(
            "docker logs --tail 100 --timestamps redis",
            CommandOutput {
                exit_code: Some(0),
                stdout: "2026-05-24T10:00:00Z Ready to accept connections\n".to_owned(),
                stderr: "2026-05-24T10:00:01Z WARNING memory overcommit disabled\n".to_owned(),
                duration: std::time::Duration::ZERO,
            },
        );
        let logs = container_logs(&transport, "redis", 100).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert!(logs[0].contains("Ready to accept connections"));
        assert!(logs[1].contains("memory overcommit"));
    }

    #[tokio::test]
    async fn missing_docker_surfaces_error() {
        // Nothing configured: the transport errors, and the collector propagates it
        // so the UI can show "Docker unavailable" rather than "no containers".
        let transport = MockTransport::new();
        assert!(DockerCollector::new().collect(&transport).await.is_err());
    }
}
