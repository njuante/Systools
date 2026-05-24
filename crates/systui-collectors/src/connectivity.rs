//! Connectivity tools: ping, DNS lookup and TCP connect tests, run on demand
//! from the TUI (`Product.md` §4.6). Each runs a single [`CommandSpec`] through
//! the transport (so it works locally or, later, over SSH) with a bounded
//! timeout, and returns a typed, fixture-tested result.
//!
//! These are diagnostics, not mutations: they read reachability, they never
//! change host state.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use systui_core::{CommandSpec, CoreError, Result, Transport};

/// Outcome of a `ping` run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PingResult {
    pub transmitted: u32,
    pub received: u32,
    pub loss_percent: f64,
    pub rtt_min_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_max_ms: Option<f64>,
}

/// Forward DNS resolution result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsLookup {
    pub host: String,
    pub addresses: Vec<String>,
}

/// Outcome of a TCP connect probe to `host:port`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpProbe {
    pub host: String,
    pub port: u16,
    pub open: bool,
}

/// Send `count` ICMP echoes to `host`, bounded by an overall `timeout`.
pub async fn ping(
    transport: &dyn Transport,
    host: &str,
    count: u32,
    timeout: Duration,
) -> Result<PingResult> {
    let deadline = timeout.as_secs().max(1);
    let spec = CommandSpec::new("ping")
        .args([
            "-c".to_owned(),
            count.to_string(),
            "-w".to_owned(),
            deadline.to_string(),
            host.to_owned(),
        ])
        .timeout(timeout + Duration::from_secs(2));
    // ping exits non-zero on packet loss but still prints statistics, so we
    // parse stdout regardless of exit status.
    let output = transport.run(&spec).await?;
    parse_ping(&output.stdout).ok_or_else(|| CoreError::parse("ping", "no ping statistics found"))
}

/// Resolve `host` to its addresses via `getent ahosts` (NSS, the most portable
/// resolver available on glibc Linux).
pub async fn dns_lookup(transport: &dyn Transport, host: &str) -> Result<DnsLookup> {
    let spec = CommandSpec::new("getent")
        .args(["ahosts".to_owned(), host.to_owned()])
        .timeout(Duration::from_secs(5));
    let output = transport.run(&spec).await?.into_result("getent")?;
    Ok(DnsLookup {
        host: host.to_owned(),
        addresses: parse_getent_ahosts(&output.stdout),
    })
}

/// Probe whether a TCP connection to `host:port` can be established, using
/// `nc -z`. A non-zero exit means closed/filtered, not a tool error.
pub async fn tcp_connect(
    transport: &dyn Transport,
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<TcpProbe> {
    let wait = timeout.as_secs().max(1);
    let spec = CommandSpec::new("nc")
        .args([
            "-z".to_owned(),
            "-w".to_owned(),
            wait.to_string(),
            host.to_owned(),
            port.to_string(),
        ])
        .timeout(timeout + Duration::from_secs(2));
    let output = transport.run(&spec).await?;
    Ok(TcpProbe {
        host: host.to_owned(),
        port,
        open: output.success(),
    })
}

fn parse_ping(s: &str) -> Option<PingResult> {
    let stats = s.lines().find(|l| l.contains("packets transmitted"))?;
    let mut transmitted = 0;
    let mut received = 0;
    let mut loss_percent = 0.0;
    for field in stats.split(',') {
        let field = field.trim();
        if field.contains("transmitted") {
            transmitted = first_number(field).unwrap_or(0.0) as u32;
        } else if field.contains("received") {
            received = first_number(field).unwrap_or(0.0) as u32;
        } else if field.contains("packet loss") {
            loss_percent = field
                .split_whitespace()
                .find_map(|t| t.strip_suffix('%'))
                .and_then(|t| t.parse().ok())
                .unwrap_or(0.0);
        }
    }

    let (mut rtt_min, mut rtt_avg, mut rtt_max) = (None, None, None);
    if let Some(line) = s
        .lines()
        .find(|l| l.contains("min/avg/max") && l.contains('='))
        && let Some((_, values)) = line.split_once('=')
    {
        let nums: Vec<f64> = values
            .split('/')
            .filter_map(|p| p.split_whitespace().next())
            .filter_map(|p| p.parse().ok())
            .collect();
        rtt_min = nums.first().copied();
        rtt_avg = nums.get(1).copied();
        rtt_max = nums.get(2).copied();
    }

    Some(PingResult {
        transmitted,
        received,
        loss_percent,
        rtt_min_ms: rtt_min,
        rtt_avg_ms: rtt_avg,
        rtt_max_ms: rtt_max,
    })
}

/// First whitespace-delimited token in `field` that parses as a number.
fn first_number(field: &str) -> Option<f64> {
    field.split_whitespace().find_map(|t| t.parse().ok())
}

fn parse_getent_ahosts(s: &str) -> Vec<String> {
    let mut addresses = Vec::new();
    for line in s.lines() {
        if let Some(addr) = line.split_whitespace().next()
            && !addresses.iter().any(|a| a == addr)
        {
            addresses.push(addr.to_owned());
        }
    }
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    #[test]
    fn parses_successful_ping() {
        let res = parse_ping(include_str!("../fixtures/ping.txt")).unwrap();
        assert_eq!(res.transmitted, 4);
        assert_eq!(res.received, 4);
        assert_eq!(res.loss_percent, 0.0);
        assert_eq!(res.rtt_min_ms, Some(11.482));
        assert_eq!(res.rtt_avg_ms, Some(11.715));
        assert_eq!(res.rtt_max_ms, Some(12.045));
    }

    #[test]
    fn parses_ping_with_total_loss() {
        let res = parse_ping(include_str!("../fixtures/ping-loss.txt")).unwrap();
        assert_eq!(res.transmitted, 5);
        assert_eq!(res.received, 0);
        assert_eq!(res.loss_percent, 100.0);
        assert_eq!(res.rtt_avg_ms, None);
    }

    #[test]
    fn parses_getent_unique_addresses() {
        let addrs = parse_getent_ahosts(include_str!("../fixtures/getent-ahosts.txt"));
        assert_eq!(
            addrs,
            ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"]
        );
    }

    #[tokio::test]
    async fn ping_runs_and_parses() {
        let transport = MockTransport::new().with_stdout(
            "ping -c 4 -w 5 example.com",
            include_str!("../fixtures/ping.txt"),
        );
        let res = ping(&transport, "example.com", 4, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(res.received, 4);
    }

    #[tokio::test]
    async fn dns_lookup_resolves() {
        let transport = MockTransport::new().with_stdout(
            "getent ahosts example.com",
            include_str!("../fixtures/getent-ahosts.txt"),
        );
        let res = dns_lookup(&transport, "example.com").await.unwrap();
        assert_eq!(res.host, "example.com");
        assert_eq!(res.addresses.len(), 2);
    }

    #[tokio::test]
    async fn tcp_connect_reads_exit_code() {
        let open = MockTransport::new().with_command(
            "nc -z -w 3 example.com 443",
            CommandOutput {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::ZERO,
            },
        );
        assert!(
            tcp_connect(&open, "example.com", 443, Duration::from_secs(3))
                .await
                .unwrap()
                .open
        );

        let closed = MockTransport::new().with_command(
            "nc -z -w 3 example.com 9999",
            CommandOutput {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::ZERO,
            },
        );
        assert!(
            !tcp_connect(&closed, "example.com", 9999, Duration::from_secs(3))
                .await
                .unwrap()
                .open
        );
    }
}
