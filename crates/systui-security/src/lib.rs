//! SysTUI security: posture checks (SSH, sudo, firewall, file perms, docker
//! socket, SUID, network exposure) producing prioritized, evidence-based
//! [`Finding`]s with severities (`Product.md` §4.10, §7.4, §9).
//!
//! Everything here is read-only: checks gather command output and file contents
//! through a [`Transport`] and feed them to pure, fixture-tested analysis
//! functions. Missing tools or denied permissions degrade to fewer findings,
//! never a crash. The richer finding lifecycle (accept/ignore/exception) is a
//! later phase; for now every finding is reported as `Open`.

pub mod certs;
pub mod ports;
pub mod ssh;
pub mod sudo;
pub mod system;

pub use certs::{
    CertInfo, certificate_findings, check_certificate, days_until_expiry, parse_x509,
    read_local_cert, read_remote_cert,
};
pub use ports::check_exposed_ports;
pub use ssh::{check_failed_logins, check_sshd_config, count_failed_logins};
pub use sudo::{SudoEntry, check_sudo, parse_sudoers};
pub use system::{
    FirewallProbes, StatInfo, check_docker_socket, check_file_perms, check_firewall, check_suid,
    detect_firewall, parse_stat,
};

use systui_collectors::ExposureEntry;
use systui_core::{CommandSpec, Finding, Severity, Transport};

/// Critical files whose permissions are inspected.
const CRITICAL_FILES: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/etc/ssh/sshd_config",
    "/etc/sudoers",
    "/etc/crontab",
];

/// Directories scanned for SUID binaries.
const SUID_DIRS: &[&str] = &[
    "/usr/bin",
    "/usr/sbin",
    "/bin",
    "/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
];

const DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// Run all read-only security checks against the host, returning a worst-first
/// list of findings. `exposures` is the exposure map computed from the network
/// snapshot (pass an empty slice to skip network findings); `cert_warning_days`
/// is the certificate-expiry window (`config.security.cert_expiry_warning_days`)
/// and `cert_hosts` are remote `host:port` endpoints to inspect for TLS certs.
pub async fn security_scan(
    transport: &dyn Transport,
    exposures: &[ExposureEntry],
    cert_warning_days: u32,
    cert_hosts: &[(String, u16)],
) -> Vec<Finding> {
    let mut findings = Vec::new();

    if let Some(config) = read_text(transport, "/etc/ssh/sshd_config").await {
        findings.extend(check_sshd_config(&config));
    }

    if let Some(log) = failed_login_log(transport).await
        && let Some(finding) = check_failed_logins(count_failed_logins(&log))
    {
        findings.push(finding);
    }

    findings.extend(check_sudo(&collect_sudoers(transport).await));

    let probes = probe_firewall(transport).await;
    if let Some(finding) = check_firewall(detect_firewall(&probes)) {
        findings.push(finding);
    }

    if let Some(out) = run_stat(transport, CRITICAL_FILES).await {
        findings.extend(check_file_perms(&parse_stat(&out)));
    }

    if let Some(out) = run_stat(transport, &[DOCKER_SOCKET]).await
        && let Some(finding) = check_docker_socket(parse_stat(&out).first())
    {
        findings.push(finding);
    }

    if let Some(out) = run_find_suid(transport).await {
        let paths: Vec<String> = out.lines().map(str::to_owned).collect();
        if let Some(finding) = check_suid(&paths) {
            findings.push(finding);
        }
    }

    findings.extend(check_exposed_ports(exposures));
    findings.extend(certs::certificate_findings(transport, cert_hosts, cert_warning_days).await);

    // Worst severity first; ties broken by id for a stable order.
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}

async fn read_text(transport: &dyn Transport, path: &str) -> Option<String> {
    transport
        .read_file(path)
        .await
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

async fn run_stdout(transport: &dyn Transport, program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    match transport.run(&spec).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

/// Recent failed SSH logins, preferring the journal and falling back to
/// `/var/log/auth.log` (Debian) or `/var/log/secure` (RHEL).
async fn failed_login_log(transport: &dyn Transport) -> Option<String> {
    if let Some(out) = run_stdout(
        transport,
        "journalctl",
        &["-u", "ssh", "-u", "sshd", "--no-pager", "-n", "2000"],
    )
    .await
    {
        return Some(out);
    }
    if let Some(out) = read_text(transport, "/var/log/auth.log").await {
        return Some(out);
    }
    read_text(transport, "/var/log/secure").await
}

async fn collect_sudoers(transport: &dyn Transport) -> Vec<SudoEntry> {
    let mut entries = Vec::new();
    if let Some(text) = read_text(transport, "/etc/sudoers").await {
        entries.extend(parse_sudoers(&text));
    }
    if let Ok(dir) = transport.list_dir("/etc/sudoers.d").await {
        for entry in dir {
            let path = format!("/etc/sudoers.d/{}", entry.name);
            if let Some(text) = read_text(transport, &path).await {
                entries.extend(parse_sudoers(&text));
            }
        }
    }
    entries
}

async fn probe_firewall(transport: &dyn Transport) -> FirewallProbes {
    FirewallProbes {
        ufw_status: run_stdout(transport, "ufw", &["status"]).await,
        firewalld_state: run_stdout(transport, "firewall-cmd", &["--state"]).await,
        nft_ruleset: run_stdout(transport, "nft", &["list", "ruleset"]).await,
        iptables_save: run_stdout(transport, "iptables", &["-S"]).await,
    }
}

async fn run_stat(transport: &dyn Transport, paths: &[&str]) -> Option<String> {
    let mut args = vec!["-c", "%a %U %G %n"];
    args.extend_from_slice(paths);
    run_stdout(transport, "stat", &args).await
}

async fn run_find_suid(transport: &dyn Transport) -> Option<String> {
    let mut args: Vec<&str> = SUID_DIRS.to_vec();
    args.extend_from_slice(&["-perm", "-4000", "-type", "f"]);
    run_stdout(transport, "find", &args).await
}

/// The worst severity among a set of findings, for a dashboard summary.
pub fn worst_severity(findings: &[Finding]) -> Option<Severity> {
    findings.iter().map(|f| f.severity).max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::{CommandOutput, DirEntry, FileType};
    use systui_transport::MockTransport;

    fn mock_host() -> MockTransport {
        MockTransport::new()
            .with_file(
                "/etc/ssh/sshd_config",
                include_bytes!("../fixtures/sshd_config-insecure.txt").to_vec(),
            )
            .with_file(
                "/etc/sudoers",
                include_bytes!("../fixtures/sudoers.txt").to_vec(),
            )
            .with_stdout(
                "journalctl -u ssh -u sshd --no-pager -n 2000",
                include_str!("../fixtures/auth-failed.txt"),
            )
            .with_stdout(
                "stat -c %a %U %G %n /etc/passwd /etc/shadow /etc/ssh/sshd_config /etc/sudoers /etc/crontab",
                include_str!("../fixtures/stat-critical.txt"),
            )
            .with_command(
                "stat -c %a %U %G %n /var/run/docker.sock",
                CommandOutput {
                    exit_code: Some(0),
                    stdout: "660 root docker /var/run/docker.sock\n".to_owned(),
                    stderr: String::new(),
                    duration: std::time::Duration::ZERO,
                },
            )
            .with_stdout(
                "find /usr/bin /usr/sbin /bin /sbin /usr/local/bin /usr/local/sbin -perm -4000 -type f",
                include_str!("../fixtures/suid-find.txt"),
            )
            .with_dir("/etc/sudoers.d", vec![DirEntry {
                name: "90-cloud-init-users".to_owned(),
                file_type: FileType::File,
            }])
            .with_file(
                "/etc/sudoers.d/90-cloud-init-users",
                b"ubuntu ALL=(ALL) NOPASSWD:ALL\n".to_vec(),
            )
    }

    #[tokio::test]
    async fn full_scan_assembles_prioritized_findings() {
        let findings = security_scan(&mock_host(), &[], 30, &[]).await;
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();

        assert!(ids.contains(&"ssh.root-login"));
        assert!(ids.contains(&"ssh.password-auth"));
        assert!(ids.contains(&"ssh.failed-logins"));
        assert!(ids.contains(&"sudo.grants"));
        assert!(ids.contains(&"sudo.nopasswd"));
        assert!(ids.contains(&"firewall.absent"));
        assert!(ids.contains(&"fs.shadow-readable"));
        assert!(ids.contains(&"fs.world-writable./etc/cron.allow"));
        assert!(ids.contains(&"docker.socket-present"));
        assert!(ids.contains(&"suid.unexpected"));

        // Sorted worst-first.
        let severities: Vec<Severity> = findings.iter().map(|f| f.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(severities, sorted);
        assert_eq!(worst_severity(&findings), Some(Severity::High));
    }

    #[tokio::test]
    async fn empty_host_degrades_without_panicking() {
        // No tools, no files: only the "no firewall detected" finding remains.
        let findings = security_scan(&MockTransport::new(), &[], 30, &[]).await;
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["firewall.absent"]);
    }
}
