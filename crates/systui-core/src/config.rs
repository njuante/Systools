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
    pub confirm_dangerous_actions: bool,
    pub audit_log: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_refresh_seconds: 3,
            theme: "dark".to_owned(),
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
    /// Name of the policy to evaluate this host against.
    #[serde(default)]
    pub policy: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

/// Expected-state policy for a host or group (`Product.md` §4.15, §13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Policy {
    pub expected_ports: Vec<u16>,
    pub forbidden_ports: Vec<u16>,
    pub expected_services: Vec<String>,
    pub forbidden_services: Vec<String>,
    pub disk_warning: Option<u8>,
    pub disk_critical: Option<u8>,
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
        assert_eq!(policy.expected_ports, [22, 80, 443]);
        assert!(policy.forbidden_ports.contains(&6379));
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
}
