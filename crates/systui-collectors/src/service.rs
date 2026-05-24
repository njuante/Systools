//! systemd service collector. For v0.1 this surfaces *failed* units via
//! `systemctl --failed`; the full service module (list/filter/detail/actions)
//! arrives in v0.2.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A systemd unit in a failed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedUnit {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
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
    type Output = Vec<FailedUnit>;

    fn module(&self) -> ModuleId {
        ModuleId::Services
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<FailedUnit>> {
        let spec = CommandSpec::new("systemctl").args([
            "--failed",
            "--no-legend",
            "--plain",
            "--no-pager",
        ]);
        let output = transport.run(&spec).await?.into_result("systemctl")?;
        Ok(parse_failed_units(&output.stdout))
    }
}

fn parse_failed_units(s: &str) -> Vec<FailedUnit> {
    s.lines().filter_map(parse_failed_unit_line).collect()
}

fn parse_failed_unit_line(line: &str) -> Option<FailedUnit> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    Some(FailedUnit {
        unit: parts[0].to_owned(),
        load: parts[1].to_owned(),
        active: parts[2].to_owned(),
        sub: parts[3].to_owned(),
        description: parts[4..].join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    const CMD: &str = "systemctl --failed --no-legend --plain --no-pager";

    #[test]
    fn parses_failed_units() {
        let units = parse_failed_units(include_str!("../fixtures/systemctl-failed.txt"));
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].unit, "nginx.service");
        assert_eq!(units[0].active, "failed");
        assert_eq!(
            units[0].description,
            "A high performance web server and a reverse proxy server"
        );
        assert_eq!(units[2].unit, "backup.timer");
    }

    #[test]
    fn empty_output_means_no_failures() {
        assert!(parse_failed_units("").is_empty());
    }

    #[tokio::test]
    async fn collector_reads_failed_units() {
        let transport =
            MockTransport::new().with_stdout(CMD, include_str!("../fixtures/systemctl-failed.txt"));
        let units = FailedUnitsCollector::new()
            .collect(&transport)
            .await
            .unwrap();
        assert_eq!(units.len(), 3);
    }
}
