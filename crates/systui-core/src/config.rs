//! Configuration schema (`Product.md` §11).
//!
//! This module only defines the shape and the defaults. Locating, reading and
//! merging the config files is the job of `systui-storage` (phase 0, S0.5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Top-level SysTUI configuration, mirroring `~/.config/systui/config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub ui: Ui,
    pub security: Security,
    pub thresholds: Thresholds,
    /// Inventory of known hosts, keyed by host id.
    pub hosts: BTreeMap<String, Host>,
    /// Expected-state policies, keyed by policy name.
    pub policies: BTreeMap<String, Policy>,
}

/// General application settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub default_refresh_seconds: u64,
    pub theme: String,
    /// Visual style for the UI ("sober" or "rich"); independent of `theme`.
    pub visual_style: String,
    pub confirm_dangerous_actions: bool,
    pub audit_log: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_refresh_seconds: 3,
            theme: "dark".to_owned(),
            visual_style: "sober".to_owned(),
            confirm_dangerous_actions: true,
            audit_log: true,
        }
    }
}

/// UI-specific settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub show_health_score: bool,
    pub compact_mode: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            show_health_score: true,
            compact_mode: false,
        }
    }
}

/// Security-related thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Security {
    pub ssh_failed_login_window_minutes: u64,
    pub cert_expiry_warning_days: u32,
}

impl Default for Security {
    fn default() -> Self {
        Self {
            ssh_failed_login_window_minutes: 60,
            cert_expiry_warning_days: 30,
        }
    }
}

/// Resource thresholds that drive checks and the health score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub disk_warning: u8,
    pub disk_critical: u8,
    pub ram_warning: u8,
    pub load_warning_multiplier: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            disk_warning: 80,
            disk_critical: 90,
            ram_warning: 85,
            load_warning_multiplier: 1.5,
        }
    }
}

/// A configured host in the inventory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    /// Hostname or IP address.
    pub host: String,
    /// SSH user; `None` falls back to the current user.
    #[serde(default)]
    pub user: Option<String>,
    /// SSH port.
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// Free-form tags for grouping/filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Force read-only mode for this host regardless of CLI flags.
    #[serde(default)]
    pub read_only: bool,
    /// Surface this host first in fleet views.
    #[serde(default)]
    pub favorite: bool,
    /// Name of the policy to evaluate this host against.
    #[serde(default)]
    pub policy: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

/// A host resolved from an `ssh` CLI target — either a known inventory id or an
/// ad-hoc `user@host` (or bare `host`) specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    /// The inventory id when matched, otherwise the raw target string.
    pub id: String,
    /// Whether the target matched a configured inventory host.
    pub from_inventory: bool,
    pub host: String,
    pub user: Option<String>,
    pub port: u16,
    /// Force read-only mode for this host (from its profile).
    pub read_only: bool,
    /// Name of the policy to evaluate this host against, if any.
    pub policy: Option<String>,
}

/// How a host was matched to an expected-state policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    /// The host explicitly set `policy = "name"`.
    ExplicitHost,
    /// The host matched a policy's `match_tags` fallback.
    TagFallback,
}

/// Borrowed policy selected for a host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolicyRef<'a> {
    pub name: &'a str,
    pub policy: &'a Policy,
    pub source: PolicySource,
}

/// Result of resolving a host's expected-state policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PolicyResolution<'a> {
    Matched(PolicyRef<'a>),
    /// The host asked for a policy name that is not present in `[policies]`.
    MissingExplicit {
        name: &'a str,
    },
    None,
}

impl<'a> PolicyResolution<'a> {
    pub fn name(self) -> Option<&'a str> {
        match self {
            Self::Matched(policy) => Some(policy.name),
            Self::MissingExplicit { name } => Some(name),
            Self::None => None,
        }
    }

    pub fn policy(self) -> Option<PolicyRef<'a>> {
        match self {
            Self::Matched(policy) => Some(policy),
            Self::MissingExplicit { .. } | Self::None => None,
        }
    }
}

impl Config {
    /// Insert or replace an inventory host. Returns `true` if it replaced an
    /// existing entry, `false` if it was newly added.
    pub fn upsert_host(&mut self, id: impl Into<String>, host: Host) -> bool {
        self.hosts.insert(id.into(), host).is_some()
    }

    /// Remove an inventory host by id. Returns whether it existed.
    pub fn remove_host(&mut self, id: &str) -> bool {
        self.hosts.remove(id).is_some()
    }

    /// Resolve an `ssh` target. A target matching an inventory id (`[hosts.<id>]`)
    /// uses that profile; otherwise it is parsed as `user@host` or a bare `host`
    /// with default port and no profile overrides.
    pub fn resolve_target(&self, target: &str) -> ResolvedHost {
        if let Some(host) = self.hosts.get(target) {
            return ResolvedHost {
                id: target.to_owned(),
                from_inventory: true,
                host: host.host.clone(),
                user: host.user.clone(),
                port: host.port,
                read_only: host.read_only,
                policy: self.resolve_policy_for_host(host).name().map(str::to_owned),
            };
        }

        let (user, host) = match target.split_once('@') {
            Some((user, host)) if !user.is_empty() && !host.is_empty() => {
                (Some(user.to_owned()), host.to_owned())
            }
            _ => (None, target.to_owned()),
        };
        ResolvedHost {
            id: target.to_owned(),
            from_inventory: false,
            host,
            user,
            port: default_ssh_port(),
            read_only: false,
            policy: None,
        }
    }

    /// Resolve the policy for an inventory host. An explicit host policy always
    /// wins; otherwise policies with `match_tags` are considered. Tag fallback is
    /// deterministic: the most specific policy (most matching tags) wins, with
    /// the policy name as the final tie-breaker.
    pub fn resolve_policy_for_host<'a>(&'a self, host: &'a Host) -> PolicyResolution<'a> {
        if let Some(name) = host.policy.as_deref() {
            return match self.policies.get(name) {
                Some(policy) => PolicyResolution::Matched(PolicyRef {
                    name,
                    policy,
                    source: PolicySource::ExplicitHost,
                }),
                None => PolicyResolution::MissingExplicit { name },
            };
        }

        self.policies
            .iter()
            .filter(|(_, policy)| policy.matches_tags(&host.tags))
            .max_by(|(left_name, left), (right_name, right)| {
                left.match_tags
                    .len()
                    .cmp(&right.match_tags.len())
                    .then_with(|| right_name.cmp(left_name))
            })
            .map_or(PolicyResolution::None, |(name, policy)| {
                PolicyResolution::Matched(PolicyRef {
                    name,
                    policy,
                    source: PolicySource::TagFallback,
                })
            })
    }
}

/// Expected-state policy for a host or group (`Product.md` §4.15, §13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Policy {
    /// Schema version for this policy entry. v0.9 starts at 1.
    #[serde(default = "default_policy_version")]
    pub version: u16,
    /// Tags used as a fallback matcher when a host has no explicit `policy`.
    /// All listed tags must be present on the host.
    pub match_tags: Vec<String>,
    pub expected_ports: Vec<u16>,
    pub forbidden_ports: Vec<u16>,
    pub expected_services: Vec<String>,
    pub forbidden_services: Vec<String>,
    pub disk_warning: Option<u8>,
    pub disk_critical: Option<u8>,
    pub ram_warning: Option<u8>,
    pub load_warning_multiplier: Option<f64>,
    pub expected_sudo_users: Vec<String>,
    pub forbidden_users: Vec<String>,
    pub expected_certs: Vec<ExpectedCertificate>,
    pub expected_containers: Vec<ExpectedContainer>,
    pub forbidden_containers: Vec<String>,
    pub forbidden_images: Vec<String>,
}

impl Policy {
    fn matches_tags(&self, host_tags: &[String]) -> bool {
        !self.match_tags.is_empty()
            && self
                .match_tags
                .iter()
                .all(|wanted| host_tags.iter().any(|tag| tag == wanted))
    }
}

fn default_policy_version() -> u16 {
    1
}

/// Certificate expectation used by the policy evaluator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExpectedCertificate {
    /// Host or endpoint label the certificate should be checked against.
    pub host: String,
    /// Accepted DNS names for the certificate.
    pub names: Vec<String>,
    /// Minimum allowed days remaining before expiry.
    pub min_days_remaining: Option<u32>,
}

/// Container expectation used by the policy evaluator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExpectedContainer {
    pub name: String,
    pub image: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_secure_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.general.default_refresh_seconds, 3);
        assert_eq!(cfg.general.theme, "dark");
        assert!(cfg.general.confirm_dangerous_actions);
        assert_eq!(cfg.thresholds.disk_critical, 90);
        assert_eq!(cfg.security.cert_expiry_warning_days, 30);
        assert!(cfg.hosts.is_empty());
    }

    #[test]
    fn parses_the_product_md_example() {
        let src = r#"
[general]
default_refresh_seconds = 5

[hosts.prod-01]
host = "192.168.1.20"
user = "admin"
port = 22
tags = ["prod", "web"]
read_only = true
policy = "production-web"

[policies.production-web]
expected_ports = [22, 80, 443]
forbidden_ports = [3306, 5432, 6379, 27017]
expected_services = ["sshd", "nginx"]
forbidden_services = ["redis"]
"#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.general.default_refresh_seconds, 5);
        // unspecified general fields keep their defaults
        assert_eq!(cfg.general.theme, "dark");

        let host = &cfg.hosts["prod-01"];
        assert_eq!(host.host, "192.168.1.20");
        assert_eq!(host.user.as_deref(), Some("admin"));
        assert!(host.read_only);
        assert_eq!(host.policy.as_deref(), Some("production-web"));

        let policy = &cfg.policies["production-web"];
        assert_eq!(policy.version, 1);
        assert_eq!(policy.expected_ports, [22, 80, 443]);
        assert!(policy.forbidden_ports.contains(&6379));
        assert_eq!(policy.forbidden_services, ["redis"]);
    }

    #[test]
    fn parses_full_policy_schema() {
        let src = r#"
[policies.production-web]
version = 1
match_tags = ["prod", "web"]
expected_ports = [22, 80, 443]
forbidden_ports = [3306, 5432, 6379, 27017]
expected_services = ["sshd", "nginx"]
forbidden_services = ["redis", "mongodb"]
disk_warning = 80
disk_critical = 90
ram_warning = 85
load_warning_multiplier = 2.0
expected_sudo_users = ["admin"]
forbidden_users = ["old-admin"]
forbidden_containers = ["debug-shell"]
forbidden_images = ["redis:latest"]

[[policies.production-web.expected_certs]]
host = "web.example.com:443"
names = ["web.example.com"]
min_days_remaining = 30

[[policies.production-web.expected_containers]]
name = "nginx"
image = "nginx:1.25"
"#;
        let cfg: Config = toml::from_str(src).unwrap();
        let policy = &cfg.policies["production-web"];

        assert_eq!(policy.match_tags, ["prod", "web"]);
        assert_eq!(policy.disk_warning, Some(80));
        assert_eq!(policy.disk_critical, Some(90));
        assert_eq!(policy.ram_warning, Some(85));
        assert_eq!(policy.load_warning_multiplier, Some(2.0));
        assert_eq!(policy.expected_sudo_users, ["admin"]);
        assert_eq!(policy.forbidden_users, ["old-admin"]);
        assert_eq!(policy.forbidden_containers, ["debug-shell"]);
        assert_eq!(policy.forbidden_images, ["redis:latest"]);
        assert_eq!(policy.expected_certs[0].host, "web.example.com:443");
        assert_eq!(policy.expected_certs[0].names, ["web.example.com"]);
        assert_eq!(policy.expected_certs[0].min_days_remaining, Some(30));
        assert_eq!(policy.expected_containers[0].name, "nginx");
        assert_eq!(
            policy.expected_containers[0].image.as_deref(),
            Some("nginx:1.25")
        );
    }

    #[test]
    fn host_port_defaults_to_22() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.db]
host = "10.0.0.1"
"#,
        )
        .unwrap();
        assert_eq!(cfg.hosts["db"].port, 22);
    }

    #[test]
    fn resolves_an_inventory_host_id() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
user = "admin"
port = 2222
read_only = true
policy = "production-web"
"#,
        )
        .unwrap();
        let resolved = cfg.resolve_target("prod-01");
        assert!(resolved.from_inventory);
        assert_eq!(resolved.host, "192.168.1.20");
        assert_eq!(resolved.user.as_deref(), Some("admin"));
        assert_eq!(resolved.port, 2222);
        assert!(resolved.read_only);
        assert_eq!(resolved.policy.as_deref(), Some("production-web"));
    }

    #[test]
    fn explicit_host_policy_wins_over_tag_fallback() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
tags = ["prod", "web"]
policy = "explicit"

[policies.explicit]
expected_ports = [22]

[policies.tagged]
match_tags = ["prod", "web"]
expected_ports = [443]
"#,
        )
        .unwrap();

        let resolution = cfg.resolve_policy_for_host(&cfg.hosts["prod-01"]);
        let policy = resolution.policy().unwrap();
        assert_eq!(policy.name, "explicit");
        assert_eq!(policy.source, PolicySource::ExplicitHost);
        assert_eq!(policy.policy.expected_ports, [22]);
    }

    #[test]
    fn missing_explicit_policy_is_reported() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
policy = "missing"
"#,
        )
        .unwrap();

        let resolution = cfg.resolve_policy_for_host(&cfg.hosts["prod-01"]);
        assert!(matches!(
            resolution,
            PolicyResolution::MissingExplicit { name: "missing" }
        ));
        assert_eq!(resolution.name(), Some("missing"));
        assert!(resolution.policy().is_none());
    }

    #[test]
    fn tag_fallback_picks_the_most_specific_policy() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
tags = ["prod", "web"]

[policies.a-prod]
match_tags = ["prod"]
expected_ports = [22]

[policies.z-prod-web]
match_tags = ["prod", "web"]
expected_ports = [443]
"#,
        )
        .unwrap();

        let resolution = cfg.resolve_policy_for_host(&cfg.hosts["prod-01"]);
        let policy = resolution.policy().unwrap();
        assert_eq!(policy.name, "z-prod-web");
        assert_eq!(policy.source, PolicySource::TagFallback);
        assert_eq!(policy.policy.expected_ports, [443]);
    }

    #[test]
    fn tag_fallback_ties_break_by_policy_name() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
tags = ["prod"]

[policies.alpha]
match_tags = ["prod"]
expected_ports = [22]

[policies.beta]
match_tags = ["prod"]
expected_ports = [443]
"#,
        )
        .unwrap();

        let resolution = cfg.resolve_policy_for_host(&cfg.hosts["prod-01"]);
        let policy = resolution.policy().unwrap();
        assert_eq!(policy.name, "alpha");
        assert_eq!(policy.policy.expected_ports, [22]);
    }

    #[test]
    fn resolve_target_uses_tag_fallback_policy_name() {
        let cfg: Config = toml::from_str(
            r#"
[hosts.prod-01]
host = "192.168.1.20"
tags = ["prod", "web"]

[policies.production-web]
match_tags = ["prod", "web"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve_target("prod-01");
        assert_eq!(resolved.policy.as_deref(), Some("production-web"));
    }

    #[test]
    fn parses_user_at_host_when_not_in_inventory() {
        let cfg = Config::default();
        let resolved = cfg.resolve_target("admin@10.0.0.5");
        assert!(!resolved.from_inventory);
        assert_eq!(resolved.user.as_deref(), Some("admin"));
        assert_eq!(resolved.host, "10.0.0.5");
        assert_eq!(resolved.port, 22);
        assert!(!resolved.read_only);
    }

    #[test]
    fn parses_bare_host_without_user() {
        let resolved = Config::default().resolve_target("server.example.com");
        assert_eq!(resolved.user, None);
        assert_eq!(resolved.host, "server.example.com");
    }

    #[test]
    fn upsert_and_remove_host() {
        let mut cfg = Config::default();
        let host = Host {
            host: "10.0.0.1".to_owned(),
            user: Some("admin".to_owned()),
            port: 22,
            tags: vec!["web".to_owned()],
            read_only: false,
            favorite: false,
            policy: None,
        };
        // First insert is new.
        assert!(!cfg.upsert_host("prod-01", host.clone()));
        assert_eq!(cfg.hosts["prod-01"].host, "10.0.0.1");
        // Re-inserting the same id replaces.
        let mut updated = host;
        updated.read_only = true;
        assert!(cfg.upsert_host("prod-01", updated));
        assert!(cfg.hosts["prod-01"].read_only);
        // Remove reports prior existence.
        assert!(cfg.remove_host("prod-01"));
        assert!(!cfg.remove_host("prod-01"));
        assert!(cfg.hosts.is_empty());
    }
}
