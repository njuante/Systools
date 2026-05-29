//! Process collector: the process list (with parent PIDs for tree building),
//! a flattened process tree and per-process detail. Read via `ps` and `/proc`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// A single running process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    /// Resident set size in kB (from `ps rss`). `0` when unavailable.
    #[serde(default)]
    pub rss_kb: u64,
    pub command: String,
}

/// Detailed state of a single process, from `/proc/<pid>`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProcessDetail {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub state: String,
    pub rss_kb: Option<u64>,
    pub cmdline: String,
}

/// A row in a flattened process tree: depth plus an index into the source slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: u16,
    pub index: usize,
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
        let spec = CommandSpec::new("ps").args(["-eo", "pid,ppid,user,pcpu,pmem,rss,comm"]);
        let output = transport.run(&spec).await?.into_result("ps")?;
        Ok(parse_ps(&output.stdout))
    }
}

/// Read detail for one process from `/proc/<pid>`.
pub async fn process_detail(transport: &dyn Transport, pid: u32) -> Result<ProcessDetail> {
    let status = transport.read_file(&format!("/proc/{pid}/status")).await?;
    let mut detail = parse_status(&String::from_utf8_lossy(&status));
    detail.pid = pid;

    if let Ok(raw) = transport.read_file(&format!("/proc/{pid}/cmdline")).await {
        detail.cmdline = String::from_utf8_lossy(&raw)
            .split('\0')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    Ok(detail)
}

/// Build a flattened, depth-annotated process tree from a process list.
///
/// Roots are processes whose parent is absent from the list (or PID 0). Cycles
/// are broken via a visited set.
pub fn build_process_tree(procs: &[Process]) -> Vec<TreeRow> {
    use std::collections::{BTreeMap, HashSet};

    let mut index_of: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, p) in procs.iter().enumerate() {
        index_of.insert(p.pid, i);
    }

    let mut children: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (i, p) in procs.iter().enumerate() {
        if p.ppid != 0 && index_of.contains_key(&p.ppid) {
            children.entry(p.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }

    let mut rows = Vec::with_capacity(procs.len());
    let mut visited = HashSet::new();
    let mut stack: Vec<(usize, u16)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
    while let Some((index, depth)) = stack.pop() {
        if !visited.insert(procs[index].pid) {
            continue;
        }
        rows.push(TreeRow { depth, index });
        if let Some(kids) = children.get(&procs[index].pid) {
            for &child in kids.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    rows
}

fn parse_ps(s: &str) -> Vec<Process> {
    s.lines().filter_map(parse_ps_line).collect()
}

fn parse_ps_line(line: &str) -> Option<Process> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 7 {
        return None;
    }
    // The header row's first field ("PID") fails to parse and is skipped.
    let pid = parts[0].parse().ok()?;
    Some(Process {
        pid,
        ppid: parts[1].parse().unwrap_or(0),
        user: parts[2].to_owned(),
        cpu_percent: parts[3].parse().unwrap_or(0.0),
        mem_percent: parts[4].parse().unwrap_or(0.0),
        rss_kb: parts[5].parse().unwrap_or(0),
        command: parts[6..].join(" "),
    })
}

fn parse_status(s: &str) -> ProcessDetail {
    let mut detail = ProcessDetail::default();
    for line in s.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Name" => detail.name = value.to_owned(),
            "State" => detail.state = value.to_owned(),
            "PPid" => detail.ppid = value.parse().unwrap_or(0),
            "VmRSS" => {
                detail.rss_kb = value.split_whitespace().next().and_then(|v| v.parse().ok());
            }
            _ => {}
        }
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    const PS_CMD: &str = "ps -eo pid,ppid,user,pcpu,pmem,rss,comm";

    #[test]
    fn parses_ps_output_and_skips_header() {
        let procs = parse_ps(include_str!("../fixtures/ps.txt"));
        assert_eq!(procs.len(), 6);
        let nginx = procs.iter().find(|p| p.command == "nginx").unwrap();
        assert_eq!(nginx.pid, 1132);
        assert_eq!(nginx.ppid, 842);
        assert_eq!(nginx.user, "www-data");
        assert_eq!(nginx.cpu_percent, 5.6);
        assert_eq!(nginx.rss_kb, 192_800);
    }

    #[test]
    fn builds_a_process_tree() {
        let procs = vec![
            Process {
                pid: 1,
                ppid: 0,
                user: "root".into(),
                cpu_percent: 0.0,
                mem_percent: 0.0,
                rss_kb: 0,
                command: "systemd".into(),
            },
            Process {
                pid: 100,
                ppid: 1,
                user: "root".into(),
                cpu_percent: 0.0,
                mem_percent: 0.0,
                rss_kb: 0,
                command: "sshd".into(),
            },
            Process {
                pid: 200,
                ppid: 100,
                user: "admin".into(),
                cpu_percent: 0.0,
                mem_percent: 0.0,
                rss_kb: 0,
                command: "bash".into(),
            },
            Process {
                pid: 300,
                ppid: 1,
                user: "root".into(),
                cpu_percent: 0.0,
                mem_percent: 0.0,
                rss_kb: 0,
                command: "cron".into(),
            },
        ];
        let tree = build_process_tree(&procs);
        let seq: Vec<(u16, u32)> = tree.iter().map(|r| (r.depth, procs[r.index].pid)).collect();
        assert_eq!(seq, vec![(0, 1), (1, 100), (2, 200), (1, 300)]);
    }

    #[test]
    fn parses_proc_status() {
        let detail =
            parse_status("Name:\tnginx\nState:\tS (sleeping)\nPPid:\t842\nVmRSS:\t20480 kB\n");
        assert_eq!(detail.name, "nginx");
        assert_eq!(detail.state, "S (sleeping)");
        assert_eq!(detail.ppid, 842);
        assert_eq!(detail.rss_kb, Some(20480));
    }

    #[tokio::test]
    async fn collector_reads_processes() {
        let transport =
            MockTransport::new().with_stdout(PS_CMD, include_str!("../fixtures/ps.txt"));
        let procs = ProcessCollector::new().collect(&transport).await.unwrap();
        assert_eq!(procs.len(), 6);
    }

    #[tokio::test]
    async fn reads_process_detail() {
        let transport = MockTransport::new()
            .with_file(
                "/proc/1132/status",
                b"Name:\tnginx\nState:\tS (sleeping)\nPPid:\t842\nVmRSS:\t20480 kB\n".to_vec(),
            )
            .with_file("/proc/1132/cmdline", b"nginx\0-g\0daemon off;\0".to_vec());
        let detail = process_detail(&transport, 1132).await.unwrap();
        assert_eq!(detail.pid, 1132);
        assert_eq!(detail.name, "nginx");
        assert_eq!(detail.cmdline, "nginx -g daemon off;");
    }
}
