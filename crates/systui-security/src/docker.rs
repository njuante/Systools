//! Docker risk checks: turn a container's [`InspectSummary`] into evidence-based
//! `Finding`s — privileged containers, `docker.sock`/dangerous host mounts,
//! sensitive published ports, unhealthy containers, restart loops, floating
//! `latest` tags and missing memory limits.
//!
//! Pure functions over the inspect data (fixture-testable). Sensitive published
//! ports reuse the v0.3 exposure classifier by treating each host-mapped port as
//! a listener, so the sensitive-port set stays defined in exactly one place.

use systui_collectors::{InspectSummary, Listener, Protocol, exposure_map};
use systui_core::{Finding, ModuleId, Severity};

/// Host paths that should never be bind-mounted into a container.
const DANGEROUS_MOUNT_SOURCES: &[&str] = &["/", "/etc", "/var/run", "/run"];

/// Restart count at or above which a container looks stuck in a restart loop.
const RESTART_LOOP_THRESHOLD: u32 = 5;

/// All Docker risk findings for a set of inspected containers, worst-first.
pub fn docker_findings(inspects: &[InspectSummary]) -> Vec<Finding> {
    let mut findings: Vec<Finding> = inspects.iter().flat_map(check_container).collect();
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}

/// Risk findings for a single container.
pub fn check_container(c: &InspectSummary) -> Vec<Finding> {
    let name = &c.name;
    let mut findings = Vec::new();

    if c.privileged {
        findings.push(
            Finding::new(
                format!("docker.privileged.{name}"),
                Severity::High,
                ModuleId::Docker,
                format!("Container {name} runs in privileged mode"),
            )
            .with_evidence(format!("{name}: HostConfig.Privileged = true"))
            .impact("A privileged container can escape to the host and gain root.")
            .recommendation("Drop --privileged; grant only the specific capabilities needed."),
        );
    }

    for mount in &c.mounts {
        if mount.source.ends_with("docker.sock") {
            findings.push(
                Finding::new(
                    format!("docker.socket-mount.{name}"),
                    Severity::Critical,
                    ModuleId::Docker,
                    format!("Container {name} mounts the Docker socket"),
                )
                .with_evidence(format!(
                    "{name}: mount {} -> {}",
                    mount.source, mount.destination
                ))
                .impact("Access to docker.sock is equivalent to root on the host.")
                .recommendation("Remove the docker.sock mount or use a tightly scoped proxy."),
            );
        } else if DANGEROUS_MOUNT_SOURCES.contains(&mount.source.trim_end_matches('/'))
            || mount.source == "/"
        {
            findings.push(
                Finding::new(
                    format!("docker.dangerous-mount.{name}"),
                    Severity::High,
                    ModuleId::Docker,
                    format!("Container {name} bind-mounts a sensitive host path"),
                )
                .with_evidence(format!(
                    "{name}: mount {} -> {} ({})",
                    mount.source,
                    mount.destination,
                    if mount.rw { "rw" } else { "ro" }
                ))
                .impact("A host path like /, /etc or /var/run exposes or lets the container tamper with the host.")
                .recommendation("Mount only the specific subdirectory required, read-only where possible."),
            );
        }
    }

    findings.extend(sensitive_port_findings(c));

    if c.health == Some(systui_collectors::ContainerHealth::Unhealthy) {
        findings.push(
            Finding::new(
                format!("docker.unhealthy.{name}"),
                Severity::Medium,
                ModuleId::Docker,
                format!("Container {name} is unhealthy"),
            )
            .with_evidence(format!("{name}: health status = unhealthy"))
            .impact("The container's healthcheck is failing; it may be serving errors.")
            .recommendation(
                "Inspect the container logs and healthcheck; restart or fix the service.",
            ),
        );
    }

    if c.restart_count >= RESTART_LOOP_THRESHOLD {
        findings.push(
            Finding::new(
                format!("docker.restart-loop.{name}"),
                Severity::Medium,
                ModuleId::Docker,
                format!("Container {name} is restarting repeatedly"),
            )
            .with_evidence(format!(
                "{name}: RestartCount = {} (policy {})",
                c.restart_count, c.restart_policy
            ))
            .impact("A crash loop usually means the service is misconfigured or failing to start.")
            .recommendation("Check the logs for the startup error; fix the root cause."),
        );
    }

    if c.uses_latest_tag() {
        findings.push(
            Finding::new(
                format!("docker.latest-tag.{name}"),
                Severity::Low,
                ModuleId::Docker,
                format!("Container {name} uses a floating image tag"),
            )
            .with_evidence(format!("{name}: image {}", c.image))
            .impact("`latest` is not reproducible and can change under you on the next pull.")
            .recommendation("Pin the image to an explicit version or digest."),
        );
    }

    if c.memory_limit_bytes == 0 {
        findings.push(
            Finding::new(
                format!("docker.no-mem-limit.{name}"),
                Severity::Low,
                ModuleId::Docker,
                format!("Container {name} has no memory limit"),
            )
            .with_evidence(format!("{name}: HostConfig.Memory = 0 (unlimited)"))
            .impact("An unbounded container can exhaust host memory and trigger the OOM killer.")
            .recommendation("Set a memory limit (`--memory`) appropriate to the workload."),
        );
    }

    findings
}

/// Treat each externally bound, host-mapped port as a listener and reuse the
/// v0.3 exposure classifier to flag sensitive ones.
fn sensitive_port_findings(c: &InspectSummary) -> Vec<Finding> {
    let listeners: Vec<Listener> = c
        .published_ports
        .iter()
        .map(|p| Listener {
            protocol: if p.protocol == "udp" {
                Protocol::Udp
            } else {
                Protocol::Tcp
            },
            local_ip: p.host_ip.clone(),
            port: p.host_port,
            process: None,
            unit: None,
        })
        .collect();

    exposure_map(&listeners)
        .into_iter()
        .filter(|e| e.severity >= Severity::High)
        .map(|e| {
            let service = e.sensitive_service.unwrap_or("service");
            Finding::new(
                format!("docker.sensitive-port.{}.{}", c.name, e.listener.port),
                e.severity,
                ModuleId::Docker,
                format!(
                    "Container {} publishes sensitive port {} ({service})",
                    c.name, e.listener.port
                ),
            )
            .with_evidence(format!("{}: {}", c.name, e.evidence))
            .impact("The container exposes a sensitive service to other hosts.")
            .recommendation("Bind the published port to loopback or restrict it with a firewall.")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{ContainerHealth, Mount, PublishedPort};

    fn risky_container() -> InspectSummary {
        InspectSummary {
            id: "abc".into(),
            name: "redis".into(),
            image: "redis:latest".into(),
            privileged: true,
            restart_policy: "always".into(),
            max_retry_count: 0,
            restart_count: 7,
            memory_limit_bytes: 0,
            networks: vec!["bridge".into()],
            mounts: vec![
                Mount {
                    source: "/var/run/docker.sock".into(),
                    destination: "/var/run/docker.sock".into(),
                    rw: true,
                },
                Mount {
                    source: "/data/redis".into(),
                    destination: "/data".into(),
                    rw: true,
                },
            ],
            published_ports: vec![PublishedPort {
                host_ip: "0.0.0.0".into(),
                host_port: 6379,
                container_port: 6379,
                protocol: "tcp".into(),
            }],
            health: Some(ContainerHealth::Unhealthy),
        }
    }

    fn ids(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn flags_all_risks_on_a_bad_container() {
        let findings = check_container(&risky_container());
        let ids = ids(&findings);
        assert!(ids.contains(&"docker.privileged.redis"));
        assert!(ids.contains(&"docker.socket-mount.redis"));
        assert!(ids.contains(&"docker.sensitive-port.redis.6379"));
        assert!(ids.contains(&"docker.unhealthy.redis"));
        assert!(ids.contains(&"docker.restart-loop.redis"));
        assert!(ids.contains(&"docker.latest-tag.redis"));
        assert!(ids.contains(&"docker.no-mem-limit.redis"));

        // The Docker socket mount is critical; the published redis port too.
        let socket = findings
            .iter()
            .find(|f| f.id == "docker.socket-mount.redis")
            .unwrap();
        assert_eq!(socket.severity, Severity::Critical);
        let port = findings
            .iter()
            .find(|f| f.id == "docker.sensitive-port.redis.6379")
            .unwrap();
        assert_eq!(port.severity, Severity::Critical);
    }

    #[test]
    fn dangerous_mount_is_flagged_but_data_mount_is_not() {
        let mut c = risky_container();
        c.mounts = vec![
            Mount {
                source: "/etc".into(),
                destination: "/host-etc".into(),
                rw: false,
            },
            Mount {
                source: "/data/redis".into(),
                destination: "/data".into(),
                rw: true,
            },
        ];
        let findings = check_container(&c);
        assert!(ids(&findings).contains(&"docker.dangerous-mount.redis"));
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.id == "docker.dangerous-mount.redis")
                .count(),
            1
        );
    }

    #[test]
    fn clean_container_yields_no_findings() {
        let clean = InspectSummary {
            id: "x".into(),
            name: "web".into(),
            image: "nginx:1.25".into(),
            privileged: false,
            restart_policy: "no".into(),
            max_retry_count: 0,
            restart_count: 0,
            memory_limit_bytes: 256 * 1024 * 1024,
            networks: vec!["bridge".into()],
            mounts: vec![Mount {
                source: "/srv/www".into(),
                destination: "/usr/share/nginx/html".into(),
                rw: false,
            }],
            published_ports: vec![PublishedPort {
                host_ip: "127.0.0.1".into(),
                host_port: 8080,
                container_port: 80,
                protocol: "tcp".into(),
            }],
            health: Some(ContainerHealth::Healthy),
        };
        assert!(check_container(&clean).is_empty());
    }

    #[test]
    fn aggregate_sorts_worst_first() {
        let findings = docker_findings(&[risky_container()]);
        // First finding is Critical (socket mount or sensitive port).
        assert_eq!(findings[0].severity, Severity::Critical);
        // Sorted non-increasing by severity.
        assert!(findings.windows(2).all(|w| w[0].severity >= w[1].severity));
    }
}
