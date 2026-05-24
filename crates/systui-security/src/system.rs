//! Host-hardening checks: firewall presence, permissions on critical files,
//! docker socket exposure and unexpected SUID binaries. The orchestrator feeds
//! these pure functions the command/file output it gathered via the transport.

use systui_core::{Finding, ModuleId, Severity};

// --- firewall --------------------------------------------------------------

/// Captured firewall-probe outputs (each `None` when the tool is absent or the
/// probe failed). The presence logic is pure so it is fixture-testable.
#[derive(Debug, Clone, Default)]
pub struct FirewallProbes {
    pub ufw_status: Option<String>,
    pub firewalld_state: Option<String>,
    pub nft_ruleset: Option<String>,
    pub iptables_save: Option<String>,
}

/// Decide whether a firewall is active and, if so, which tool reports it.
pub fn detect_firewall(probes: &FirewallProbes) -> Option<&'static str> {
    if probes
        .ufw_status
        .as_deref()
        .is_some_and(|s| s.to_ascii_lowercase().contains("status: active"))
    {
        return Some("ufw");
    }
    if probes
        .firewalld_state
        .as_deref()
        .is_some_and(|s| s.trim() == "running")
    {
        return Some("firewalld");
    }
    if probes
        .nft_ruleset
        .as_deref()
        .is_some_and(|s| s.contains("chain") && s.contains('{'))
    {
        return Some("nftables");
    }
    if probes
        .iptables_save
        .as_deref()
        .is_some_and(iptables_has_rules)
    {
        return Some("iptables");
    }
    None
}

/// `iptables-save`/`iptables -S` shows custom rules beyond the default
/// accept-all policy lines (`-P CHAIN ACCEPT`).
fn iptables_has_rules(output: &str) -> bool {
    output.lines().any(|l| {
        let l = l.trim();
        (l.starts_with("-A") || l.starts_with("-I"))
            || (l.starts_with("-P") && !l.ends_with("ACCEPT"))
    })
}

/// A finding when no active firewall was detected.
pub fn check_firewall(detected: Option<&str>) -> Option<Finding> {
    if detected.is_some() {
        return None;
    }
    Some(
        Finding::new(
            "firewall.absent",
            Severity::Medium,
            ModuleId::Firewall,
            "No active firewall detected",
        )
        .with_evidence("ufw, firewalld, nftables and iptables show no active ruleset")
        .impact("All listening services are reachable from any network that can route to the host.")
        .recommendation(
            "Enable a host firewall (ufw/firewalld/nftables) and allow only required ports.",
        ),
    )
}

// --- file permissions ------------------------------------------------------

/// One line of `stat -c '%a %U %G %n'` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatInfo {
    pub mode: u32,
    pub owner: String,
    pub group: String,
    pub path: String,
}

/// Parse `stat -c '%a %U %G %n'` output into [`StatInfo`] rows.
pub fn parse_stat(output: &str) -> Vec<StatInfo> {
    output.lines().filter_map(parse_stat_line).collect()
}

fn parse_stat_line(line: &str) -> Option<StatInfo> {
    let mut parts = line.trim().splitn(4, char::is_whitespace);
    let mode = u32::from_str_radix(parts.next()?, 8).ok()?;
    Some(StatInfo {
        mode,
        owner: parts.next()?.to_owned(),
        group: parts.next()?.to_owned(),
        path: parts.next()?.to_owned(),
    })
}

/// Findings from the permissions of critical files: world-writable files and a
/// world/group-readable `/etc/shadow`.
pub fn check_file_perms(stats: &[StatInfo]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for stat in stats {
        if stat.mode & 0o002 != 0 {
            findings.push(
                Finding::new(
                    format!("fs.world-writable.{}", stat.path),
                    Severity::High,
                    ModuleId::Security,
                    "World-writable critical file",
                )
                .with_evidence(format!(
                    "{} is mode {:o} (writable by any user)",
                    stat.path, stat.mode
                ))
                .impact("Any local user can modify this file, potentially escalating privileges.")
                .recommendation(format!(
                    "Remove world-write, e.g. `chmod o-w {}`.",
                    stat.path
                )),
            );
        }
        if stat.path == "/etc/shadow" && stat.mode & 0o044 != 0 {
            findings.push(
                Finding::new(
                    "fs.shadow-readable",
                    Severity::High,
                    ModuleId::Security,
                    "/etc/shadow is readable beyond root",
                )
                .with_evidence(format!("/etc/shadow is mode {:o}", stat.mode))
                .impact("Password hashes can be read and cracked offline.")
                .recommendation("Restrict to root, e.g. `chmod 600 /etc/shadow`."),
            );
        }
    }
    findings
}

// --- docker socket ---------------------------------------------------------

/// A finding for an exposed docker socket. World-writable is critical (root for
/// anyone); otherwise its mere presence is noted, since the docker group is
/// root-equivalent.
pub fn check_docker_socket(stat: Option<&StatInfo>) -> Option<Finding> {
    let stat = stat?;
    if stat.mode & 0o002 != 0 {
        Some(
            Finding::new(
                "docker.socket-world-writable",
                Severity::Critical,
                ModuleId::Security,
                "Docker socket is world-writable",
            )
            .with_evidence(format!("{} is mode {:o}", stat.path, stat.mode))
            .impact("Any local user can control Docker and trivially gain root on the host.")
            .recommendation("Restrict the socket to root:docker (mode 660)."),
        )
    } else {
        Some(
            Finding::new(
                "docker.socket-present",
                Severity::Info,
                ModuleId::Security,
                "Docker socket present (group has root-equivalent access)",
            )
            .with_evidence(format!(
                "{} owned by {}:{} mode {:o}",
                stat.path, stat.owner, stat.group, stat.mode
            ))
            .impact("Members of the docker group can gain root through the daemon.")
            .recommendation("Keep docker group membership minimal and audited."),
        )
    }
}

// --- SUID binaries ---------------------------------------------------------

/// Baseline of SUID binaries that are expected on a typical Linux host.
const SUID_ALLOWLIST: &[&str] = &[
    "su",
    "sudo",
    "sudoedit",
    "passwd",
    "chsh",
    "chfn",
    "newgrp",
    "gpasswd",
    "mount",
    "umount",
    "ping",
    "ping6",
    "pkexec",
    "fusermount",
    "fusermount3",
    "ssh-keysign",
    "dbus-daemon-launch-helper",
    "polkit-agent-helper-1",
    "unix_chkpwd",
    "chage",
    "expiry",
    "at",
    "crontab",
];

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A finding listing SUID binaries that are not in the expected baseline.
pub fn check_suid(paths: &[String]) -> Option<Finding> {
    let unexpected: Vec<&String> = paths
        .iter()
        .filter(|p| !SUID_ALLOWLIST.contains(&basename(p)))
        .collect();
    if unexpected.is_empty() {
        return None;
    }
    Some(
        Finding::new(
            "suid.unexpected",
            Severity::Low,
            ModuleId::Security,
            "Unexpected SUID binaries",
        )
        .evidence(
            unexpected
                .iter()
                .map(|p| format!("SUID binary not in baseline: {p}")),
        )
        .impact("A vulnerable or misconfigured SUID binary can be abused for privilege escalation.")
        .recommendation("Review each binary; remove the SUID bit where it is not required."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_active_ufw() {
        let probes = FirewallProbes {
            ufw_status: Some("Status: active\n".to_owned()),
            ..Default::default()
        };
        assert_eq!(detect_firewall(&probes), Some("ufw"));
        assert!(check_firewall(detect_firewall(&probes)).is_none());
    }

    #[test]
    fn detects_iptables_rules_but_not_default_policy() {
        let default = "-P INPUT ACCEPT\n-P FORWARD ACCEPT\n-P OUTPUT ACCEPT\n";
        let with_rules = "-P INPUT DROP\n-A INPUT -p tcp --dport 22 -j ACCEPT\n";
        assert!(!iptables_has_rules(default));
        assert!(iptables_has_rules(with_rules));
    }

    #[test]
    fn no_firewall_is_medium_finding() {
        let finding = check_firewall(detect_firewall(&FirewallProbes::default())).unwrap();
        assert_eq!(finding.id, "firewall.absent");
        assert_eq!(finding.severity, Severity::Medium);
        assert_eq!(finding.module, ModuleId::Firewall);
    }

    #[test]
    fn flags_world_writable_and_shadow() {
        let stats = parse_stat(include_str!("../fixtures/stat-critical.txt"));
        assert_eq!(stats.len(), 4);
        let findings = check_file_perms(&stats);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"fs.shadow-readable"));
        assert!(ids.contains(&"fs.world-writable./etc/cron.allow"));
        assert!(findings.iter().all(|f| f.severity == Severity::High));
    }

    #[test]
    fn docker_socket_severity_depends_on_mode() {
        let world = StatInfo {
            mode: 0o666,
            owner: "root".into(),
            group: "root".into(),
            path: "/var/run/docker.sock".into(),
        };
        assert_eq!(
            check_docker_socket(Some(&world)).unwrap().severity,
            Severity::Critical
        );
        let normal = StatInfo {
            mode: 0o660,
            owner: "root".into(),
            group: "docker".into(),
            path: "/var/run/docker.sock".into(),
        };
        assert_eq!(
            check_docker_socket(Some(&normal)).unwrap().severity,
            Severity::Info
        );
        assert!(check_docker_socket(None).is_none());
    }

    #[test]
    fn flags_only_unexpected_suid() {
        let paths: Vec<String> = include_str!("../fixtures/suid-find.txt")
            .lines()
            .map(str::to_owned)
            .collect();
        let finding = check_suid(&paths).unwrap();
        assert_eq!(finding.evidence.len(), 2);
        assert!(finding.evidence.iter().any(|e| e.contains("backup-helper")));
        assert!(finding.evidence.iter().any(|e| e.contains("runner")));
    }

    #[test]
    fn all_allowlisted_suid_is_clean() {
        let paths = vec!["/usr/bin/sudo".to_owned(), "/bin/mount".to_owned()];
        assert!(check_suid(&paths).is_none());
    }
}
