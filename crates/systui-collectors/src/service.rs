//! systemd service collector: full unit listing, failed-only listing and
//! per-unit detail. All read agentlessly via `systemctl`. The service *actions*
//! (start/stop/restart/…) live in `systui-actions`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A systemd unit row from `systemctl list-units`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceUnit {
    pub name: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

impl ServiceUnit {
    /// Whether the unit is in a failed state.
    pub fn is_failed(&self) -> bool {
        self.active == "failed" || self.sub == "failed"
    }
}

/// Detailed state of a single unit from `systemctl show`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UnitDetail {
    pub name: String,
    pub description: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub main_pid: Option<u32>,
    pub unit_file_state: String,
    pub fragment_path: String,
}

/// Lists all service units via `systemctl list-units`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ServiceCollector;

impl ServiceCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for ServiceCollector {
    type Output = Vec<ServiceUnit>;

    fn module(&self) -> ModuleId {
        ModuleId::Services
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<ServiceUnit>> {
        let spec = CommandSpec::new("systemctl").args([
            "list-units",
            "--type=service",
            "--all",
            "--no-legend",
            "--plain",
            "--no-pager",
        ]);
        let output = transport.run(&spec).await?.into_result("systemctl")?;
        Ok(parse_units(&output.stdout))
    }
}

/// Collects failed systemd units via `systemctl --failed`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FailedUnitsCollector;

impl FailedUnitsCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for FailedUnitsCollector {
    type Output = Vec<ServiceUnit>;

    fn module(&self) -> ModuleId {
        ModuleId::Services
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<ServiceUnit>> {
        let spec = CommandSpec::new("systemctl").args([
            "--failed",
            "--no-legend",
            "--plain",
            "--no-pager",
        ]);
        let output = transport.run(&spec).await?.into_result("systemctl")?;
        Ok(parse_units(&output.stdout))
    }
}

/// A unit-file row from `systemctl list-unit-files` (the install state, which
/// `list-units` does not carry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitFile {
    pub name: String,
    pub state: String,
}

impl UnitFile {
    /// Whether the unit is enabled to start at boot (`enabled` / `enabled-runtime`).
    pub fn is_enabled(&self) -> bool {
        self.state == "enabled" || self.state == "enabled-runtime"
    }
}

/// Lists service unit files and their install state via
/// `systemctl list-unit-files`. Slow-changing: it answers the
/// enabled/disabled view that `list-units` cannot.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnitFilesCollector;

impl UnitFilesCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for UnitFilesCollector {
    type Output = Vec<UnitFile>;

    fn module(&self) -> ModuleId {
        ModuleId::Services
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<UnitFile>> {
        let spec = CommandSpec::new("systemctl").args([
            "list-unit-files",
            "--type=service",
            "--no-legend",
            "--plain",
            "--no-pager",
        ]);
        let output = transport.run(&spec).await?.into_result("systemctl")?;
        Ok(parse_unit_files(&output.stdout))
    }
}

/// Read the detailed state of one unit via `systemctl show`.
pub async fn unit_detail(transport: &dyn Transport, unit: &str) -> Result<UnitDetail> {
    let spec = CommandSpec::new("systemctl").args([
        "show",
        unit,
        "--property=Id,Description,LoadState,ActiveState,SubState,MainPID,UnitFileState,FragmentPath",
        "--no-pager",
    ]);
    let output = transport.run(&spec).await?.into_result("systemctl")?;
    Ok(parse_show(&output.stdout))
}

/// List a unit's dependencies via `systemctl list-dependencies`. Read-only.
pub async fn unit_dependencies(transport: &dyn Transport, unit: &str) -> Result<Vec<String>> {
    let spec =
        CommandSpec::new("systemctl").args(["list-dependencies", unit, "--plain", "--no-pager"]);
    let output = transport.run(&spec).await?.into_result("systemctl")?;
    Ok(parse_dependencies(&output.stdout, unit))
}

/// Parse `list-dependencies` output into a flat, de-duplicated unit list,
/// dropping the unit itself and any tree/bullet glyphs that survive `--plain`.
fn parse_dependencies(s: &str, unit: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in s.lines() {
        let name = line
            .trim_start_matches(|c: char| c.is_whitespace() || "●○•│├└─".contains(c))
            .trim();
        if name.is_empty() || !name.contains('.') || name == unit {
            continue;
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
}

fn parse_units(s: &str) -> Vec<ServiceUnit> {
    s.lines().filter_map(parse_unit_line).collect()
}

fn parse_unit_line(line: &str) -> Option<ServiceUnit> {
    let mut parts: Vec<&str> = line.split_whitespace().collect();
    // Drop a leading status bullet ("●") if present.
    if parts.first().is_some_and(|p| !p.contains('.')) && parts.len() > 5 {
        parts.remove(0);
    }
    if parts.len() < 4 {
        return None;
    }
    Some(ServiceUnit {
        name: parts[0].to_owned(),
        load: parts[1].to_owned(),
        active: parts[2].to_owned(),
        sub: parts[3].to_owned(),
        description: parts[4..].join(" "),
    })
}

fn parse_unit_files(s: &str) -> Vec<UnitFile> {
    s.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let state = parts.next()?;
            if !name.contains('.') {
                return None;
            }
            Some(UnitFile {
                name: name.to_owned(),
                state: state.to_owned(),
            })
        })
        .collect()
}

fn parse_show(s: &str) -> UnitDetail {
    let mut detail = UnitDetail::default();
    for line in s.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Id" => detail.name = value.to_owned(),
            "Description" => detail.description = value.to_owned(),
            "LoadState" => detail.load_state = value.to_owned(),
            "ActiveState" => detail.active_state = value.to_owned(),
            "SubState" => detail.sub_state = value.to_owned(),
            "MainPID" => detail.main_pid = value.parse().ok().filter(|&p| p != 0),
            "UnitFileState" => detail.unit_file_state = value.to_owned(),
            "FragmentPath" => detail.fragment_path = value.to_owned(),
            _ => {}
        }
    }
    detail
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use systui_testkit::fuzz::messy_output;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn service_parsers_never_panic(s in messy_output()) {
            let _ = parse_units(&s);
            let _ = parse_unit_files(&s);
            let _ = parse_show(&s);
            let _ = parse_dependencies(&s, "x.service");
            for line in s.lines() {
                let _ = parse_unit_line(line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    const FAILED_CMD: &str = "systemctl --failed --no-legend --plain --no-pager";
    const LIST_CMD: &str =
        "systemctl list-units --type=service --all --no-legend --plain --no-pager";

    #[test]
    fn parses_failed_units() {
        let units = parse_units(include_str!("../fixtures/systemctl-failed.txt"));
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].name, "nginx.service");
        assert!(units[0].is_failed());
        assert_eq!(
            units[0].description,
            "A high performance web server and a reverse proxy server"
        );
    }

    #[test]
    fn parses_full_unit_list() {
        let units = parse_units(include_str!("../fixtures/systemctl-list-units.txt"));
        assert_eq!(units.len(), 4);
        let docker = units.iter().find(|u| u.name == "docker.service").unwrap();
        assert!(docker.is_failed());
        let nginx = units.iter().find(|u| u.name == "nginx.service").unwrap();
        assert_eq!(nginx.active, "active");
        assert!(!nginx.is_failed());
    }

    #[test]
    fn parses_unit_files_and_enabled_state() {
        let files = parse_unit_files(include_str!("../fixtures/systemctl-unit-files.txt"));
        assert_eq!(files.len(), 8);
        let enabled: Vec<&str> = files
            .iter()
            .filter(|f| f.is_enabled())
            .map(|f| f.name.as_str())
            .collect();
        assert!(enabled.contains(&"nginx.service"));
        assert!(enabled.contains(&"unattended-upgrades.service")); // enabled-runtime
        assert!(!enabled.contains(&"bluetooth.service")); // disabled
        assert!(!enabled.contains(&"getty@.service")); // static
    }

    #[test]
    fn parses_unit_detail() {
        let detail = parse_show(include_str!("../fixtures/systemctl-show.txt"));
        assert_eq!(detail.name, "nginx.service");
        assert_eq!(detail.active_state, "active");
        assert_eq!(detail.main_pid, Some(1132));
        assert_eq!(detail.unit_file_state, "enabled");
        assert_eq!(detail.fragment_path, "/lib/systemd/system/nginx.service");
    }

    #[test]
    fn parses_dependencies_dropping_self_and_glyphs() {
        let out = "nginx.service\n● ├─system.slice\n  ├─network.target\n  └─basic.target\n  network.target\n";
        let deps = parse_dependencies(out, "nginx.service");
        assert_eq!(deps, ["system.slice", "network.target", "basic.target"]);
    }

    #[test]
    fn main_pid_zero_is_none() {
        let detail = parse_show("Id=x.service\nMainPID=0\n");
        assert_eq!(detail.main_pid, None);
    }

    #[tokio::test]
    async fn collectors_read_units() {
        let transport = MockTransport::new()
            .with_stdout(FAILED_CMD, include_str!("../fixtures/systemctl-failed.txt"))
            .with_stdout(
                LIST_CMD,
                include_str!("../fixtures/systemctl-list-units.txt"),
            );
        assert_eq!(
            FailedUnitsCollector::new()
                .collect(&transport)
                .await
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            ServiceCollector::new()
                .collect(&transport)
                .await
                .unwrap()
                .len(),
            4
        );
    }

    #[tokio::test]
    async fn reads_unit_detail() {
        let cmd = "systemctl show nginx.service --property=Id,Description,LoadState,ActiveState,SubState,MainPID,UnitFileState,FragmentPath --no-pager";
        let transport =
            MockTransport::new().with_stdout(cmd, include_str!("../fixtures/systemctl-show.txt"));
        let detail = unit_detail(&transport, "nginx.service").await.unwrap();
        assert_eq!(detail.main_pid, Some(1132));
    }
}
