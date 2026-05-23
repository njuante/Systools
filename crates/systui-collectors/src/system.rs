//! System metrics collector: OS/kernel/hostname/uptime, CPU, memory, swap, load,
//! disks and logged-in users, assembled into a [`SystemSnapshot`].
//!
//! All data is read agentlessly through a [`Transport`]: `/proc` files via
//! `read_file`, and `df`/`who`/`uname` via `CommandSpec`. Parsers are pure
//! functions covered by fixture tests.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, CoreError, ModuleId, Result, Transport};

/// A point-in-time view of core system metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub hostname: String,
    pub os: Option<String>,
    pub kernel: String,
    pub uptime_secs: u64,
    pub load: LoadAverage,
    pub cpu: CpuUsage,
    pub memory: Memory,
    pub swap: Swap,
    pub disks: Vec<Disk>,
    pub users: Vec<LoggedUser>,
}

/// 1/5/15-minute load averages.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// Aggregate CPU busy percentage and core count.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CpuUsage {
    pub busy_percent: f64,
    pub cores: usize,
}

/// RAM totals (kB). "Used" is derived from available memory.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Memory {
    pub total_kb: u64,
    pub available_kb: u64,
}

impl Memory {
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.available_kb)
    }

    pub fn used_percent(&self) -> f64 {
        percent(self.used_kb(), self.total_kb)
    }
}

/// Swap totals (kB).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Swap {
    pub total_kb: u64,
    pub free_kb: u64,
}

impl Swap {
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.free_kb)
    }

    pub fn used_percent(&self) -> f64 {
        percent(self.used_kb(), self.total_kb)
    }
}

/// A mounted filesystem and its usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    pub filesystem: String,
    pub size_kb: u64,
    pub used_kb: u64,
    pub avail_kb: u64,
    pub use_percent: u8,
    pub mount: String,
}

/// A logged-in user session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedUser {
    pub name: String,
    pub tty: String,
    pub from: Option<String>,
    pub login_time: String,
}

/// Collects a [`SystemSnapshot`]. CPU usage is sampled twice over a short delay.
#[derive(Debug, Clone, Copy)]
pub struct SystemCollector {
    cpu_sample_delay: Duration,
}

impl Default for SystemCollector {
    fn default() -> Self {
        Self {
            cpu_sample_delay: Duration::from_millis(200),
        }
    }
}

impl SystemCollector {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Collector for SystemCollector {
    type Output = SystemSnapshot;

    fn module(&self) -> ModuleId {
        ModuleId::System
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<SystemSnapshot> {
        // Core metrics: failures here fail the snapshot.
        let hostname = run_trimmed(transport, "uname", &["-n"]).await?;
        let kernel = run_trimmed(transport, "uname", &["-r"]).await?;
        let uptime_secs = parse_uptime(&read_text(transport, "/proc/uptime").await?)?;
        let load = parse_loadavg(&read_text(transport, "/proc/loadavg").await?)?;
        let (memory, swap) = parse_meminfo(&read_text(transport, "/proc/meminfo").await?)?;

        // CPU: two samples of /proc/stat with a short delay between them.
        let first = parse_proc_stat(&read_text(transport, "/proc/stat").await?)?;
        tokio::time::sleep(self.cpu_sample_delay).await;
        let second = parse_proc_stat(&read_text(transport, "/proc/stat").await?)?;
        let cpu = cpu_usage(first, second);

        // Best-effort metrics: degrade to defaults rather than failing.
        let os = read_text(transport, "/etc/os-release")
            .await
            .ok()
            .and_then(|s| parse_os_release(&s));
        let disks = match transport.run(&CommandSpec::new("df").arg("-P")).await {
            Ok(out) if out.success() => parse_df(&out.stdout),
            _ => Vec::new(),
        };
        let users = match transport.run(&CommandSpec::new("who")).await {
            Ok(out) if out.success() => parse_who(&out.stdout),
            _ => Vec::new(),
        };

        Ok(SystemSnapshot {
            hostname,
            os,
            kernel,
            uptime_secs,
            load,
            cpu,
            memory,
            swap,
            disks,
            users,
        })
    }
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

async fn read_text(transport: &dyn Transport, path: &str) -> Result<String> {
    let bytes = transport.read_file(path).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn run_trimmed(transport: &dyn Transport, program: &str, args: &[&str]) -> Result<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    let output = transport.run(&spec).await?.into_result(program)?;
    Ok(output.stdout.trim().to_owned())
}

/// Cumulative CPU times read from the aggregate `/proc/stat` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    idle: u64,
    total: u64,
    cores: usize,
}

fn parse_proc_stat(s: &str) -> Result<CpuTimes> {
    let mut cores = 0;
    let mut aggregate = None;

    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("cpu") {
            if rest.starts_with(char::is_numeric) {
                cores += 1;
            } else if rest.starts_with(char::is_whitespace) {
                let fields: Vec<u64> = rest
                    .split_whitespace()
                    .map(|f| f.parse().unwrap_or(0))
                    .collect();
                // user nice system idle iowait irq softirq steal guest guest_nice
                if fields.len() < 4 {
                    return Err(CoreError::parse("/proc/stat", "too few cpu fields"));
                }
                let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
                let total: u64 = fields.iter().sum();
                aggregate = Some((idle, total));
            }
        }
    }

    let (idle, total) =
        aggregate.ok_or_else(|| CoreError::parse("/proc/stat", "missing aggregate cpu line"))?;
    Ok(CpuTimes { idle, total, cores })
}

fn cpu_usage(prev: CpuTimes, cur: CpuTimes) -> CpuUsage {
    let total_delta = cur.total.saturating_sub(prev.total);
    let idle_delta = cur.idle.saturating_sub(prev.idle);
    let busy = if total_delta == 0 {
        0.0
    } else {
        (1.0 - idle_delta as f64 / total_delta as f64) * 100.0
    };
    CpuUsage {
        busy_percent: busy.clamp(0.0, 100.0),
        cores: cur.cores,
    }
}

fn parse_uptime(s: &str) -> Result<u64> {
    s.split_whitespace()
        .next()
        .and_then(|t| t.parse::<f64>().ok())
        .map(|secs| secs as u64)
        .ok_or_else(|| CoreError::parse("/proc/uptime", "missing uptime field"))
}

fn parse_loadavg(s: &str) -> Result<LoadAverage> {
    let mut it = s.split_whitespace();
    let mut next = || it.next().and_then(|t| t.parse::<f64>().ok());
    match (next(), next(), next()) {
        (Some(one), Some(five), Some(fifteen)) => Ok(LoadAverage { one, five, fifteen }),
        _ => Err(CoreError::parse("/proc/loadavg", "missing load fields")),
    }
}

fn parse_meminfo(s: &str) -> Result<(Memory, Swap)> {
    let mut total = None;
    let mut available = None;
    let mut free = None;
    let mut swap_total = 0;
    let mut swap_free = 0;

    for line in s.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let value = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        match key.trim() {
            "MemTotal" => total = value,
            "MemAvailable" => available = value,
            "MemFree" => free = value,
            "SwapTotal" => swap_total = value.unwrap_or(0),
            "SwapFree" => swap_free = value.unwrap_or(0),
            _ => {}
        }
    }

    let total_kb = total.ok_or_else(|| CoreError::parse("/proc/meminfo", "missing MemTotal"))?;
    let available_kb = available.or(free).unwrap_or(0);

    Ok((
        Memory {
            total_kb,
            available_kb,
        },
        Swap {
            total_kb: swap_total,
            free_kb: swap_free,
        },
    ))
}

fn parse_os_release(s: &str) -> Option<String> {
    for line in s.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn parse_df(s: &str) -> Vec<Disk> {
    s.lines().skip(1).filter_map(parse_df_line).collect()
}

fn parse_df_line(line: &str) -> Option<Disk> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }
    Some(Disk {
        filesystem: parts[0].to_owned(),
        size_kb: parts[1].parse().ok()?,
        used_kb: parts[2].parse().ok()?,
        avail_kb: parts[3].parse().ok()?,
        use_percent: parts[4].trim_end_matches('%').parse().ok()?,
        mount: parts[5..].join(" "),
    })
}

fn parse_who(s: &str) -> Vec<LoggedUser> {
    s.lines().filter_map(parse_who_line).collect()
}

fn parse_who_line(line: &str) -> Option<LoggedUser> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let from = parts
        .iter()
        .find(|p| p.starts_with('(') && p.ends_with(')'))
        .map(|p| p.trim_matches(|c| c == '(' || c == ')').to_owned());
    let login_time = parts[2..]
        .iter()
        .filter(|p| !p.starts_with('('))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    Some(LoggedUser {
        name: parts[0].to_owned(),
        tty: parts[1].to_owned(),
        from,
        login_time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_usage_from_two_samples() {
        let first = parse_proc_stat(include_str!("../fixtures/proc-stat-1.txt")).unwrap();
        let second = parse_proc_stat(include_str!("../fixtures/proc-stat-2.txt")).unwrap();
        assert_eq!(first.cores, 4);
        let usage = cpu_usage(first, second);
        assert_eq!(usage.cores, 4);
        // total delta 258, idle delta 185 -> ~28.3% busy
        assert!(
            (usage.busy_percent - 28.29).abs() < 0.1,
            "{}",
            usage.busy_percent
        );
    }

    #[test]
    fn cpu_usage_handles_zero_delta() {
        let same = parse_proc_stat(include_str!("../fixtures/proc-stat-1.txt")).unwrap();
        assert_eq!(cpu_usage(same, same).busy_percent, 0.0);
    }

    #[test]
    fn parses_meminfo() {
        let (mem, swap) = parse_meminfo(include_str!("../fixtures/proc-meminfo.txt")).unwrap();
        assert_eq!(mem.total_kb, 16_327_680);
        assert_eq!(mem.available_kb, 9_876_543);
        assert_eq!(mem.used_kb(), 16_327_680 - 9_876_543);
        assert_eq!(swap.total_kb, 4_194_304);
        assert_eq!(swap.used_kb(), 4_194_304 - 3_145_728);
    }

    #[test]
    fn meminfo_requires_total() {
        assert!(parse_meminfo("Foo: 1 kB\n").is_err());
    }

    #[test]
    fn parses_loadavg() {
        let load = parse_loadavg("0.52 0.58 0.59 2/1234 56789\n").unwrap();
        assert_eq!(load.one, 0.52);
        assert_eq!(load.fifteen, 0.59);
    }

    #[test]
    fn parses_uptime() {
        assert_eq!(parse_uptime("123456.78 654321.00\n").unwrap(), 123_456);
    }

    #[test]
    fn parses_os_release_pretty_name() {
        let os = parse_os_release(include_str!("../fixtures/os-release.txt"));
        assert_eq!(os.as_deref(), Some("Debian GNU/Linux 12 (bookworm)"));
    }

    #[test]
    fn parses_df() {
        let disks = parse_df(include_str!("../fixtures/df-P.txt"));
        assert_eq!(disks.len(), 3);
        let root = &disks[0];
        assert_eq!(root.mount, "/");
        assert_eq!(root.use_percent, 89);
        assert_eq!(disks[2].mount, "/home");
    }

    #[test]
    fn parses_who() {
        let users = parse_who(include_str!("../fixtures/who.txt"));
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].name, "admin");
        assert_eq!(users[0].from.as_deref(), Some("10.0.0.5"));
        assert_eq!(users[1].name, "root");
        assert_eq!(users[1].from, None);
    }

    #[tokio::test]
    async fn collector_assembles_snapshot_over_mock_transport() {
        use systui_transport::MockTransport;

        let transport = MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0-18-amd64\n")
            .with_file("/proc/uptime", "123456.78 654321.00\n".as_bytes().to_vec())
            .with_file(
                "/proc/loadavg",
                "0.52 0.58 0.59 2/1234 56789\n".as_bytes().to_vec(),
            )
            .with_file(
                "/proc/meminfo",
                include_str!("../fixtures/proc-meminfo.txt")
                    .as_bytes()
                    .to_vec(),
            )
            .with_file(
                "/proc/stat",
                include_str!("../fixtures/proc-stat-1.txt")
                    .as_bytes()
                    .to_vec(),
            )
            .with_file(
                "/etc/os-release",
                include_str!("../fixtures/os-release.txt")
                    .as_bytes()
                    .to_vec(),
            )
            .with_stdout("df -P", include_str!("../fixtures/df-P.txt"))
            .with_stdout("who", include_str!("../fixtures/who.txt"));

        let collector = SystemCollector {
            cpu_sample_delay: Duration::ZERO,
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.hostname, "prod-01");
        assert_eq!(snap.kernel, "6.1.0-18-amd64");
        assert_eq!(snap.os.as_deref(), Some("Debian GNU/Linux 12 (bookworm)"));
        assert_eq!(snap.uptime_secs, 123_456);
        assert_eq!(snap.load.one, 0.52);
        assert_eq!(snap.memory.total_kb, 16_327_680);
        assert_eq!(snap.disks.len(), 3);
        assert_eq!(snap.users.len(), 2);
        // Both /proc/stat reads return the same fixture, so CPU delta is zero.
        assert_eq!(snap.cpu.cores, 4);
        assert_eq!(snap.cpu.busy_percent, 0.0);
    }

    #[tokio::test]
    async fn missing_disks_and_users_degrade_to_empty() {
        use systui_transport::MockTransport;

        let transport = MockTransport::new()
            .with_stdout("uname -n", "host\n")
            .with_stdout("uname -r", "6.0\n")
            .with_file("/proc/uptime", "10.0 5.0\n".as_bytes().to_vec())
            .with_file("/proc/loadavg", "0 0 0 1/1 1\n".as_bytes().to_vec())
            .with_file(
                "/proc/meminfo",
                "MemTotal: 100 kB\nMemAvailable: 50 kB\n"
                    .as_bytes()
                    .to_vec(),
            )
            .with_file(
                "/proc/stat",
                "cpu  1 0 1 8 0 0 0 0 0 0\ncpu0 1 0 1 8 0 0 0 0 0 0\n"
                    .as_bytes()
                    .to_vec(),
            );
        // no os-release, df or who configured

        let collector = SystemCollector {
            cpu_sample_delay: Duration::ZERO,
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.os, None);
        assert!(snap.disks.is_empty());
        assert!(snap.users.is_empty());
        assert_eq!(snap.cpu.cores, 1);
    }
}
