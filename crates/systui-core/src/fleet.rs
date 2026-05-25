//! Fleet selection: turning the host inventory into a filtered, ordered set of
//! hosts to review (`Product.md` §4.16, §8 v0.8).
//!
//! v0.8 fleet mode is **inspection and reporting only** — this module just decides
//! *which* inventory hosts a fleet run targets. Gathering their state concurrently
//! over [`crate::Transport`] is a later session; here there is no I/O, so selection
//! is pure and easy to test.

use crate::config::{Config, Host};

/// A host chosen for a fleet run, drawn from the `[hosts.<id>]` inventory.
///
/// It carries everything needed to connect and label the host later, so the
/// concurrent-gather step does not have to revisit the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetHost {
    /// Inventory id (the `[hosts.<id>]` key).
    pub id: String,
    pub host: String,
    pub user: Option<String>,
    pub port: u16,
    pub tags: Vec<String>,
    pub read_only: bool,
    pub favorite: bool,
    pub policy: Option<String>,
}

/// Criteria for choosing hosts from the inventory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FleetFilter {
    /// Keep a host if it carries *any* of these tags (OR semantics). An empty
    /// list applies no tag filter.
    pub tags: Vec<String>,
    /// Keep only hosts flagged as favorites.
    pub favorites_only: bool,
}

impl FleetFilter {
    /// A filter that selects the whole inventory.
    pub fn all() -> Self {
        Self::default()
    }

    /// Whether `host` satisfies this filter.
    fn matches(&self, host: &Host) -> bool {
        if self.favorites_only && !host.favorite {
            return false;
        }
        if self.tags.is_empty() {
            return true;
        }
        self.tags.iter().any(|wanted| host.tags.contains(wanted))
    }
}

impl Config {
    /// Select the inventory hosts matching `filter`, ordered deterministically:
    /// favorites first, then by id. The stable ordering keeps fleet output
    /// reproducible and diff-friendly, matching the per-host report contract.
    pub fn select_fleet(&self, filter: &FleetFilter) -> Vec<FleetHost> {
        let mut selected: Vec<FleetHost> = self
            .hosts
            .iter()
            .filter(|(_, host)| filter.matches(host))
            .map(|(id, host)| FleetHost {
                id: id.clone(),
                host: host.host.clone(),
                user: host.user.clone(),
                port: host.port,
                tags: host.tags.clone(),
                read_only: host.read_only,
                favorite: host.favorite,
                policy: self.resolve_policy_for_host(host).name().map(str::to_owned),
            })
            .collect();
        // `hosts` is a BTreeMap, so iteration is already id-sorted; a stable sort
        // by `!favorite` floats favorites to the top while preserving id order
        // within each group.
        selected.sort_by(|a, b| b.favorite.cmp(&a.favorite).then_with(|| a.id.cmp(&b.id)));
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Config {
        let src = r#"
[hosts.prod-01]
host = "10.0.0.1"
tags = ["prod", "web"]

[hosts.prod-02]
host = "10.0.0.2"
tags = ["prod", "db"]
favorite = true

[hosts.staging-01]
host = "10.0.1.1"
tags = ["staging", "web"]
"#;
        toml::from_str(src).unwrap()
    }

    #[test]
    fn empty_filter_selects_every_host() {
        let cfg = fixture();
        let ids: Vec<_> = cfg
            .select_fleet(&FleetFilter::all())
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids.len(), 3);
        // prod-02 is a favorite, so it floats to the top; the rest stay id-sorted.
        assert_eq!(ids, ["prod-02", "prod-01", "staging-01"]);
    }

    #[test]
    fn tag_filter_is_or_across_tags() {
        let cfg = fixture();
        let filter = FleetFilter {
            tags: vec!["db".to_owned(), "staging".to_owned()],
            favorites_only: false,
        };
        let ids: Vec<_> = cfg
            .select_fleet(&filter)
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, ["prod-02", "staging-01"]);
    }

    #[test]
    fn favorites_only_drops_non_favorites() {
        let cfg = fixture();
        let filter = FleetFilter {
            tags: Vec::new(),
            favorites_only: true,
        };
        let selected = cfg.select_fleet(&filter);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "prod-02");
        assert!(selected[0].favorite);
    }

    #[test]
    fn tag_and_favorites_filters_combine() {
        let cfg = fixture();
        // `web` matches prod-01 and staging-01, but neither is a favorite.
        let filter = FleetFilter {
            tags: vec!["web".to_owned()],
            favorites_only: true,
        };
        assert!(cfg.select_fleet(&filter).is_empty());
    }

    #[test]
    fn unknown_tag_selects_nothing() {
        let cfg = fixture();
        let filter = FleetFilter {
            tags: vec!["edge".to_owned()],
            favorites_only: false,
        };
        assert!(cfg.select_fleet(&filter).is_empty());
    }

    #[test]
    fn fleet_host_carries_connection_details() {
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
        let selected = cfg.select_fleet(&FleetFilter::all());
        let host = &selected[0];
        assert_eq!(host.host, "192.168.1.20");
        assert_eq!(host.user.as_deref(), Some("admin"));
        assert_eq!(host.port, 2222);
        assert!(host.read_only);
        assert_eq!(host.policy.as_deref(), Some("production-web"));
    }

    #[test]
    fn fleet_host_carries_tag_fallback_policy() {
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

        let selected = cfg.select_fleet(&FleetFilter::all());
        assert_eq!(selected[0].policy.as_deref(), Some("production-web"));
    }
}
