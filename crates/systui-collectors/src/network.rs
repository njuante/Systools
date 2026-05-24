//! Network collector: interfaces and their addresses, routes, DNS configuration,
//! listening sockets (with the owning process) and active connections by state.
//!
//! All data is read agentlessly through a [`Transport`]: `ip -j addr` / `ip -j
//! route` (JSON, with a text fallback for older `iproute2`/busybox), the
//! `/etc/resolv.conf` file, and `ss` for sockets. Parsers are pure functions
//! covered by fixture tests. Each source degrades to empty independently, so a
//! missing tool or denied permission yields partial data rather than a crash.
//!
//! Port -> process -> systemd unit *correlation* and the exposure-map
//! *classification* are built on top of this snapshot in later v0.3 sessions.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// IP address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddrFamily {
    V4,
    V6,
}

/// A single address bound to an interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceAddr {
    pub ip: String,
    pub prefix_len: u8,
    pub family: AddrFamily,
}

/// A network interface and the addresses bound to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetInterface {
    pub name: String,
    /// Operational state, e.g. `UP`, `DOWN`, `UNKNOWN`.
    pub state: String,
    pub addrs: Vec<InterfaceAddr>,
}

/// A routing-table entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// Destination, e.g. `default` or `192.168.1.0/24`.
    pub dst: String,
    pub gateway: Option<String>,
    pub dev: String,
    /// Preferred source address, when the kernel reports one.
    pub prefsrc: Option<String>,
}

/// Resolver configuration parsed from `/etc/resolv.conf`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsConfig {
    pub nameservers: Vec<String>,
    pub search: Vec<String>,
}

/// Transport-layer protocol of a socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

/// The process owning a socket, as reported by `ss -p`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessRef {
    pub pid: u32,
    pub name: String,
}

/// A socket in the LISTEN/UNCONN state, with its owning process when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listener {
    pub protocol: Protocol,
    /// The local bind address, e.g. `0.0.0.0`, `127.0.0.1`, `::` or `fe80::1`.
    pub local_ip: String,
    pub port: u16,
    /// `None` when `ss` could not attribute the socket (busybox, no privilege).
    pub process: Option<ProcessRef>,
    /// The systemd unit owning the process, derived from `/proc/<pid>/cgroup`.
    /// `None` when there is no owning process, the cgroup is unreadable, or the
    /// process is not under a systemd unit.
    pub unit: Option<String>,
}

impl Listener {
    /// Whether the socket binds a wildcard/any address (`0.0.0.0`, `::`, `*`),
    /// i.e. it is reachable on every interface. Refined into a full exposure
    /// classification in the S3.4 exposure map.
    pub fn binds_wildcard(&self) -> bool {
        matches!(self.local_ip.as_str(), "0.0.0.0" | "::" | "*")
    }
}

/// An active connection from `ss -tan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// TCP state, e.g. `ESTAB`, `TIME-WAIT`, `SYN-RECV`, `LISTEN`.
    pub state: String,
    pub local_ip: String,
    pub local_port: u16,
    pub peer_ip: String,
    pub peer_port: u16,
}

/// A point-in-time view of host networking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub interfaces: Vec<NetInterface>,
    pub routes: Vec<Route>,
    pub dns: DnsConfig,
    pub listeners: Vec<Listener>,
    pub connections: Vec<Connection>,
}

impl NetworkSnapshot {
    /// Count connections grouped by TCP state (e.g. how many `TIME-WAIT`).
    pub fn connection_state_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for conn in &self.connections {
            *counts.entry(conn.state.clone()).or_insert(0) += 1;
        }
        counts
    }
}

/// Slow-changing host networking: interfaces, routes and DNS config. Cached so
/// later refreshes skip `ip addr`, `ip route` and the `resolv.conf` read while
/// the live listeners/connections are still collected every tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetStatics {
    pub interfaces: Vec<NetInterface>,
    pub routes: Vec<Route>,
    pub dns: DnsConfig,
}

impl NetStatics {
    /// Extract the cacheable, slow-changing parts of a collected snapshot.
    pub fn from_snapshot(snapshot: &NetworkSnapshot) -> Self {
        Self {
            interfaces: snapshot.interfaces.clone(),
            routes: snapshot.routes.clone(),
            dns: snapshot.dns.clone(),
        }
    }
}

/// Reads a full [`NetworkSnapshot`] from the host.
///
/// If constructed with [`NetworkCollector::with_statics`], the slow-changing
/// parts ([`NetStatics`]) are reused instead of re-collected, so a refresh only
/// runs the live `ss` queries.
#[derive(Debug, Default, Clone)]
pub struct NetworkCollector {
    statics: Option<NetStatics>,
}

impl NetworkCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reuse cached slow-changing networking when present (tiered refresh); pass
    /// `None` to collect it fresh.
    pub fn with_statics(statics: Option<NetStatics>) -> Self {
        Self { statics }
    }
}

#[async_trait]
impl Collector for NetworkCollector {
    type Output = NetworkSnapshot;

    fn module(&self) -> ModuleId {
        ModuleId::Network
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<NetworkSnapshot> {
        // Slow-changing parts: reuse the cache when tiered, else collect them.
        let (interfaces, routes, dns) = match &self.statics {
            Some(s) => (s.interfaces.clone(), s.routes.clone(), s.dns.clone()),
            None => (
                collect_interfaces(transport).await,
                collect_routes(transport).await,
                collect_dns(transport).await,
            ),
        };
        Ok(NetworkSnapshot {
            interfaces,
            routes,
            dns,
            // Live: always collected.
            listeners: collect_listeners(transport).await,
            connections: collect_connections(transport).await,
        })
    }
}

async fn run_stdout(transport: &dyn Transport, program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    match transport.run(&spec).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

async fn collect_interfaces(transport: &dyn Transport) -> Vec<NetInterface> {
    if let Some(out) = run_stdout(transport, "ip", &["-j", "addr"]).await
        && let Some(ifaces) = parse_ip_addr_json(&out)
    {
        return ifaces;
    }
    match run_stdout(transport, "ip", &["addr"]).await {
        Some(out) => parse_ip_addr_text(&out),
        None => Vec::new(),
    }
}

async fn collect_routes(transport: &dyn Transport) -> Vec<Route> {
    if let Some(out) = run_stdout(transport, "ip", &["-j", "route"]).await
        && let Some(routes) = parse_ip_route_json(&out)
    {
        return routes;
    }
    match run_stdout(transport, "ip", &["route"]).await {
        Some(out) => parse_ip_route_text(&out),
        None => Vec::new(),
    }
}

async fn collect_dns(transport: &dyn Transport) -> DnsConfig {
    match transport.read_file("/etc/resolv.conf").await {
        Ok(bytes) => parse_resolv_conf(&String::from_utf8_lossy(&bytes)),
        Err(_) => DnsConfig::default(),
    }
}

async fn collect_listeners(transport: &dyn Transport) -> Vec<Listener> {
    let mut listeners = match run_stdout(transport, "ss", &["-tulpn"]).await {
        Some(out) => parse_ss_listeners(&out),
        None => Vec::new(),
    };
    correlate_units(transport, &mut listeners).await;
    listeners
}

async fn collect_connections(transport: &dyn Transport) -> Vec<Connection> {
    match run_stdout(transport, "ss", &["-tan"]).await {
        Some(out) => parse_ss_connections(&out),
        None => Vec::new(),
    }
}

// --- ip addr ---------------------------------------------------------------

#[derive(Deserialize)]
struct IpAddrEntry {
    ifname: String,
    #[serde(default)]
    operstate: String,
    #[serde(default)]
    addr_info: Vec<IpAddrInfo>,
}

#[derive(Deserialize)]
struct IpAddrInfo {
    family: String,
    local: String,
    prefixlen: u8,
}

fn family_from_str(s: &str) -> Option<AddrFamily> {
    match s {
        "inet" => Some(AddrFamily::V4),
        "inet6" => Some(AddrFamily::V6),
        _ => None,
    }
}

fn parse_ip_addr_json(s: &str) -> Option<Vec<NetInterface>> {
    let entries: Vec<IpAddrEntry> = serde_json::from_str(s).ok()?;
    Some(
        entries
            .into_iter()
            .map(|e| NetInterface {
                name: e.ifname,
                state: if e.operstate.is_empty() {
                    "UNKNOWN".to_owned()
                } else {
                    e.operstate
                },
                addrs: e
                    .addr_info
                    .into_iter()
                    .filter_map(|a| {
                        family_from_str(&a.family).map(|family| InterfaceAddr {
                            ip: a.local,
                            prefix_len: a.prefixlen,
                            family,
                        })
                    })
                    .collect(),
            })
            .collect(),
    )
}

fn parse_ip_addr_text(s: &str) -> Vec<NetInterface> {
    let mut ifaces: Vec<NetInterface> = Vec::new();
    for line in s.lines() {
        if is_iface_header(line) {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let name = tokens
                .get(1)
                .map(|t| t.trim_end_matches(':'))
                .unwrap_or_default()
                .to_owned();
            let state = tokens
                .iter()
                .position(|&t| t == "state")
                .and_then(|i| tokens.get(i + 1))
                .map(|s| (*s).to_owned())
                .unwrap_or_else(|| "UNKNOWN".to_owned());
            ifaces.push(NetInterface {
                name,
                state,
                addrs: Vec::new(),
            });
            continue;
        }
        let trimmed = line.trim_start();
        let (rest, family) = if let Some(rest) = trimmed.strip_prefix("inet ") {
            (rest, AddrFamily::V4)
        } else if let Some(rest) = trimmed.strip_prefix("inet6 ") {
            (rest, AddrFamily::V6)
        } else {
            continue;
        };
        let Some(iface) = ifaces.last_mut() else {
            continue;
        };
        if let Some(addr) = parse_cidr(rest.split_whitespace().next().unwrap_or_default(), family) {
            iface.addrs.push(addr);
        }
    }
    ifaces
}

/// A header line in `ip addr` text output, e.g. `2: eth0: <...> state UP`.
fn is_iface_header(line: &str) -> bool {
    !line.starts_with(char::is_whitespace)
        && line
            .split_once(':')
            .is_some_and(|(index, _)| index.trim().parse::<u32>().is_ok())
}

fn parse_cidr(token: &str, family: AddrFamily) -> Option<InterfaceAddr> {
    let (ip, prefix) = token.split_once('/')?;
    Some(InterfaceAddr {
        ip: ip.to_owned(),
        prefix_len: prefix.parse().ok()?,
        family,
    })
}

// --- ip route --------------------------------------------------------------

#[derive(Deserialize)]
struct IpRouteEntry {
    dst: String,
    #[serde(default)]
    gateway: Option<String>,
    #[serde(default)]
    dev: String,
    #[serde(default)]
    prefsrc: Option<String>,
}

fn parse_ip_route_json(s: &str) -> Option<Vec<Route>> {
    let entries: Vec<IpRouteEntry> = serde_json::from_str(s).ok()?;
    Some(
        entries
            .into_iter()
            .map(|e| Route {
                dst: e.dst,
                gateway: e.gateway,
                dev: e.dev,
                prefsrc: e.prefsrc,
            })
            .collect(),
    )
}

fn parse_ip_route_text(s: &str) -> Vec<Route> {
    s.lines().filter_map(parse_ip_route_line).collect()
}

fn parse_ip_route_line(line: &str) -> Option<Route> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let dst = (*tokens.first()?).to_owned();
    let value_after = |key: &str| {
        tokens
            .iter()
            .position(|&t| t == key)
            .and_then(|i| tokens.get(i + 1))
            .map(|v| (*v).to_owned())
    };
    Some(Route {
        dst,
        gateway: value_after("via"),
        dev: value_after("dev").unwrap_or_default(),
        prefsrc: value_after("src"),
    })
}

// --- resolv.conf -----------------------------------------------------------

fn parse_resolv_conf(s: &str) -> DnsConfig {
    let mut dns = DnsConfig::default();
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("nameserver") => {
                if let Some(addr) = tokens.next() {
                    dns.nameservers.push(addr.to_owned());
                }
            }
            // `domain` and `search` are mutually exclusive; both feed the
            // search list (the last directive in the file wins, like libc).
            Some("search") | Some("domain") => {
                dns.search = tokens.map(str::to_owned).collect();
            }
            _ => {}
        }
    }
    dns
}

// --- ss --------------------------------------------------------------------

fn protocol_from_netid(netid: &str) -> Option<Protocol> {
    match netid {
        "tcp" => Some(Protocol::Tcp),
        "udp" => Some(Protocol::Udp),
        _ => None,
    }
}

pub(crate) fn parse_ss_listeners(s: &str) -> Vec<Listener> {
    s.lines().filter_map(parse_ss_listener_line).collect()
}

fn parse_ss_listener_line(line: &str) -> Option<Listener> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    // Netid State Recv-Q Send-Q Local Peer [Process]
    if fields.len() < 6 {
        return None;
    }
    let protocol = protocol_from_netid(fields[0])?;
    let (local_ip, port) = parse_socket_addr(fields[4])?;
    let process = fields.get(6).and_then(|f| parse_ss_process(f));
    Some(Listener {
        protocol,
        local_ip,
        port,
        process,
        unit: None,
    })
}

// --- port -> process -> unit correlation -----------------------------------

/// Fill in each listener's owning systemd unit by reading the cgroup of its
/// process. PIDs are read at most once. Missing/unreadable cgroups leave the
/// unit as `None` (partial data, never a failure).
pub async fn correlate_units(transport: &dyn Transport, listeners: &mut [Listener]) {
    let mut cache: HashMap<u32, Option<String>> = HashMap::new();
    for listener in listeners.iter_mut() {
        let Some(process) = &listener.process else {
            continue;
        };
        let unit = match cache.get(&process.pid) {
            Some(unit) => unit.clone(),
            None => {
                let unit = unit_for_pid(transport, process.pid).await;
                cache.insert(process.pid, unit.clone());
                unit
            }
        };
        listener.unit = unit;
    }
}

async fn unit_for_pid(transport: &dyn Transport, pid: u32) -> Option<String> {
    let bytes = transport
        .read_file(&format!("/proc/{pid}/cgroup"))
        .await
        .ok()?;
    unit_from_cgroup(&String::from_utf8_lossy(&bytes))
}

/// Extract the leaf-most systemd unit from `/proc/<pid>/cgroup`, handling both
/// cgroup v2 (`0::/system.slice/nginx.service`) and v1 (multiple
/// `hierarchy:controller:path` lines). The first `.service`/`.scope` segment
/// found walking each path from the leaf wins; pure slices yield `None`.
fn unit_from_cgroup(content: &str) -> Option<String> {
    for line in content.lines() {
        let path = line.rsplit_once(':').map_or(line, |(_, path)| path);
        for segment in path.rsplit('/') {
            if segment.ends_with(".service") || segment.ends_with(".scope") {
                return Some(segment.to_owned());
            }
        }
    }
    None
}

fn parse_ss_connections(s: &str) -> Vec<Connection> {
    s.lines().filter_map(parse_ss_connection_line).collect()
}

fn parse_ss_connection_line(line: &str) -> Option<Connection> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    // State Recv-Q Send-Q Local Peer
    if fields.len() < 5 {
        return None;
    }
    let (local_ip, local_port) = parse_socket_addr(fields[3])?;
    let (peer_ip, peer_port) = parse_socket_addr(fields[4])?;
    Some(Connection {
        state: fields[0].to_owned(),
        local_ip,
        local_port,
        peer_ip,
        peer_port,
    })
}

/// Split an `ss` `address:port` token, tolerating IPv6 brackets and the `*`
/// wildcard (`0.0.0.0:*` / `[::]:*`), which map to port 0. Returns `None` for
/// header rows whose port field is non-numeric (e.g. `Address:Port`).
fn parse_socket_addr(token: &str) -> Option<(String, u16)> {
    let (host, port) = token.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = if port == "*" { 0 } else { port.parse().ok()? };
    Some((host.to_owned(), port))
}

/// Extract the first process from an `ss` users field, e.g.
/// `users:(("sshd",pid=842,fd=3))`.
fn parse_ss_process(field: &str) -> Option<ProcessRef> {
    let name_start = field.find("((\"")? + 3;
    let name_len = field[name_start..].find('"')?;
    let name = field[name_start..name_start + name_len].to_owned();
    let pid_start = field.find("pid=")? + 4;
    let pid_str: String = field[pid_start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some(ProcessRef {
        pid: pid_str.parse().ok()?,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    #[test]
    fn parses_ip_addr_json() {
        let ifaces = parse_ip_addr_json(include_str!("../fixtures/ip-addr.json")).unwrap();
        assert_eq!(ifaces.len(), 2);
        assert_eq!(ifaces[0].name, "lo");
        assert_eq!(ifaces[0].state, "UNKNOWN");
        assert_eq!(ifaces[0].addrs[0].ip, "127.0.0.1");
        assert_eq!(ifaces[0].addrs[0].prefix_len, 8);
        assert_eq!(ifaces[0].addrs[1].family, AddrFamily::V6);
        let eth0 = &ifaces[1];
        assert_eq!(eth0.name, "eth0");
        assert_eq!(eth0.state, "UP");
        assert_eq!(eth0.addrs[0].ip, "192.168.1.10");
        assert_eq!(eth0.addrs[0].prefix_len, 24);
    }

    #[test]
    fn ip_addr_text_fallback_matches_json() {
        let json = parse_ip_addr_json(include_str!("../fixtures/ip-addr.json")).unwrap();
        let text = parse_ip_addr_text(include_str!("../fixtures/ip-addr.txt"));
        assert_eq!(text, json);
    }

    #[test]
    fn invalid_json_falls_back_to_none() {
        assert!(parse_ip_addr_json("not json").is_none());
        assert!(parse_ip_route_json("not json").is_none());
    }

    #[test]
    fn parses_ip_route_json() {
        let routes = parse_ip_route_json(include_str!("../fixtures/ip-route.json")).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].dst, "default");
        assert_eq!(routes[0].gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(routes[0].dev, "eth0");
        assert_eq!(routes[1].gateway, None);
        assert_eq!(routes[1].prefsrc.as_deref(), Some("192.168.1.10"));
    }

    #[test]
    fn ip_route_text_fallback_matches_json() {
        let json = parse_ip_route_json(include_str!("../fixtures/ip-route.json")).unwrap();
        let text = parse_ip_route_text(include_str!("../fixtures/ip-route.txt"));
        assert_eq!(text, json);
    }

    #[test]
    fn parses_resolv_conf() {
        let dns = parse_resolv_conf(include_str!("../fixtures/resolv.conf"));
        assert_eq!(dns.nameservers, ["192.168.1.1", "8.8.8.8"]);
        // `domain example.com` follows `search`, so it wins.
        assert_eq!(dns.search, ["example.com"]);
    }

    #[test]
    fn parses_ss_listeners_with_owning_process() {
        let listeners = parse_ss_listeners(include_str!("../fixtures/ss-tulpn.txt"));
        assert_eq!(listeners.len(), 6);

        let ssh = &listeners[0];
        assert_eq!(ssh.protocol, Protocol::Tcp);
        assert_eq!(ssh.local_ip, "0.0.0.0");
        assert_eq!(ssh.port, 22);
        assert_eq!(ssh.process.as_ref().unwrap().name, "sshd");
        assert_eq!(ssh.process.as_ref().unwrap().pid, 842);
        assert!(ssh.binds_wildcard());

        let pg = &listeners[1];
        assert_eq!(pg.local_ip, "127.0.0.1");
        assert!(!pg.binds_wildcard());

        // IPv6 brackets are stripped and the first of several processes is kept.
        let nginx = &listeners[2];
        assert_eq!(nginx.local_ip, "::");
        assert_eq!(nginx.port, 443);
        assert_eq!(nginx.process.as_ref().unwrap().pid, 1132);

        let udp = listeners.iter().find(|l| l.port == 68).unwrap();
        assert_eq!(udp.protocol, Protocol::Udp);

        // A listener with no process column degrades to `None`.
        let rpc = listeners.iter().find(|l| l.port == 111).unwrap();
        assert!(rpc.process.is_none());
    }

    #[test]
    fn parses_ss_connections_and_counts_states() {
        let conns = parse_ss_connections(include_str!("../fixtures/ss-tan.txt"));
        assert_eq!(conns.len(), 6);
        let snapshot = NetworkSnapshot {
            interfaces: Vec::new(),
            routes: Vec::new(),
            dns: DnsConfig::default(),
            listeners: Vec::new(),
            connections: conns,
        };
        let counts = snapshot.connection_state_counts();
        assert_eq!(counts["ESTAB"], 2);
        assert_eq!(counts["TIME-WAIT"], 2);
        assert_eq!(counts["SYN-RECV"], 1);
        assert_eq!(counts["LISTEN"], 1);
    }

    #[test]
    fn unit_from_cgroup_handles_v2_v1_and_scopes() {
        assert_eq!(
            unit_from_cgroup(include_str!("../fixtures/cgroup-v2-service.txt")).as_deref(),
            Some("nginx.service")
        );
        assert_eq!(
            unit_from_cgroup(include_str!("../fixtures/cgroup-v1-service.txt")).as_deref(),
            Some("sshd.service")
        );
        // A login session resolves to its leaf scope, not the user manager.
        assert_eq!(
            unit_from_cgroup(include_str!("../fixtures/cgroup-user-scope.txt")).as_deref(),
            Some("session-3.scope")
        );
        // A bare slice carries no unit.
        assert_eq!(unit_from_cgroup("0::/system.slice"), None);
        assert_eq!(unit_from_cgroup("0::/"), None);
    }

    #[tokio::test]
    async fn correlate_units_maps_listeners_to_units() {
        let mut listeners = parse_ss_listeners(include_str!("../fixtures/ss-tulpn.txt"));
        let transport = MockTransport::new()
            .with_file(
                "/proc/842/cgroup",
                include_bytes!("../fixtures/cgroup-v1-service.txt").to_vec(),
            )
            .with_file(
                "/proc/1132/cgroup",
                include_bytes!("../fixtures/cgroup-v2-service.txt").to_vec(),
            );
        correlate_units(&transport, &mut listeners).await;

        let ssh = listeners.iter().find(|l| l.port == 22).unwrap();
        assert_eq!(ssh.unit.as_deref(), Some("sshd.service"));
        let nginx = listeners.iter().find(|l| l.port == 443).unwrap();
        assert_eq!(nginx.unit.as_deref(), Some("nginx.service"));
        // postgres pid has no cgroup fixture -> degrades to no unit.
        let pg = listeners.iter().find(|l| l.port == 5432).unwrap();
        assert_eq!(pg.unit, None);
        // The processless rpcbind listener stays unattributed.
        let rpc = listeners.iter().find(|l| l.port == 111).unwrap();
        assert_eq!(rpc.unit, None);
    }

    #[tokio::test]
    async fn collector_prefers_json_and_assembles_snapshot() {
        let transport = MockTransport::new()
            .with_stdout("ip -j addr", include_str!("../fixtures/ip-addr.json"))
            .with_stdout("ip -j route", include_str!("../fixtures/ip-route.json"))
            .with_stdout("ss -tulpn", include_str!("../fixtures/ss-tulpn.txt"))
            .with_stdout("ss -tan", include_str!("../fixtures/ss-tan.txt"))
            .with_file(
                "/etc/resolv.conf",
                include_bytes!("../fixtures/resolv.conf").to_vec(),
            );
        let snap = NetworkCollector::new().collect(&transport).await.unwrap();
        assert_eq!(snap.interfaces.len(), 2);
        assert_eq!(snap.routes.len(), 2);
        assert_eq!(snap.dns.nameservers.len(), 2);
        assert_eq!(snap.listeners.len(), 6);
        assert_eq!(snap.connections.len(), 6);
    }

    #[tokio::test]
    async fn collector_falls_back_to_text_then_degrades() {
        // No JSON `ip` configured: the text fallback is used; `ss` and DNS are
        // absent entirely, so they degrade to empty without failing.
        let transport = MockTransport::new()
            .with_stdout("ip addr", include_str!("../fixtures/ip-addr.txt"))
            .with_stdout("ip route", include_str!("../fixtures/ip-route.txt"));
        let snap = NetworkCollector::new().collect(&transport).await.unwrap();
        assert_eq!(snap.interfaces.len(), 2);
        assert_eq!(snap.routes.len(), 2);
        assert!(snap.dns.nameservers.is_empty());
        assert!(snap.listeners.is_empty());
        assert!(snap.connections.is_empty());
    }

    #[tokio::test]
    async fn cached_statics_skip_slow_collection() {
        // Only the live `ss` queries are configured — no `ip`/resolv.conf. With
        // cached statics the interfaces/routes/DNS are reused and those commands
        // are skipped, while listeners/connections are still collected.
        let transport = MockTransport::new()
            .with_stdout("ss -tulpn", include_str!("../fixtures/ss-tulpn.txt"))
            .with_stdout("ss -tan", include_str!("../fixtures/ss-tan.txt"));

        let statics = NetStatics {
            interfaces: vec![NetInterface {
                name: "eth0".to_owned(),
                state: "UP".to_owned(),
                addrs: Vec::new(),
            }],
            routes: Vec::new(),
            dns: DnsConfig::default(),
        };
        let snap = NetworkCollector::with_statics(Some(statics))
            .collect(&transport)
            .await
            .unwrap();

        // Slow parts came from the cache.
        assert_eq!(snap.interfaces.len(), 1);
        assert_eq!(snap.interfaces[0].name, "eth0");
        // Live parts were still collected from the transport.
        assert!(!snap.listeners.is_empty());
        assert!(!snap.connections.is_empty());
    }
}
