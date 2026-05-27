//! Expected-state policy evaluation.
//!
//! The evaluator is deliberately pure: callers pass the policy selection and
//! already-collected host facts. This keeps policies out of the command-running
//! path and preserves local/SSH parity.

use std::collections::{BTreeMap, BTreeSet};

use systui_collectors::{
    BindScope, Container, ExposureEntry, InspectSummary, NetworkSnapshot, ServiceUnit,
    SystemSnapshot,
};
use systui_core::{Config, Finding, ModuleId, Policy, PolicyResolution, PolicySource, Severity};

/// Owned policy selection for a host. This can cross async task boundaries while
/// preserving whether the policy came from an explicit host setting or tag
/// fallback.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PolicySelection {
    Matched {
        name: String,
        policy: Box<Policy>,
        source: PolicySource,
    },
    MissingExplicit {
        name: String,
    },
    #[default]
    None,
}

impl PolicySelection {
    /// Resolve and clone the policy selected for `host_id` from config.
    pub fn for_host(config: &Config, host_id: &str) -> Self {
        let Some(host) = config.hosts.get(host_id) else {
            return Self::None;
        };
        match config.resolve_policy_for_host(host) {
            PolicyResolution::Matched(matched) => Self::Matched {
                name: matched.name.to_owned(),
                policy: Box::new(matched.policy.clone()),
                source: matched.source,
            },
            PolicyResolution::MissingExplicit { name } => Self::MissingExplicit {
                name: name.to_owned(),
            },
            PolicyResolution::None => Self::None,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Matched { name, .. } | Self::MissingExplicit { name } => Some(name),
            Self::None => None,
        }
    }
}

/// Collected facts the policy evaluator can inspect.
#[derive(Debug, Clone, Copy)]
pub struct PolicyFacts<'a> {
    pub host_label: &'a str,
    pub snapshot: &'a SystemSnapshot,
    pub network: Option<&'a NetworkSnapshot>,
    pub exposures: &'a [ExposureEntry],
    pub services: &'a [ServiceUnit],
    pub containers: &'a [Container],
    pub container_inspects: &'a [InspectSummary],
    pub docker_available: bool,
}

/// Evaluate an expected-state policy into stable `policy.*` findings.
pub fn policy_findings(selection: &PolicySelection, facts: PolicyFacts<'_>) -> Vec<Finding> {
    let mut findings = match selection {
        PolicySelection::Matched { name, policy, .. } => evaluate_matched(name, policy, facts),
        PolicySelection::MissingExplicit { name } => vec![missing_policy(name, facts.host_label)],
        PolicySelection::None => Vec::new(),
    };
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}

fn evaluate_matched(name: &str, policy: &Policy, facts: PolicyFacts<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();

    findings.extend(port_findings(name, policy, facts.network, facts.exposures));
    findings.extend(service_findings(name, policy, facts.services));
    findings.extend(threshold_findings(name, policy, facts.snapshot));
    findings.extend(docker_policy_findings(
        name,
        policy,
        facts.containers,
        facts.container_inspects,
        facts.docker_available,
    ));
    findings.extend(unsupported_fact_findings(name, policy));

    findings
}

fn missing_policy(name: &str, host_label: &str) -> Finding {
    Finding::new(
        format!("policy.config.missing.{}", stable(name)),
        Severity::Medium,
        ModuleId::Security,
        format!("Configured policy `{name}` was not found"),
    )
    .with_evidence(format!(
        "host {host_label} references policy `{name}`, but `[policies.{name}]` is missing"
    ))
    .impact("Expected-state validation could not run for this host.")
    .recommendation("Add the policy definition or update the host's `policy` setting.")
}

fn port_findings(
    policy_name: &str,
    policy: &Policy,
    network: Option<&NetworkSnapshot>,
    exposures: &[ExposureEntry],
) -> Vec<Finding> {
    if policy.expected_ports.is_empty() && policy.forbidden_ports.is_empty() {
        return Vec::new();
    }
    let Some(network) = network else {
        return vec![partial_finding(
            policy_name,
            "network",
            "Network policy could not be fully evaluated",
            "network snapshot is unavailable; port expectations are unknown",
        )];
    };

    let listeners = listeners_by_port(network);
    let mut findings = Vec::new();

    for port in sorted_ports(&policy.expected_ports) {
        if !listeners.contains_key(&port) {
            findings.push(
                Finding::new(
                    format!("policy.port.missing.{}.{}", stable(policy_name), port),
                    Severity::Medium,
                    ModuleId::Security,
                    format!("Expected port {port} is not listening"),
                )
                .with_evidence(format!(
                    "policy `{policy_name}` requires port {port}, but no listener was found"
                ))
                .impact("A required service may be down or bound to the wrong interface.")
                .recommendation("Start the expected service or update the policy if it changed."),
            );
        }
    }

    for port in sorted_ports(&policy.forbidden_ports) {
        let Some(evidence) = listeners.get(&port) else {
            continue;
        };
        let severity = forbidden_port_severity(port, exposures);
        findings.push(
            Finding::new(
                format!("policy.port.forbidden.{}.{}", stable(policy_name), port),
                severity,
                ModuleId::Security,
                format!("Forbidden port {port} is listening"),
            )
            .evidence(evidence.iter().cloned())
            .impact("The host exposes a port that is not allowed by policy.")
            .recommendation("Stop the listener, restrict its bind address, or update the policy."),
        );
    }

    findings
}

fn listeners_by_port(network: &NetworkSnapshot) -> BTreeMap<u16, Vec<String>> {
    let mut by_port: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for listener in &network.listeners {
        let owner = listener
            .unit
            .as_deref()
            .or_else(|| listener.process.as_ref().map(|p| p.name.as_str()))
            .unwrap_or("unknown owner");
        by_port.entry(listener.port).or_default().push(format!(
            "{} {}:{} ({owner})",
            protocol_label(listener.protocol),
            listener.local_ip,
            listener.port
        ));
    }
    by_port
}

fn forbidden_port_severity(port: u16, exposures: &[ExposureEntry]) -> Severity {
    exposures
        .iter()
        .filter(|entry| entry.listener.port == port)
        .map(|entry| match entry.scope {
            BindScope::External => entry.severity.max(Severity::High),
            BindScope::Loopback => Severity::Medium,
        })
        .max()
        .unwrap_or(Severity::Medium)
}

fn protocol_label(protocol: systui_collectors::Protocol) -> &'static str {
    match protocol {
        systui_collectors::Protocol::Tcp => "tcp",
        systui_collectors::Protocol::Udp => "udp",
    }
}

fn service_findings(policy_name: &str, policy: &Policy, services: &[ServiceUnit]) -> Vec<Finding> {
    if policy.expected_services.is_empty() && policy.forbidden_services.is_empty() {
        return Vec::new();
    }
    if services.is_empty() {
        return vec![partial_finding(
            policy_name,
            "services",
            "Service policy could not be fully evaluated",
            "service inventory is unavailable; service expectations are unknown",
        )];
    }

    let mut by_name = BTreeMap::new();
    for unit in services {
        by_name.insert(canonical_service_name(&unit.name), unit);
        by_name.insert(unit.name.clone(), unit);
    }

    let mut findings = Vec::new();
    for service in sorted_strings(&policy.expected_services) {
        match by_name.get(&canonical_service_name(&service)) {
            None => findings.push(
                Finding::new(
                    format!(
                        "policy.service.missing.{}.{}",
                        stable(policy_name),
                        stable(&service)
                    ),
                    Severity::Medium,
                    ModuleId::Security,
                    format!("Expected service `{service}` is missing"),
                )
                .with_evidence(format!(
                    "policy `{policy_name}` requires `{service}`, but it was not listed by systemd"
                ))
                .impact("A required service is not installed or not visible to systemd.")
                .recommendation("Install/start the service or update the policy."),
            ),
            Some(unit) if unit.is_failed() => findings.push(service_state_finding(
                policy_name,
                &service,
                unit,
                Severity::High,
                "Expected service is failed",
            )),
            Some(unit) if unit.active != "active" => findings.push(service_state_finding(
                policy_name,
                &service,
                unit,
                Severity::Medium,
                "Expected service is not active",
            )),
            Some(_) => {}
        }
    }

    for service in sorted_strings(&policy.forbidden_services) {
        if let Some(unit) = by_name.get(&canonical_service_name(&service)) {
            let severity = if unit.active == "active" {
                Severity::High
            } else {
                Severity::Low
            };
            findings.push(
                Finding::new(
                    format!(
                        "policy.service.forbidden.{}.{}",
                        stable(policy_name),
                        stable(&service)
                    ),
                    severity,
                    ModuleId::Security,
                    format!("Forbidden service `{service}` is present"),
                )
                .with_evidence(format!(
                    "{} load={} active={} sub={}",
                    unit.name, unit.load, unit.active, unit.sub
                ))
                .impact("The host has a service that is not allowed by policy.")
                .recommendation(
                    "Disable/remove the service or update the policy if it is intended.",
                ),
            );
        }
    }

    findings
}

fn service_state_finding(
    policy_name: &str,
    service: &str,
    unit: &ServiceUnit,
    severity: Severity,
    title: &str,
) -> Finding {
    Finding::new(
        format!(
            "policy.service.state.{}.{}",
            stable(policy_name),
            stable(service)
        ),
        severity,
        ModuleId::Security,
        format!("{title}: `{service}`"),
    )
    .with_evidence(format!(
        "{} load={} active={} sub={}",
        unit.name, unit.load, unit.active, unit.sub
    ))
    .impact("A required service is not running in the expected state.")
    .recommendation("Investigate the unit and restore the expected state.")
}

fn threshold_findings(policy_name: &str, policy: &Policy, snap: &SystemSnapshot) -> Vec<Finding> {
    let mut findings = Vec::new();

    for disk in &snap.disks {
        if let Some(critical) = policy.disk_critical
            && disk.use_percent >= critical
        {
            findings.push(
                Finding::new(
                    format!(
                        "policy.threshold.disk-critical.{}.{}",
                        stable(policy_name),
                        stable(&disk.mount)
                    ),
                    Severity::Critical,
                    ModuleId::Security,
                    format!("Disk `{}` exceeds policy critical threshold", disk.mount),
                )
                .with_evidence(format!(
                    "{} is at {}% (critical threshold {}%)",
                    disk.mount, disk.use_percent, critical
                ))
                .impact("The filesystem may run out of space.")
                .recommendation("Free space, expand the filesystem, or adjust the policy."),
            );
            continue;
        }
        if let Some(warning) = policy.disk_warning
            && disk.use_percent >= warning
        {
            findings.push(
                Finding::new(
                    format!(
                        "policy.threshold.disk-warning.{}.{}",
                        stable(policy_name),
                        stable(&disk.mount)
                    ),
                    Severity::High,
                    ModuleId::Security,
                    format!("Disk `{}` exceeds policy warning threshold", disk.mount),
                )
                .with_evidence(format!(
                    "{} is at {}% (warning threshold {}%)",
                    disk.mount, disk.use_percent, warning
                ))
                .impact("The filesystem is outside the expected operating range.")
                .recommendation("Free space, expand the filesystem, or adjust the policy."),
            );
        }
    }

    if let Some(warning) = policy.ram_warning {
        let used = snap.memory.used_percent();
        if used >= f64::from(warning) {
            findings.push(
                Finding::new(
                    format!("policy.threshold.ram.{}", stable(policy_name)),
                    Severity::Medium,
                    ModuleId::Security,
                    "RAM usage exceeds policy threshold",
                )
                .with_evidence(format!("RAM is at {used:.0}% (threshold {warning}%)"))
                .impact("The host is outside the expected memory usage range.")
                .recommendation("Reduce workload, add memory, or adjust the policy."),
            );
        }
    }

    if let Some(multiplier) = policy.load_warning_multiplier {
        let limit = (snap.cpu.cores.max(1) as f64) * multiplier;
        if snap.load.one > limit {
            findings.push(
                Finding::new(
                    format!("policy.threshold.load.{}", stable(policy_name)),
                    Severity::Medium,
                    ModuleId::Security,
                    "Load exceeds policy threshold",
                )
                .with_evidence(format!(
                    "1m load {:.2} > {:.2} ({} cores * {multiplier})",
                    snap.load.one, limit, snap.cpu.cores
                ))
                .impact("The host is busier than policy allows.")
                .recommendation("Investigate workload, scale capacity, or adjust the policy."),
            );
        }
    }

    findings
}

fn docker_policy_findings(
    policy_name: &str,
    policy: &Policy,
    containers: &[Container],
    inspects: &[InspectSummary],
    docker_available: bool,
) -> Vec<Finding> {
    let has_policy = !policy.expected_containers.is_empty()
        || !policy.forbidden_containers.is_empty()
        || !policy.forbidden_images.is_empty();
    if !has_policy {
        return Vec::new();
    }
    if !docker_available {
        return vec![partial_finding(
            policy_name,
            "docker",
            "Docker policy could not be fully evaluated",
            "Docker is unavailable; container expectations are unknown",
        )];
    }

    let mut by_name = BTreeMap::new();
    for container in containers {
        by_name.insert(container.name.clone(), container);
    }
    let inspect_images: BTreeMap<&str, &str> = inspects
        .iter()
        .map(|inspect| (inspect.name.as_str(), inspect.image.as_str()))
        .collect();

    let mut findings = Vec::new();
    for expected in &policy.expected_containers {
        let Some(container) = by_name.get(&expected.name) else {
            findings.push(
                Finding::new(
                    format!(
                        "policy.container.missing.{}.{}",
                        stable(policy_name),
                        stable(&expected.name)
                    ),
                    Severity::Medium,
                    ModuleId::Security,
                    format!("Expected container `{}` is missing", expected.name),
                )
                .with_evidence(format!(
                    "policy `{policy_name}` requires container `{}`",
                    expected.name
                ))
                .impact("A required containerized workload is not present.")
                .recommendation("Start/deploy the container or update the policy."),
            );
            continue;
        };

        if !container.is_running() {
            findings.push(
                Finding::new(
                    format!(
                        "policy.container.stopped.{}.{}",
                        stable(policy_name),
                        stable(&expected.name)
                    ),
                    Severity::Medium,
                    ModuleId::Security,
                    format!("Expected container `{}` is not running", expected.name),
                )
                .with_evidence(format!(
                    "{} state={} status={}",
                    container.name, container.state, container.status
                ))
                .impact("A required containerized workload is stopped.")
                .recommendation("Start the container or update the policy."),
            );
        }

        if let Some(expected_image) = &expected.image {
            let actual = inspect_images
                .get(expected.name.as_str())
                .copied()
                .unwrap_or(container.image.as_str());
            if actual != expected_image {
                findings.push(
                    Finding::new(
                        format!(
                            "policy.container.image.{}.{}",
                            stable(policy_name),
                            stable(&expected.name)
                        ),
                        Severity::Medium,
                        ModuleId::Security,
                        format!("Container `{}` image differs from policy", expected.name),
                    )
                    .with_evidence(format!(
                        "{} image is `{actual}`, expected `{expected_image}`",
                        expected.name
                    ))
                    .impact("The container is not running the expected image.")
                    .recommendation("Deploy the expected image or update the policy."),
                );
            }
        }
    }

    let forbidden_containers: BTreeSet<&str> = policy
        .forbidden_containers
        .iter()
        .map(String::as_str)
        .collect();
    for container in containers
        .iter()
        .filter(|container| forbidden_containers.contains(container.name.as_str()))
    {
        findings.push(forbidden_container_finding(policy_name, container));
    }

    for forbidden_image in &policy.forbidden_images {
        for container in containers.iter().filter(|c| c.image == *forbidden_image) {
            let severity = if container.is_running() {
                Severity::High
            } else {
                Severity::Medium
            };
            findings.push(
                Finding::new(
                    format!(
                        "policy.image.forbidden.{}.{}.{}",
                        stable(policy_name),
                        stable(forbidden_image),
                        stable(&container.name)
                    ),
                    severity,
                    ModuleId::Security,
                    format!("Forbidden image `{forbidden_image}` is present"),
                )
                .with_evidence(format!(
                    "{} uses forbidden image `{}` (state {})",
                    container.name, container.image, container.state
                ))
                .impact("The host is running or retaining an image disallowed by policy.")
                .recommendation("Remove the container/image or update the policy."),
            );
        }
    }

    findings
}

fn forbidden_container_finding(policy_name: &str, container: &Container) -> Finding {
    let severity = if container.is_running() {
        Severity::High
    } else {
        Severity::Medium
    };
    Finding::new(
        format!(
            "policy.container.forbidden.{}.{}",
            stable(policy_name),
            stable(&container.name)
        ),
        severity,
        ModuleId::Security,
        format!("Forbidden container `{}` is present", container.name),
    )
    .with_evidence(format!(
        "{} image={} state={} status={}",
        container.name, container.image, container.state, container.status
    ))
    .impact("A container disallowed by policy exists on this host.")
    .recommendation("Remove the container or update the policy if it is intended.")
}

fn unsupported_fact_findings(policy_name: &str, policy: &Policy) -> Vec<Finding> {
    let mut findings = Vec::new();
    if !policy.expected_sudo_users.is_empty() || !policy.forbidden_users.is_empty() {
        findings.push(partial_finding(
            policy_name,
            "identity",
            "Identity policy could not be fully evaluated",
            "sudo/user facts are not part of the collected host snapshot yet",
        ));
    }
    if !policy.expected_certs.is_empty() {
        findings.push(partial_finding(
            policy_name,
            "certificates",
            "Certificate policy could not be fully evaluated",
            "structured certificate facts are not part of the collected host snapshot yet",
        ));
    }
    findings
}

fn partial_finding(policy_name: &str, area: &str, title: &str, evidence: &str) -> Finding {
    Finding::new(
        format!("policy.partial.{}.{}", stable(policy_name), stable(area)),
        Severity::Info,
        ModuleId::Security,
        title,
    )
    .with_evidence(evidence)
    .impact("Policy compliance for this area is unknown, not confirmed.")
    .recommendation("Collect the required facts or remove this policy expectation.")
}

fn canonical_service_name(name: &str) -> String {
    if name.ends_with(".service") {
        name.to_owned()
    } else {
        format!("{name}.service")
    }
}

fn sorted_ports(ports: &[u16]) -> Vec<u16> {
    ports
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted_strings(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn stable(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let stable = out.trim_matches('-');
    if stable.is_empty() {
        "root".to_owned()
    } else {
        stable.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::{
        AddrFamily, BindScope, CpuUsage, Disk, DnsConfig, InterfaceAddr, Listener, Memory,
        NetInterface, Protocol, Route, Swap,
    };

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            hostname: "prod-01".to_owned(),
            os: None,
            kernel: "6.1".to_owned(),
            uptime_secs: 1,
            load: systui_collectors::LoadAverage {
                one: 9.0,
                five: 0.0,
                fifteen: 0.0,
            },
            cpu: CpuUsage {
                busy_percent: 0.0,
                cores: 4,
            },
            memory: Memory {
                total_kb: 100,
                available_kb: 10,
            },
            swap: Swap::default(),
            disks: vec![Disk {
                filesystem: "/dev/sda1".to_owned(),
                size_kb: 100,
                used_kb: 91,
                avail_kb: 9,
                use_percent: 91,
                mount: "/".to_owned(),
            }],
            users: Vec::new(),
            cpu_model: None,
            virtualization: None,
        }
    }

    fn network() -> NetworkSnapshot {
        NetworkSnapshot {
            interfaces: vec![NetInterface {
                name: "eth0".to_owned(),
                state: "UP".to_owned(),
                addrs: vec![InterfaceAddr {
                    ip: "10.0.0.10".to_owned(),
                    prefix_len: 24,
                    family: AddrFamily::V4,
                }],
            }],
            routes: vec![Route {
                dst: "default".to_owned(),
                gateway: Some("10.0.0.1".to_owned()),
                dev: "eth0".to_owned(),
                prefsrc: None,
            }],
            dns: DnsConfig::default(),
            listeners: vec![
                listener(22, "0.0.0.0", Some("sshd.service")),
                listener(6379, "0.0.0.0", Some("redis.service")),
            ],
            connections: Vec::new(),
        }
    }

    fn listener(port: u16, ip: &str, unit: Option<&str>) -> Listener {
        Listener {
            protocol: Protocol::Tcp,
            local_ip: ip.to_owned(),
            port,
            process: None,
            unit: unit.map(str::to_owned),
        }
    }

    fn exposure(port: u16, severity: Severity) -> ExposureEntry {
        ExposureEntry {
            listener: listener(port, "0.0.0.0", None),
            scope: BindScope::External,
            sensitive_service: None,
            severity,
            evidence: format!("port {port} exposed"),
        }
    }

    fn facts<'a>(
        snap: &'a SystemSnapshot,
        net: Option<&'a NetworkSnapshot>,
        exposures: &'a [ExposureEntry],
        services: &'a [ServiceUnit],
        containers: &'a [Container],
    ) -> PolicyFacts<'a> {
        PolicyFacts {
            host_label: "prod-01",
            snapshot: snap,
            network: net,
            exposures,
            services,
            containers,
            container_inspects: &[],
            docker_available: true,
        }
    }

    #[test]
    fn missing_explicit_policy_becomes_a_finding() {
        let findings = policy_findings(
            &PolicySelection::MissingExplicit {
                name: "missing".to_owned(),
            },
            facts(&snapshot(), Some(&network()), &[], &[], &[]),
        );

        assert_eq!(findings[0].id, "policy.config.missing.missing");
        assert_eq!(findings[0].severity, Severity::Medium);
    }

    #[test]
    fn evaluates_ports_services_and_thresholds() {
        let policy = Policy {
            expected_ports: vec![22, 443],
            forbidden_ports: vec![6379],
            expected_services: vec!["nginx".to_owned(), "sshd.service".to_owned()],
            forbidden_services: vec!["redis".to_owned()],
            disk_warning: Some(80),
            disk_critical: Some(90),
            ram_warning: Some(80),
            load_warning_multiplier: Some(1.5),
            ..Policy::default()
        };
        let services = vec![
            ServiceUnit {
                name: "sshd.service".to_owned(),
                load: "loaded".to_owned(),
                active: "active".to_owned(),
                sub: "running".to_owned(),
                description: "ssh".to_owned(),
            },
            ServiceUnit {
                name: "redis.service".to_owned(),
                load: "loaded".to_owned(),
                active: "active".to_owned(),
                sub: "running".to_owned(),
                description: "redis".to_owned(),
            },
        ];
        let exposures = vec![exposure(6379, Severity::Critical)];
        let selection = PolicySelection::Matched {
            name: "prod-web".to_owned(),
            policy: Box::new(policy),
            source: PolicySource::ExplicitHost,
        };

        let findings = policy_findings(
            &selection,
            facts(&snapshot(), Some(&network()), &exposures, &services, &[]),
        );
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();

        assert!(ids.contains(&"policy.port.missing.prod-web.443"));
        assert!(ids.contains(&"policy.port.forbidden.prod-web.6379"));
        assert!(ids.contains(&"policy.service.missing.prod-web.nginx"));
        assert!(ids.contains(&"policy.service.forbidden.prod-web.redis"));
        assert!(ids.contains(&"policy.threshold.disk-critical.prod-web.root"));
        assert!(ids.contains(&"policy.threshold.ram.prod-web"));
        assert!(ids.contains(&"policy.threshold.load.prod-web"));
        let redis = findings
            .iter()
            .find(|f| f.id == "policy.port.forbidden.prod-web.6379")
            .unwrap();
        assert_eq!(redis.severity, Severity::Critical);
    }

    #[test]
    fn docker_policy_checks_expected_and_forbidden_containers() {
        let policy = Policy {
            expected_containers: vec![systui_core::ExpectedContainer {
                name: "web".to_owned(),
                image: Some("nginx:1.25".to_owned()),
            }],
            forbidden_containers: vec!["debug".to_owned()],
            forbidden_images: vec!["redis:latest".to_owned()],
            ..Policy::default()
        };
        let containers = vec![
            container("web", "nginx:latest", "running"),
            container("debug", "busybox:latest", "exited"),
            container("cache", "redis:latest", "running"),
        ];
        let selection = PolicySelection::Matched {
            name: "prod-web".to_owned(),
            policy: Box::new(policy),
            source: PolicySource::ExplicitHost,
        };

        let findings = policy_findings(
            &selection,
            facts(&snapshot(), Some(&network()), &[], &[], &containers),
        );
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();

        assert!(ids.contains(&"policy.container.image.prod-web.web"));
        assert!(ids.contains(&"policy.container.forbidden.prod-web.debug"));
        assert!(ids.contains(&"policy.image.forbidden.prod-web.redis-latest.cache"));
    }

    #[test]
    fn partial_findings_mark_unknown_policy_areas() {
        let policy = Policy {
            expected_ports: vec![443],
            expected_services: vec!["nginx".to_owned()],
            expected_sudo_users: vec!["admin".to_owned()],
            expected_certs: vec![systui_core::ExpectedCertificate {
                host: "example.com:443".to_owned(),
                names: vec!["example.com".to_owned()],
                min_days_remaining: Some(30),
            }],
            ..Policy::default()
        };
        let selection = PolicySelection::Matched {
            name: "prod-web".to_owned(),
            policy: Box::new(policy),
            source: PolicySource::TagFallback,
        };

        let findings = policy_findings(&selection, facts(&snapshot(), None, &[], &[], &[]));
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();

        assert!(ids.contains(&"policy.partial.prod-web.network"));
        assert!(ids.contains(&"policy.partial.prod-web.services"));
        assert!(ids.contains(&"policy.partial.prod-web.identity"));
        assert!(ids.contains(&"policy.partial.prod-web.certificates"));
    }

    fn container(name: &str, image: &str, state: &str) -> Container {
        Container {
            id: name.to_owned(),
            name: name.to_owned(),
            image: image.to_owned(),
            state: state.to_owned(),
            status: state.to_owned(),
            health: None,
            ports: String::new(),
            created: String::new(),
        }
    }
}
