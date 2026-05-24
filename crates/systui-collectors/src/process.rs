//! Process collector: a snapshot of running processes with CPU/memory usage,
//! read via `ps`. Sorting and top-N selection are left to the caller (the UI).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A single running process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub command: String,
}

/// Collects the process list via `ps`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessCollector;

impl ProcessCollector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Collector for ProcessCollector {
    type Output = Vec<Process>;

    fn module(&self) -> ModuleId {
        ModuleId::Processes
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<Process>> {
        let spec = CommandSpec::new("ps").args(["-eo", "pid,user,pcpu,pmem,comm"]);
        let output = transport.run(&spec).await?.into_result("ps")?;
        Ok(parse_ps(&output.stdout))
    }
}

fn parse_ps(s: &str) -> Vec<Process> {
    s.lines().filter_map(parse_ps_line).collect()
}

fn parse_ps_line(line: &str) -> Option<Process> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    // The header row's first field ("PID") fails to parse and is skipped.
    let pid = parts[0].parse().ok()?;
    Some(Process {
        pid,
        user: parts[1].to_owned(),
        cpu_percent: parts[2].parse().unwrap_or(0.0),
        mem_percent: parts[3].parse().unwrap_or(0.0),
        command: parts[4..].join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    #[test]
    fn parses_ps_output_and_skips_header() {
        let procs = parse_ps(include_str!("../fixtures/ps.txt"));
        assert_eq!(procs.len(), 6);
        let nginx = procs.iter().find(|p| p.command == "nginx").unwrap();
        assert_eq!(nginx.pid, 1132);
        assert_eq!(nginx.user, "www-data");
        assert_eq!(nginx.cpu_percent, 5.6);
        assert_eq!(nginx.mem_percent, 3.1);
    }

    #[tokio::test]
    async fn collector_reads_processes_over_mock_transport() {
        let transport = MockTransport::new().with_stdout(
            "ps -eo pid,user,pcpu,pmem,comm",
            include_str!("../fixtures/ps.txt"),
        );
        let procs = ProcessCollector::new().collect(&transport).await.unwrap();
        assert_eq!(procs.len(), 6);
        assert!(
            procs
                .iter()
                .any(|p| p.command == "postgres" && p.mem_percent == 4.2)
        );
    }
}
