//! Certificate checks: discover local certificates and inspect a remote
//! `host:443`, then flag expiry, self-signed issuers and issuer/subject details.
//!
//! Rather than pulling a Rust X.509 stack, we shell out to `openssl` through the
//! transport (`Product.md` §4.12 decision): `openssl x509 -noout -enddate
//! -subject -issuer` for local files, and `openssl s_client` for a remote host
//! whose PEM is fed into `openssl x509` via [`CommandSpec::stdin`] (no shell
//! pipe). If `openssl` is absent the checks degrade to nothing.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use systui_core::{CommandSpec, Finding, ModuleId, Severity, Transport};

/// Directories scanned for server certificates. The system CA trust store
/// (`/etc/ssl/certs`) is deliberately excluded: it holds hundreds of long-lived,
/// self-signed roots that would flood the findings.
const COMMON_CERT_DIRS: &[&str] = &[
    "/etc/letsencrypt/live",
    "/etc/nginx/ssl",
    "/etc/apache2/ssl",
    "/etc/pki/tls/certs",
];

const CERT_EXTENSIONS: &[&str] = &[".pem", ".crt", ".cer"];

/// Fields read from a certificate via `openssl x509`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    /// Where the cert came from: a file path or `host:port`.
    pub source: String,
    pub subject: String,
    pub issuer: String,
    /// Raw `notAfter` string, e.g. `Aug 15 23:59:59 2026 GMT`.
    pub not_after: String,
}

impl CertInfo {
    /// A certificate whose subject equals its issuer is self-signed.
    pub fn is_self_signed(&self) -> bool {
        !self.subject.is_empty() && self.subject == self.issuer
    }
}

/// Parse `openssl x509 -noout -enddate -subject -issuer` output. Returns `None`
/// when no `notAfter` is present (openssl failed or the input was not a cert).
pub fn parse_x509(source: impl Into<String>, output: &str) -> Option<CertInfo> {
    let mut subject = String::new();
    let mut issuer = String::new();
    let mut not_after = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("subject=") {
            subject = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("issuer=") {
            issuer = v.trim().to_owned();
        } else if let Some(v) = line.strip_prefix("notAfter=") {
            not_after = Some(v.trim().to_owned());
        }
    }
    Some(CertInfo {
        source: source.into(),
        subject,
        issuer,
        not_after: not_after?,
    })
}

/// Parse an `openssl` date such as `Aug 15 23:59:59 2026 GMT` as UTC.
pub fn parse_openssl_date(value: &str) -> Option<DateTime<Utc>> {
    // Normalise whitespace (openssl space-pads single-digit days) and drop the
    // trailing timezone token; openssl emits GMT, which we treat as UTC.
    let tokens: Vec<&str> = value.split_whitespace().collect();
    if tokens.len() < 4 {
        return None;
    }
    let normalised = tokens[..4].join(" ");
    NaiveDateTime::parse_from_str(&normalised, "%b %d %H:%M:%S %Y")
        .ok()
        .map(|naive| naive.and_utc())
}

/// Days until the certificate expires (negative if already expired).
pub fn days_until_expiry(not_after: &str, now: DateTime<Utc>) -> Option<i64> {
    Some((parse_openssl_date(not_after)? - now).num_days())
}

/// Findings for one certificate: expired, expiring within the warning window,
/// and self-signed. A valid, CA-issued certificate produces nothing.
pub fn check_certificate(cert: &CertInfo, warning_days: u32, now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();

    match days_until_expiry(&cert.not_after, now) {
        Some(days) if days < 0 => findings.push(
            Finding::new(
                format!("cert.expired.{}", cert.source),
                Severity::High,
                ModuleId::Certificates,
                format!("Certificate expired ({})", cert.source),
            )
            .evidence([
                format!("source: {}", cert.source),
                format!("subject: {}", cert.subject),
                format!("notAfter: {} ({} days ago)", cert.not_after, -days),
            ])
            .impact("Clients reject the certificate; TLS connections fail or warn.")
            .recommendation("Renew and deploy the certificate immediately."),
        ),
        Some(days) if days <= i64::from(warning_days) => findings.push(
            Finding::new(
                format!("cert.expiring.{}", cert.source),
                Severity::Medium,
                ModuleId::Certificates,
                format!("Certificate expiring soon ({})", cert.source),
            )
            .evidence([
                format!("source: {}", cert.source),
                format!("subject: {}", cert.subject),
                format!("notAfter: {} (in {days} days)", cert.not_after),
            ])
            .impact("The certificate will stop being trusted once it expires.")
            .recommendation(format!(
                "Renew before it expires (warning window {warning_days} days)."
            )),
        ),
        _ => {}
    }

    if cert.is_self_signed() {
        findings.push(
            Finding::new(
                format!("cert.self-signed.{}", cert.source),
                Severity::Low,
                ModuleId::Certificates,
                format!("Self-signed certificate ({})", cert.source),
            )
            .evidence([
                format!("source: {}", cert.source),
                format!("subject == issuer: {}", cert.subject),
            ])
            .impact("Clients cannot verify the identity through a trusted CA.")
            .recommendation("Use a CA-issued certificate for anything externally facing."),
        );
    }

    findings
}

/// Extract the first PEM certificate block from `openssl s_client` output.
fn extract_pem(text: &str) -> Option<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END)? + start + END.len();
    Some(text[start..end].to_owned())
}

async fn run_stdout(transport: &dyn Transport, spec: CommandSpec) -> Option<String> {
    match transport.run(&spec).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

/// Read a local certificate file via `openssl x509 -in <path>`.
pub async fn read_local_cert(transport: &dyn Transport, path: &str) -> Option<CertInfo> {
    let spec = CommandSpec::new("openssl")
        .args([
            "x509", "-in", path, "-noout", "-enddate", "-subject", "-issuer",
        ])
        .timeout(Duration::from_secs(5));
    parse_x509(path, &run_stdout(transport, spec).await?)
}

/// Fetch and inspect a remote certificate from `host:port`.
pub async fn read_remote_cert(
    transport: &dyn Transport,
    host: &str,
    port: u16,
) -> Option<CertInfo> {
    let connect = format!("{host}:{port}");
    let s_client = CommandSpec::new("openssl")
        .args(["s_client", "-connect", &connect, "-servername", host])
        // Empty stdin closes the connection so s_client returns instead of hanging.
        .stdin(String::new())
        .timeout(Duration::from_secs(8));
    let pem = extract_pem(&run_stdout(transport, s_client).await?)?;

    let x509 = CommandSpec::new("openssl")
        .args(["x509", "-noout", "-enddate", "-subject", "-issuer"])
        .stdin(pem)
        .timeout(Duration::from_secs(5));
    parse_x509(connect, &run_stdout(transport, x509).await?)
}

/// Discover candidate local certificate files in the common server-cert dirs
/// (one level of subdirectories deep, e.g. `letsencrypt/live/<domain>/`).
pub async fn discover_local_certs(transport: &dyn Transport) -> Vec<String> {
    let mut paths = Vec::new();
    for dir in COMMON_CERT_DIRS {
        collect_certs_in(transport, dir, true, &mut paths).await;
    }
    paths
}

async fn collect_certs_in(
    transport: &dyn Transport,
    dir: &str,
    descend: bool,
    out: &mut Vec<String>,
) {
    let Ok(entries) = transport.list_dir(dir).await else {
        return;
    };
    for entry in entries {
        let path = format!("{dir}/{}", entry.name);
        let is_cert = CERT_EXTENSIONS.iter().any(|ext| entry.name.ends_with(ext));
        if is_cert {
            out.push(path);
        } else if descend && entry.file_type == systui_core::FileType::Dir {
            Box::pin(collect_certs_in(transport, &path, false, out)).await;
        }
    }
}

/// Inspect all discovered local certificates and the given remote hosts,
/// returning the resulting findings.
pub async fn certificate_findings(
    transport: &dyn Transport,
    remote_hosts: &[(String, u16)],
    warning_days: u32,
) -> Vec<Finding> {
    let now = Utc::now();
    let mut findings = Vec::new();

    for path in discover_local_certs(transport).await {
        if let Some(cert) = read_local_cert(transport, &path).await {
            findings.extend(check_certificate(&cert, warning_days, now));
        }
    }
    for (host, port) in remote_hosts {
        if let Some(cert) = read_remote_cert(transport, host, *port).await {
            findings.extend(check_certificate(&cert, warning_days, now));
        }
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
        fn cert_parsers_never_panic(s in messy_output()) {
            let now = Utc::now();
            let _ = parse_x509("fuzz", &s);
            let _ = parse_openssl_date(&s);
            let _ = days_until_expiry(&s, now);
            let _ = extract_pem(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use systui_core::{DirEntry, FileType};
    use systui_transport::MockTransport;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 24, 0, 0, 0).unwrap()
    }

    #[test]
    fn parses_x509_fields() {
        let cert = parse_x509(
            "/etc/ssl/c.pem",
            include_str!("../fixtures/openssl-x509.txt"),
        )
        .unwrap();
        assert_eq!(cert.subject, "CN = example.com");
        assert!(cert.issuer.contains("Let's Encrypt"));
        assert_eq!(cert.not_after, "Aug 15 23:59:59 2026 GMT");
        assert!(!cert.is_self_signed());
    }

    #[test]
    fn detects_self_signed() {
        let cert = parse_x509(
            "host:443",
            include_str!("../fixtures/openssl-x509-selfsigned.txt"),
        )
        .unwrap();
        assert!(cert.is_self_signed());
    }

    #[test]
    fn missing_not_after_is_none() {
        assert!(parse_x509("x", "subject=CN = a\nissuer=CN = b\n").is_none());
    }

    #[test]
    fn parses_openssl_date_and_computes_days() {
        let date = parse_openssl_date("Aug 15 23:59:59 2026 GMT").unwrap();
        assert_eq!(date.format("%Y-%m-%d").to_string(), "2026-08-15");
        // Single-digit space-padded day must parse too.
        assert!(parse_openssl_date("Aug  5 23:59:59 2026 GMT").is_some());
        let days = days_until_expiry("Aug 15 23:59:59 2026 GMT", now()).unwrap();
        assert!((82..=84).contains(&days));
    }

    #[test]
    fn flags_expired_expiring_and_self_signed() {
        let expired = CertInfo {
            source: "host:443".into(),
            subject: "CN = a".into(),
            issuer: "CN = ca".into(),
            not_after: "Jan 01 00:00:00 2020 GMT".into(),
        };
        assert_eq!(
            check_certificate(&expired, 30, now())[0].severity,
            Severity::High
        );

        let expiring = CertInfo {
            not_after: "Jun 10 00:00:00 2026 GMT".into(),
            ..expired.clone()
        };
        assert_eq!(
            check_certificate(&expiring, 30, now())[0].severity,
            Severity::Medium
        );

        let selfsigned = CertInfo {
            subject: "CN = internal".into(),
            issuer: "CN = internal".into(),
            not_after: "Jan 10 00:00:00 2027 GMT".into(),
            ..expired.clone()
        };
        let findings = check_certificate(&selfsigned, 30, now());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Low);
    }

    #[test]
    fn extracts_pem_block() {
        let pem = extract_pem(include_str!("../fixtures/s_client.txt")).unwrap();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(pem.trim_end().ends_with("-----END CERTIFICATE-----"));
    }

    #[tokio::test]
    async fn reads_local_cert_via_openssl() {
        let transport = MockTransport::new().with_stdout(
            "openssl x509 -in /etc/nginx/ssl/site.pem -noout -enddate -subject -issuer",
            include_str!("../fixtures/openssl-x509.txt"),
        );
        let cert = read_local_cert(&transport, "/etc/nginx/ssl/site.pem")
            .await
            .unwrap();
        assert_eq!(cert.subject, "CN = example.com");
    }

    #[tokio::test]
    async fn reads_remote_cert_via_s_client_and_x509() {
        let transport = MockTransport::new()
            .with_stdout(
                "openssl s_client -connect example.com:443 -servername example.com",
                include_str!("../fixtures/s_client.txt"),
            )
            .with_stdout(
                "openssl x509 -noout -enddate -subject -issuer",
                include_str!("../fixtures/openssl-x509.txt"),
            );
        let cert = read_remote_cert(&transport, "example.com", 443)
            .await
            .unwrap();
        assert_eq!(cert.source, "example.com:443");
        assert_eq!(cert.subject, "CN = example.com");
    }

    #[tokio::test]
    async fn discovers_certs_one_level_deep() {
        let transport = MockTransport::new()
            .with_dir(
                "/etc/letsencrypt/live",
                vec![DirEntry {
                    name: "example.com".to_owned(),
                    file_type: FileType::Dir,
                }],
            )
            .with_dir(
                "/etc/letsencrypt/live/example.com",
                vec![
                    DirEntry {
                        name: "fullchain.pem".to_owned(),
                        file_type: FileType::File,
                    },
                    DirEntry {
                        name: "README".to_owned(),
                        file_type: FileType::File,
                    },
                ],
            );
        let paths = discover_local_certs(&transport).await;
        assert_eq!(paths, ["/etc/letsencrypt/live/example.com/fullchain.pem"]);
    }
}
