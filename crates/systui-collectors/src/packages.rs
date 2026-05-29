//! Pending package updates across the common Linux package managers
//! (apt / dnf / pacman / zypper).
//!
//! Read-only and **cache-only**: every probe reads the package manager's local
//! metadata and never triggers a network refresh or takes a packaging lock
//! (`apt list --upgradable`, `dnf --cacheonly check-update`, `pacman -Qu`,
//! `zypper list-updates`). If no manager is present the snapshot is "none" with
//! zero counts. Security-update counts are only reported where they can be read
//! cheaply (apt's `-security` pockets); elsewhere the count stays 0 rather than
//! guessing.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A single upgradable package, as parsed from the manager's cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageUpdate {
    pub name: String,
    /// Installed version, when the manager reports it (empty otherwise).
    pub current: String,
    /// Candidate (new) version.
    pub candidate: String,
    /// Whether this update comes from a security pocket (apt only; else false).
    pub security: bool,
}

/// A summary of pending package updates.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageUpdates {
    /// The detected manager: `apt`, `dnf`, `pacman`, `zypper`, or `none`.
    pub manager: String,
    /// Total upgradable packages.
    pub pending: usize,
    /// Of those, updates from a security pocket (best-effort; 0 when unknown).
    pub security: usize,
    /// Whether a known package manager was detected.
    pub available: bool,
    /// The upgradable packages (best-effort; empty when only a count is known).
    #[serde(default)]
    pub packages: Vec<PackageUpdate>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Reads pending updates from whichever package manager is present.
#[derive(Debug, Default, Clone, Copy)]
pub struct PackagesCollector;

impl PackagesCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for PackagesCollector {
    type Output = PackageUpdates;

    fn module(&self) -> ModuleId {
        ModuleId::System
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<PackageUpdates> {
        // Try each manager's cache-only query in turn; the first whose binary
        // exists (the command spawns) decides the manager. A non-zero exit is
        // fine (dnf returns 100 when updates exist) — only a spawn failure means
        // "not this manager".
        if let Some(out) = try_run(transport, "apt", &["list", "--upgradable"]).await {
            return Ok(found("apt", parse_apt(&out)));
        }
        if let Some(out) = try_run(
            transport,
            "dnf",
            &["--cacheonly", "--quiet", "check-update"],
        )
        .await
        {
            return Ok(found("dnf", parse_dnf(&out)));
        }
        if let Some(out) = try_run(transport, "pacman", &["-Qu"]).await {
            return Ok(found("pacman", parse_pacman(&out)));
        }
        if let Some(out) = try_run(transport, "zypper", &["--quiet", "list-updates"]).await {
            return Ok(found("zypper", parse_zypper(&out)));
        }
        Ok(PackageUpdates {
            manager: "none".to_owned(),
            ..PackageUpdates::default()
        })
    }
}

fn found(manager: &str, packages: Vec<PackageUpdate>) -> PackageUpdates {
    let security = packages.iter().filter(|p| p.security).count();
    PackageUpdates {
        manager: manager.to_owned(),
        pending: packages.len(),
        security,
        available: true,
        packages,
    }
}

/// Run a probe; `Some(stdout)` if the binary exists (any exit code), `None` if
/// it could not be spawned (manager absent) or timed out.
async fn try_run(transport: &dyn Transport, program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program)
        .args(args.iter().copied())
        .timeout(PROBE_TIMEOUT);
    transport.run(&spec).await.ok().map(|out| out.stdout)
}

/// `apt list --upgradable` lines look like
/// `nginx/jammy-updates 1.2 amd64 [upgradable from: 1.1]`; security updates come
/// from a `*-security` pocket. The `Listing…` header is skipped.
fn parse_apt(s: &str) -> Vec<PackageUpdate> {
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains('/') || line.starts_with("Listing") {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(name_pocket) = tokens.next() else {
            continue;
        };
        let (name, pocket) = name_pocket.split_once('/').unwrap_or((name_pocket, ""));
        let candidate = tokens.next().unwrap_or("").to_owned();
        // `[upgradable from: 1.1]` → current version is the trailing token.
        let current = line
            .rsplit_once(':')
            .map(|(_, v)| v.trim().trim_end_matches(']').trim().to_owned())
            .unwrap_or_default();
        out.push(PackageUpdate {
            name: name.to_owned(),
            current,
            candidate,
            security: pocket.contains("-security"),
        });
    }
    out
}

/// `dnf check-update` lists `name.arch  version  repo` lines after an optional
/// blank line; sections like `Obsoleting Packages` are ignored.
fn parse_dnf(s: &str) -> Vec<PackageUpdate> {
    s.lines()
        .filter_map(|line| {
            if line.starts_with(' ') {
                return None;
            }
            let mut tokens = line.split_whitespace();
            match (tokens.next(), tokens.next(), tokens.next()) {
                (Some(name), Some(ver), Some(_repo)) if name.contains('.') => Some(PackageUpdate {
                    name: name.to_owned(),
                    current: String::new(),
                    candidate: ver.to_owned(),
                    security: false,
                }),
                _ => None,
            }
        })
        .collect()
}

/// `pacman -Qu`: `name current -> candidate` per non-empty line.
fn parse_pacman(s: &str) -> Vec<PackageUpdate> {
    s.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            let tokens: Vec<&str> = l.split_whitespace().collect();
            let candidate = tokens
                .iter()
                .position(|t| *t == "->")
                .and_then(|i| tokens.get(i + 1))
                .map(|s| (*s).to_owned())
                .unwrap_or_default();
            Some(PackageUpdate {
                name: tokens[0].to_owned(),
                current: tokens.get(1).map(|s| (*s).to_owned()).unwrap_or_default(),
                candidate,
                security: false,
            })
        })
        .collect()
}

/// `zypper list-updates` prints a table; package rows start with `v |` and have
/// `v | repo | name | current | available | arch` columns.
fn parse_zypper(s: &str) -> Vec<PackageUpdate> {
    s.lines()
        .filter(|l| l.trim_start().starts_with("v |"))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split('|').map(str::trim).collect();
            cols.get(2).map(|name| PackageUpdate {
                name: (*name).to_owned(),
                current: cols.get(3).map(|s| (*s).to_owned()).unwrap_or_default(),
                candidate: cols.get(4).map(|s| (*s).to_owned()).unwrap_or_default(),
                security: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use systui_testkit::fuzz::messy_output;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn package_parsers_never_panic(s in messy_output()) {
            let _ = parse_apt(&s);
            let _ = parse_dnf(&s);
            let _ = parse_pacman(&s);
            let _ = parse_zypper(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apt_pending_and_security() {
        let out = "Listing...\n\
            nginx/jammy-updates 1.2 amd64 [upgradable from: 1.1]\n\
            openssl/jammy-security 3.0.2 amd64 [upgradable from: 3.0.1]\n\
            libc6/jammy-security 2.35 amd64 [upgradable from: 2.34]\n";
        let pkgs = parse_apt(out);
        assert_eq!(pkgs.len(), 3);
        assert_eq!(pkgs.iter().filter(|p| p.security).count(), 2);
        assert_eq!(pkgs[0].name, "nginx");
        assert_eq!(pkgs[0].candidate, "1.2");
        assert_eq!(pkgs[0].current, "1.1");
        assert!(pkgs[1].security);
    }

    #[test]
    fn parses_dnf_count() {
        let out = "\nkernel.x86_64      5.14.0-70.el9    baseos\n\
            nginx.x86_64       1.20.1-1.el9     appstream\n";
        let pkgs = parse_dnf(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "kernel.x86_64");
        assert_eq!(pkgs[0].candidate, "5.14.0-70.el9");
    }

    #[test]
    fn parses_pacman_count() {
        let out = "linux 6.1.1-1 -> 6.1.2-1\nvim 9.0.1-1 -> 9.0.2-1\n";
        let pkgs = parse_pacman(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "linux");
        assert_eq!(pkgs[0].current, "6.1.1-1");
        assert_eq!(pkgs[0].candidate, "6.1.2-1");
    }

    #[test]
    fn parses_zypper_count() {
        let out = "S | Repository | Name | Current | Available | Arch\n\
            --+------------+------+---------+-----------+-----\n\
            v | repo-oss   | curl | 7.1     | 7.2       | x86_64\n";
        let pkgs = parse_zypper(out);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "curl");
        assert_eq!(pkgs[0].candidate, "7.2");
    }

    #[tokio::test]
    async fn no_manager_degrades_to_none() {
        use systui_transport::MockTransport;
        let updates = PackagesCollector::new()
            .collect(&MockTransport::new())
            .await
            .unwrap();
        assert_eq!(updates.manager, "none");
        assert!(!updates.available);
        assert_eq!(updates.pending, 0);
    }
}
