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

/// Slow-changing host identity: hostname, OS and kernel. Read once and cached so
/// later refreshes skip the `uname` calls and the `/etc/os-release` read (three
/// round-trips that effectively never change within a session). See [`tiering`].
///
/// [`tiering`]: SystemCollector::with_statics
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostStatics {
    pub hostname: String,
    pub os: Option<String>,
    pub kernel: String,
}

impl HostStatics {
    /// Extract the cacheable identity from an already-collected snapshot.
    pub fn from_snapshot(snapshot: &SystemSnapshot) -> Self {
        Self {
            hostname: snapshot.hostname.clone(),
            os: snapshot.os.clone(),
            kernel: snapshot.kernel.clone(),
        }
    }
}

/// Collects a [`SystemSnapshot`]. CPU usage is sampled twice over a short delay.
///
/// If constructed with [`SystemCollector::with_statics`], the slow-changing
/// identity ([`HostStatics`]) is reused instead of re-read, so a refresh skips
/// `uname -n`, `uname -r` and the `/etc/os-release` read.
#[derive(Debug, Clone)]
pub struct SystemCollector {
    cpu_sample_delay: Duration,
    statics: Option<HostStatics>,
}

impl Default for SystemCollector {
    fn default() -> Self {
        Self {
            cpu_sample_delay: Duration::from_millis(200),
            statics: None,
        }
    }
}

impl SystemCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reuse cached slow-changing identity when present (tiered refresh); pass
    /// `None` to read it fresh.
    pub fn with_statics(statics: Option<HostStatics>) -> Self {
        Self {
            statics,
            ..Self::default()
        }
    }
}

#[async_trait]
impl Collector for SystemCollector {
    type Output = SystemSnapshot;

    fn module(&self) -> ModuleId {
        ModuleId::System
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<SystemSnapshot> {
        // Slow-changing identity: reuse the cache when tiered, else read it.
        // `hostname`/`kernel` are required (their read failing fails the snapshot);
        // `os` is best-effort.
        let (hostname, kernel, os) = match &self.statics {
            Some(s) => (s.hostname.clone(), s.kernel.clone(), s.os.clone()),
            None => {
                let hostname = run_trimmed(transport, "uname", &["-n"]).await?;
                let kernel = run_trimmed(transport, "uname", &["-r"]).await?;
                let os = read_text(transport, "/etc/os-release")
                    .await
                    .ok()
                    .and_then(|s| parse_os_release(&s));
                (hostname, kernel, os)
            }
        };

        // Live metrics: failures here fail the snapshot. The four /proc files are
        // read in a single `tail -v` command (one round-trip instead of four,
        // which compounds with SSH multiplexing); falls back to per-file reads if
        // the batch command is unavailable.
        let (uptime_txt, loadavg_txt, meminfo_txt, stat_txt) =
            match read_proc_batch(transport).await {
                Some(batch) => (batch.uptime, batch.loadavg, batch.meminfo, batch.stat),
                None => (
                    read_text(transport, "/proc/uptime").await?,
                    read_text(transport, "/proc/loadavg").await?,
                    read_text(transport, "/proc/meminfo").await?,
                    read_text(transport, "/proc/stat").await?,
                ),
            };
        let uptime_secs = parse_uptime(&uptime_txt)?;
        let load = parse_loadavg(&loadavg_txt)?;
        let (memory, swap) = parse_meminfo(&meminfo_txt)?;

        // CPU: two samples of /proc/stat with a short delay between them. The
        // second sample must follow the delay, so it stays a separate read.
        let first = parse_proc_stat(&stat_txt)?;
        tokio::time::sleep(self.cpu_sample_delay).await;
        let second = parse_proc_stat(&read_text(transport, "/proc/stat").await?)?;
        let cpu = cpu_usage(first, second);

        // Best-effort metrics: degrade to defaults rather than failing.
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

/// The four live `/proc` files read in one shot.
struct ProcBatch {
    uptime: String,
    loadavg: String,
    meminfo: String,
    stat: String,
}

/// The `/proc` files read by [`ProcBatch`], in `tail` argument order.
const PROC_BATCH_FILES: [&str; 4] = [
    "/proc/uptime",
    "/proc/loadavg",
    "/proc/meminfo",
    "/proc/stat",
];

/// Read the four live `/proc` files in a single `tail -n +1 -v` round-trip.
/// Returns `None` (so the caller falls back to per-file reads) if the command is
/// unavailable, fails, or its output can't be split back into all four files.
async fn read_proc_batch(transport: &dyn Transport) -> Option<ProcBatch> {
    let mut args = vec!["-n", "+1", "-v"];
    args.extend_from_slice(&PROC_BATCH_FILES);
    let spec = CommandSpec::new("tail").args(args);
    let out = transport.run(&spec).await.ok()?;
    if !out.success() {
        return None;
    }
    parse_proc_batch(&out.stdout)
}

/// Split `tail -v` output into its per-file sections, keyed by the `==> path <==`
/// headers it prints before each file. Returns `None` if any expected file is
/// missing from the output.
fn parse_proc_batch(s: &str) -> Option<ProcBatch> {
    let mut sections: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut current: Option<&str> = None;
    let mut buf = String::new();
    for line in s.lines() {
        if let Some(path) = line
            .strip_prefix("==> ")
            .and_then(|rest| rest.strip_suffix(" <=="))
        {
            if let Some(prev) = current.take() {
                sections.insert(prev, std::mem::take(&mut buf));
            }
            current = Some(path);
        } else {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(prev) = current.take() {
        sections.insert(prev, buf);
    }
    Some(ProcBatch {
        uptime: sections.remove("/proc/uptime")?,
        loadavg: sections.remove("/proc/loadavg")?,
        meminfo: sections.remove("/proc/meminfo")?,
        stat: sections.remove("/proc/stat")?,
    })
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

/// Pseudo filesystem device names skipped in the disk view (they are not real
/// storage and only add noise).
const PSEUDO_FS: &[&str] = &[
    "tmpfs",
    "devtmpfs",
    "none",
    "efivarfs",
    "overlay",
    "squashfs",
    "ramfs",
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "mqueue",
    "hugetlbfs",
    "debugfs",
    "tracefs",
];

fn parse_df(s: &str) -> Vec<Disk> {
    s.lines()
        .skip(1)
        .filter_map(parse_df_line)
        .filter(|d| !PSEUDO_FS.contains(&d.filesystem.as_str()))
        .collect()
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
    fn parses_df_and_skips_pseudo_filesystems() {
        // fixture has /dev/sda1, tmpfs, /dev/sda2 — tmpfs is filtered out.
        let disks = parse_df(include_str!("../fixtures/df-P.txt"));
        assert_eq!(disks.len(), 2);
        let root = &disks[0];
        assert_eq!(root.mount, "/");
        assert_eq!(root.use_percent, 89);
        assert_eq!(disks[1].mount, "/home");
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
            statics: None,
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.hostname, "prod-01");
        assert_eq!(snap.kernel, "6.1.0-18-amd64");
        assert_eq!(snap.os.as_deref(), Some("Debian GNU/Linux 12 (bookworm)"));
        assert_eq!(snap.uptime_secs, 123_456);
        assert_eq!(snap.load.one, 0.52);
        assert_eq!(snap.memory.total_kb, 16_327_680);
        assert_eq!(snap.disks.len(), 2); // tmpfs filtered out
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
            statics: None,
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.os, None);
        assert!(snap.disks.is_empty());
        assert!(snap.users.is_empty());
        assert_eq!(snap.cpu.cores, 1);
    }

    #[tokio::test]
    async fn cached_statics_skip_identity_reads() {
        use systui_transport::MockTransport;

        // Only the live /proc data is configured — no `uname` and no
        // `/etc/os-release`. A fresh collect would fail on the missing `uname -n`;
        // with cached statics the identity is reused and those reads are skipped.
        let transport = MockTransport::new()
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
                "cpu  1 0 1 8 0 0 0 0 0 0\n".as_bytes().to_vec(),
            );

        let statics = HostStatics {
            hostname: "cached-host".to_owned(),
            os: Some("Cached OS 1.0".to_owned()),
            kernel: "9.9.9".to_owned(),
        };
        let collector = SystemCollector {
            cpu_sample_delay: Duration::ZERO,
            statics: Some(statics),
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.hostname, "cached-host");
        assert_eq!(snap.kernel, "9.9.9");
        assert_eq!(snap.os.as_deref(), Some("Cached OS 1.0"));
        // Live data was still collected fresh.
        assert_eq!(snap.uptime_secs, 10);
        assert_eq!(snap.memory.total_kb, 100);
    }

    #[tokio::test]
    async fn reads_proc_in_one_batched_command() {
        use systui_transport::MockTransport;

        // The four live /proc files arrive in one `tail -v` response; none of them
        // is configured as an individual file, so a successful parse proves the
        // batch path supplied them. Only the post-delay /proc/stat re-read is a
        // separate file read.
        let batched = "==> /proc/uptime <==\n\
             123456.78 654321.00\n\
             \n\
             ==> /proc/loadavg <==\n\
             0.52 0.58 0.59 2/1234 56789\n\
             \n\
             ==> /proc/meminfo <==\n\
             MemTotal:       16327680 kB\n\
             MemAvailable:    8000000 kB\n\
             \n\
             ==> /proc/stat <==\n\
             cpu  1 0 1 8 0 0 0 0 0 0\n\
             cpu0 1 0 1 8 0 0 0 0 0 0\n";
        let transport = MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0\n")
            .with_stdout(
                "tail -n +1 -v /proc/uptime /proc/loadavg /proc/meminfo /proc/stat",
                batched,
            )
            .with_file(
                "/proc/stat",
                "cpu  1 0 1 8 0 0 0 0 0 0\ncpu0 1 0 1 8 0 0 0 0 0 0\n"
                    .as_bytes()
                    .to_vec(),
            );

        let collector = SystemCollector {
            cpu_sample_delay: Duration::ZERO,
            statics: None,
        };
        let snap = collector.collect(&transport).await.unwrap();

        assert_eq!(snap.hostname, "prod-01");
        assert_eq!(snap.uptime_secs, 123_456);
        assert_eq!(snap.load.one, 0.52);
        assert_eq!(snap.memory.total_kb, 16_327_680);
        assert_eq!(snap.cpu.cores, 1);
    }

    #[test]
    fn parse_proc_batch_splits_on_headers() {
        let out = "==> /proc/uptime <==\n1.0 2.0\n==> /proc/loadavg <==\n0 0 0 1/1 1\n\
             ==> /proc/meminfo <==\nMemTotal: 1 kB\n==> /proc/stat <==\ncpu 1 2 3\n";
        let batch = parse_proc_batch(out).expect("all four sections present");
        assert_eq!(batch.uptime.trim(), "1.0 2.0");
        assert_eq!(batch.loadavg.trim(), "0 0 0 1/1 1");
        assert_eq!(batch.meminfo.trim(), "MemTotal: 1 kB");
        assert_eq!(batch.stat.trim(), "cpu 1 2 3");
        // A response missing a file degrades to None (caller falls back).
        assert!(parse_proc_batch("==> /proc/uptime <==\n1.0 2.0\n").is_none());
    }
}
