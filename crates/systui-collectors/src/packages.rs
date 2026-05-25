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
            let (pending, security) = parse_apt(&out);
            return Ok(found("apt", pending, security));
        }
        if let Some(out) = try_run(
            transport,
            "dnf",
            &["--cacheonly", "--quiet", "check-update"],
        )
        .await
        {
            return Ok(found("dnf", parse_dnf(&out), 0));
        }
        if let Some(out) = try_run(transport, "pacman", &["-Qu"]).await {
            return Ok(found("pacman", parse_line_count(&out), 0));
        }
        if let Some(out) = try_run(transport, "zypper", &["--quiet", "list-updates"]).await {
            return Ok(found("zypper", parse_zypper(&out), 0));
        }
        Ok(PackageUpdates {
            manager: "none".to_owned(),
            ..PackageUpdates::default()
        })
    }
}

fn found(manager: &str, pending: usize, security: usize) -> PackageUpdates {
    PackageUpdates {
        manager: manager.to_owned(),
        pending,
        security,
        available: true,
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

/// `apt list --upgradable` → `(pending, security)`. Each upgradable line looks
/// like `nginx/jammy-updates 1.2 amd64 [upgradable from: 1.1]`; security updates
/// come from a `*-security` pocket. The `Listing…` header is skipped.
fn parse_apt(s: &str) -> (usize, usize) {
    let mut pending = 0;
    let mut security = 0;
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains('/') || line.starts_with("Listing") {
            continue;
        }
        pending += 1;
        // The pocket is the part after the first '/', before whitespace.
        if let Some((_, rest)) = line.split_once('/')
            && rest
                .split_whitespace()
                .next()
                .is_some_and(|pocket| pocket.contains("-security"))
        {
            security += 1;
        }
    }
    (pending, security)
}

/// `dnf check-update` lists `name.arch  version  repo` lines after an optional
/// blank line; sections like `Obsoleting Packages` are ignored. Count lines
/// whose first token looks like `name.arch`.
fn parse_dnf(s: &str) -> usize {
    s.lines()
        .filter(|line| {
            let mut tokens = line.split_whitespace();
            match (tokens.next(), tokens.next(), tokens.next()) {
                (Some(name), Some(_ver), Some(_repo)) => {
                    name.contains('.') && !line.starts_with(' ')
                }
                _ => false,
            }
        })
        .count()
}

/// `pacman -Qu` / similar: one upgradable package per non-empty line.
fn parse_line_count(s: &str) -> usize {
    s.lines().filter(|l| !l.trim().is_empty()).count()
}

/// `zypper list-updates` prints a table; package rows start with `v |`.
fn parse_zypper(s: &str) -> usize {
    s.lines()
        .filter(|l| l.trim_start().starts_with("v |"))
        .count()
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
        let (pending, security) = parse_apt(out);
        assert_eq!(pending, 3);
        assert_eq!(security, 2);
    }

    #[test]
    fn parses_dnf_count() {
        let out = "\nkernel.x86_64      5.14.0-70.el9    baseos\n\
            nginx.x86_64       1.20.1-1.el9     appstream\n";
        assert_eq!(parse_dnf(out), 2);
    }

    #[test]
    fn parses_pacman_count() {
        let out = "linux 6.1.1-1 -> 6.1.2-1\nvim 9.0.1-1 -> 9.0.2-1\n";
        assert_eq!(parse_line_count(out), 2);
    }

    #[test]
    fn parses_zypper_count() {
        let out = "S | Repository | Name | Current | Available | Arch\n\
            --+------------+------+---------+-----------+-----\n\
            v | repo-oss   | curl | 7.1     | 7.2       | x86_64\n";
        assert_eq!(parse_zypper(out), 1);
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
