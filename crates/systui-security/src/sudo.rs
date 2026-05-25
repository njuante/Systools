//! Sudo posture checks: who can elevate to root and whether any grant is
//! passwordless. Parses `/etc/sudoers` and `/etc/sudoers.d/*` content.

use systui_core::{Finding, ModuleId, Severity};

/// A privilege grant parsed from a sudoers file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SudoEntry {
    /// The user, or group name (without the leading `%`) when `is_group`.
    pub principal: String,
    pub is_group: bool,
    /// Whether the grant allows running commands without a password.
    pub nopasswd: bool,
}

/// Parse user/group privilege specifications from sudoers content. `Defaults`,
/// comments, `#includedir` directives and blank lines are ignored.
pub fn parse_sudoers(text: &str) -> Vec<SudoEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("Defaults") {
            continue;
        }
        // A spec is `principal host=(runas) commands`; require the `ALL=` shape.
        let Some((principal, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !rest.contains('=') {
            continue;
        }
        let is_group = principal.starts_with('%');
        entries.push(SudoEntry {
            principal: principal.trim_start_matches('%').to_owned(),
            is_group,
            nopasswd: rest.contains("NOPASSWD"),
        });
    }
    entries
}

/// Findings from sudo grants: an informational inventory of who can elevate,
/// and a medium-severity finding if any grant is passwordless.
pub fn check_sudo(entries: &[SudoEntry]) -> Vec<Finding> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut findings = Vec::new();

    let principals: Vec<String> = entries
        .iter()
        .map(|e| {
            if e.is_group {
                format!("group {}", e.principal)
            } else {
                e.principal.clone()
            }
        })
        .collect();
    findings.push(
        Finding::new(
            "sudo.grants",
            Severity::Info,
            ModuleId::Security,
            "Accounts with sudo privileges",
        )
        .evidence(
            principals
                .iter()
                .map(|p| format!("sudoers grants root to {p}")),
        )
        .impact("These principals can execute commands as root.")
        .recommendation("Confirm each grant is intended and remove unused ones."),
    );

    let nopasswd: Vec<&SudoEntry> = entries.iter().filter(|e| e.nopasswd).collect();
    if !nopasswd.is_empty() {
        findings.push(
            Finding::new(
                "sudo.nopasswd",
                Severity::Medium,
                ModuleId::Security,
                "Passwordless sudo is configured",
            )
            .evidence(nopasswd.iter().map(|e| {
                let kind = if e.is_group { "group " } else { "" };
                format!("NOPASSWD grant for {kind}{}", e.principal)
            }))
            .impact("A compromised account can become root without re-authenticating.")
            .recommendation(
                "Require a password for sudo unless a specific automation needs NOPASSWD.",
            ),
        );
    }

    findings
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use systui_testkit::fuzz::messy_output;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn sudoers_parser_never_panics(s in messy_output()) {
            let _ = parse_sudoers(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_users_and_groups() {
        let entries = parse_sudoers(include_str!("../fixtures/sudoers.txt"));
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().any(|e| e.principal == "root" && !e.is_group));
        assert!(entries.iter().any(|e| e.principal == "admin" && e.is_group));
        assert!(entries.iter().any(|e| e.principal == "sudo" && e.is_group));
        assert!(
            entries
                .iter()
                .any(|e| e.principal == "deploy" && e.nopasswd)
        );
    }

    #[test]
    fn reports_grants_and_nopasswd() {
        let entries = parse_sudoers(include_str!("../fixtures/sudoers.txt"));
        let findings = check_sudo(&entries);
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"sudo.grants"));
        assert!(ids.contains(&"sudo.nopasswd"));
        let nopasswd = findings.iter().find(|f| f.id == "sudo.nopasswd").unwrap();
        assert_eq!(nopasswd.severity, Severity::Medium);
    }

    #[test]
    fn empty_sudoers_yields_nothing() {
        assert!(check_sudo(&parse_sudoers("Defaults env_reset\n# comment\n")).is_empty());
    }
}
