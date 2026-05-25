//! SSH posture checks over `sshd_config` and the auth log: root login policy,
//! password authentication, and the volume of recent failed logins. All checks
//! are pure functions over already-read text so they are fixture-testable.

use systui_core::{Finding, ModuleId, Severity};

/// First value for `key` in an `sshd_config` (OpenSSH uses the first match),
/// matched case-insensitively. Comments and blank lines are skipped.
fn sshd_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let found = parts.next()?;
        if found.eq_ignore_ascii_case(key) {
            return parts.next().map(str::trim);
        }
    }
    None
}

/// Findings derived from `sshd_config`: root login and password authentication.
pub fn check_sshd_config(config: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    match sshd_value(config, "PermitRootLogin").map(str::to_ascii_lowercase) {
        Some(value) if value == "yes" => findings.push(
            Finding::new(
                "ssh.root-login",
                Severity::High,
                ModuleId::Security,
                "SSH permits direct root login",
            )
            .with_evidence(format!("/etc/ssh/sshd_config: PermitRootLogin {value}"))
            .impact("Lets an attacker brute-force the root account directly over SSH.")
            .recommendation(
                "Set PermitRootLogin to `no` (or `prohibit-password`) and use an \
                 unprivileged account with sudo.",
            ),
        ),
        _ => {}
    }

    match sshd_value(config, "PasswordAuthentication").map(str::to_ascii_lowercase) {
        Some(value) if value == "yes" => findings.push(
            Finding::new(
                "ssh.password-auth",
                Severity::High,
                ModuleId::Security,
                "SSH password authentication is enabled",
            )
            .with_evidence("/etc/ssh/sshd_config: PasswordAuthentication yes")
            .impact("Allows online brute-force attempts when SSH is exposed.")
            .recommendation("Disable password auth and use key-based login."),
        ),
        None => findings.push(
            Finding::new(
                "ssh.password-auth-default",
                Severity::Low,
                ModuleId::Security,
                "SSH password authentication is not explicitly disabled",
            )
            .with_evidence(
                "/etc/ssh/sshd_config: PasswordAuthentication is unset (OpenSSH defaults to yes)",
            )
            .impact("Password logins remain possible by default if SSH is exposed.")
            .recommendation("Explicitly set PasswordAuthentication no and rely on keys."),
        ),
        _ => {}
    }

    findings
}

/// Count failed SSH authentication attempts in an auth log / journal excerpt.
pub fn count_failed_logins(log: &str) -> usize {
    log.lines()
        .filter(|l| l.contains("Failed password") || l.contains("authentication failure"))
        .count()
}

/// A finding summarising recent failed SSH logins, if any. Severity scales with
/// volume: a handful is informational, a flood suggests an active brute-force.
pub fn check_failed_logins(count: usize) -> Option<Finding> {
    if count == 0 {
        return None;
    }
    let severity = match count {
        0..=9 => Severity::Info,
        10..=49 => Severity::Low,
        _ => Severity::Medium,
    };
    Some(
        Finding::new(
            "ssh.failed-logins",
            severity,
            ModuleId::Security,
            "Recent failed SSH login attempts",
        )
        .with_evidence(format!("{count} failed SSH authentication attempts in the inspected log"))
        .impact("A high volume indicates an ongoing brute-force or credential-stuffing attempt.")
        .recommendation(
            "Restrict SSH exposure, enforce key-based auth, and consider fail2ban or rate limiting.",
        ),
    )
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use systui_testkit::fuzz::messy_output;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn ssh_parsers_never_panic(s in messy_output()) {
            let _ = check_sshd_config(&s);
            let _ = count_failed_logins(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_insecure_sshd_config() {
        let findings = check_sshd_config(include_str!("../fixtures/sshd_config-insecure.txt"));
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"ssh.root-login"));
        assert!(ids.contains(&"ssh.password-auth"));
        assert!(
            findings
                .iter()
                .all(|f| f.severity == Severity::High && !f.recommendation.is_empty())
        );
    }

    #[test]
    fn hardened_config_only_passes() {
        let findings = check_sshd_config(include_str!("../fixtures/sshd_config-hardened.txt"));
        assert!(findings.is_empty());
    }

    #[test]
    fn unset_password_auth_is_low() {
        let findings = check_sshd_config("PermitRootLogin no\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "ssh.password-auth-default");
        assert_eq!(findings[0].severity, Severity::Low);
    }

    #[test]
    fn counts_failed_logins_and_scales_severity() {
        let count = count_failed_logins(include_str!("../fixtures/auth-failed.txt"));
        assert_eq!(count, 4);
        assert_eq!(check_failed_logins(count).unwrap().severity, Severity::Info);
        assert_eq!(check_failed_logins(25).unwrap().severity, Severity::Low);
        assert_eq!(check_failed_logins(100).unwrap().severity, Severity::Medium);
        assert!(check_failed_logins(0).is_none());
    }
}
