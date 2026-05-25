//! Firewall collector: name the active manager (firewalld / ufw) and read the
//! *effective* packet-filter ruleset (`nft` first, then `iptables -S`) into a
//! summary of tables, chains and rule counts.
//!
//! Read-only and best-effort: listing the ruleset usually needs root, so when
//! the probe is denied the snapshot degrades to a "needs privilege" note rather
//! than failing. The cross-correlation of firewall rules against listening
//! sockets (the prototype's "rule but no process bound" note) is deliberately
//! left out — it cannot be derived reliably across backends without
//! fragile, fakeable parsing.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A summary of the host's firewall state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallSnapshot {
    /// The active manager or ruleset engine: `firewalld`, `ufw`, `nftables`,
    /// `iptables`, or `none`.
    pub backend: String,
    /// Whether a firewall is active (a manager is running or a ruleset exists).
    pub active: bool,
    /// Tables present in the effective ruleset (e.g. `inet filter`, `ip nat`).
    pub tables: Vec<String>,
    /// Chains present in the effective ruleset.
    pub chains: Vec<String>,
    /// Number of rule statements across all chains.
    pub rule_count: usize,
    /// Caveats worth surfacing (e.g. that the listing needs elevated privilege).
    pub notes: Vec<String>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads the firewall state agentlessly.
#[derive(Debug, Default, Clone, Copy)]
pub struct FirewallCollector;

impl FirewallCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for FirewallCollector {
    type Output = FirewallSnapshot;

    fn module(&self) -> ModuleId {
        ModuleId::Network
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<FirewallSnapshot> {
        let manager = detect_manager(transport).await;
        let ruleset = read_ruleset(transport).await;

        let mut snapshot = ruleset.unwrap_or_default();
        match manager {
            Some((name, active)) => {
                // A manager (firewalld/ufw) front-ends the engine; name it but
                // keep the real table/chain/rule counts from the engine read.
                snapshot.backend = name;
                snapshot.active = active || snapshot.rule_count > 0;
            }
            None => {
                if snapshot.backend.is_empty() {
                    snapshot.backend = "none".to_owned();
                }
            }
        }
        if snapshot.backend.is_empty() {
            snapshot.backend = "none".to_owned();
        }
        Ok(snapshot)
    }
}

/// Detect a firewall manager and whether it is running. Returns `(name, active)`.
async fn detect_manager(transport: &dyn Transport) -> Option<(String, bool)> {
    // firewalld: `firewall-cmd --state` prints `running` / `not running`.
    let spec = CommandSpec::new("firewall-cmd")
        .arg("--state")
        .timeout(PROBE_TIMEOUT);
    if let Ok(out) = transport.run(&spec).await {
        let text = format!("{}{}", out.stdout, out.stderr);
        if text.contains("running") {
            return Some(("firewalld".to_owned(), !text.contains("not running")));
        }
    }
    // ufw: `ufw status` prints `Status: active` / `Status: inactive`.
    let spec = CommandSpec::new("ufw")
        .arg("status")
        .timeout(PROBE_TIMEOUT)
        .privileged();
    if let Ok(out) = transport.run(&spec).await
        && out.stdout.contains("Status:")
    {
        return Some(("ufw".to_owned(), out.stdout.contains("Status: active")));
    }
    None
}

/// Read the effective ruleset via `nft` then `iptables -S`. Returns `None` when
/// neither tool answers (missing or permission denied).
async fn read_ruleset(transport: &dyn Transport) -> Option<FirewallSnapshot> {
    let nft = CommandSpec::new("nft")
        .args(["list", "ruleset"])
        .timeout(PROBE_TIMEOUT)
        .privileged();
    if let Ok(out) = transport.run(&nft).await
        && out.success()
        && !out.stdout.trim().is_empty()
    {
        return Some(parse_nft(&out.stdout));
    }

    let ipt = CommandSpec::new("iptables")
        .arg("-S")
        .timeout(PROBE_TIMEOUT)
        .privileged();
    if let Ok(out) = transport.run(&ipt).await
        && out.success()
    {
        return Some(parse_iptables(&out.stdout));
    }

    // Both tools refused. Surface that a listing needs privilege rather than
    // claiming "no firewall".
    Some(FirewallSnapshot {
        notes: vec!["ruleset listing unavailable (needs privilege?)".to_owned()],
        ..FirewallSnapshot::default()
    })
}

/// Parse `nft list ruleset` into a summary. Tables are `family name`; chains are
/// the chain identifiers; rule statements are the lines inside a chain that are
/// neither the `type … hook … policy …` header nor a brace.
fn parse_nft(s: &str) -> FirewallSnapshot {
    let mut tables = Vec::new();
    let mut chains = Vec::new();
    let mut rule_count = 0usize;
    let mut in_chain = false;

    for raw in s.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("table ") {
            // e.g. "inet filter {" → "inet filter"
            let name = rest.trim_end_matches('{').trim();
            tables.push(name.to_owned());
        } else if let Some(rest) = line.strip_prefix("chain ") {
            let name = rest.trim_end_matches('{').trim();
            chains.push(name.to_owned());
            in_chain = true;
        } else if line == "}" {
            in_chain = false;
        } else if in_chain && !line.starts_with("type ") {
            rule_count += 1;
        }
    }

    FirewallSnapshot {
        backend: "nftables".to_owned(),
        active: rule_count > 0 || !chains.is_empty(),
        tables,
        chains,
        rule_count,
        notes: Vec::new(),
    }
}

/// Parse `iptables -S` (filter table) into a summary: `-P`/`-N` declare chains,
/// `-A` lines are the appended rules.
fn parse_iptables(s: &str) -> FirewallSnapshot {
    let mut chains = Vec::new();
    let mut rule_count = 0usize;
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("-P ")
            .or_else(|| line.strip_prefix("-N "))
        {
            if let Some(name) = rest.split_whitespace().next() {
                let name = name.to_owned();
                if !chains.contains(&name) {
                    chains.push(name);
                }
            }
        } else if line.starts_with("-A ") {
            rule_count += 1;
        }
    }
    FirewallSnapshot {
        backend: "iptables".to_owned(),
        active: rule_count > 0,
        tables: vec!["filter".to_owned()],
        chains,
        rule_count,
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    #[test]
    fn parses_nft_ruleset() {
        let snap = parse_nft(include_str!("../fixtures/nft-ruleset.txt"));
        assert_eq!(snap.backend, "nftables");
        assert!(snap.active);
        assert_eq!(snap.tables, ["inet filter", "ip nat"]);
        assert_eq!(
            snap.chains,
            ["input", "forward", "output", "prerouting", "postrouting"]
        );
        // 5 rules in input + 1 masquerade in postrouting; chain `type … policy`
        // headers and braces are not counted.
        assert_eq!(snap.rule_count, 6);
    }

    #[test]
    fn parses_iptables_save() {
        let snap = parse_iptables(include_str!("../fixtures/iptables-s.txt"));
        assert_eq!(snap.backend, "iptables");
        assert!(snap.active);
        assert_eq!(snap.rule_count, 6); // six -A lines
        assert!(snap.chains.contains(&"INPUT".to_owned()));
        assert!(snap.chains.contains(&"DOCKER".to_owned()));
    }

    #[tokio::test]
    async fn degrades_when_ruleset_listing_denied() {
        // No firewall tools configured → manager None, ruleset unavailable.
        let snap = FirewallCollector::new()
            .collect(&MockTransport::new())
            .await
            .unwrap();
        assert_eq!(snap.backend, "none");
        assert!(!snap.active);
        assert!(!snap.notes.is_empty());
    }

    #[tokio::test]
    async fn reads_nft_when_available() {
        let transport = MockTransport::new().with_stdout(
            "nft list ruleset",
            include_str!("../fixtures/nft-ruleset.txt"),
        );
        let snap = FirewallCollector::new().collect(&transport).await.unwrap();
        assert_eq!(snap.backend, "nftables");
        assert_eq!(snap.rule_count, 6);
    }
}
