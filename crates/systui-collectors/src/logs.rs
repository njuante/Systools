//! Log collector. Reads recent journald entries via `journalctl -o json`,
//! filtered server-side by priority/unit/time window. Tailing and error
//! grouping arrive in phase 4.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// Server-side journald filter passed to `journalctl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    /// Lowest priority to include (0 = emerg .. 7 = debug); shows this and worse.
    pub min_priority: u8,
    /// Restrict to a single unit (`-u`).
    pub unit: Option<String>,
    /// Time window, e.g. `"1 hour ago"` (`--since`).
    pub since: Option<String>,
    /// Maximum number of lines (`-n`).
    pub lines: usize,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            min_priority: 3, // err and worse
            unit: None,
            since: None,
            lines: 200,
        }
    }
}

/// A single journald entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Local time of day, `HH:MM:SS`.
    pub time: String,
    /// syslog priority (0 = emerg .. 7 = debug).
    pub priority: u8,
    /// Unit, syslog identifier or command that emitted the line.
    pub identifier: String,
    pub message: String,
}

impl LogEntry {
    /// Short uppercase priority label.
    pub fn priority_label(&self) -> &'static str {
        match self.priority {
            0 => "EMERG",
            1 => "ALERT",
            2 => "CRIT",
            3 => "ERR",
            4 => "WARN",
            5 => "NOTICE",
            6 => "INFO",
            _ => "DEBUG",
        }
    }

    /// Whether this entry is error severity or worse.
    pub fn is_error(&self) -> bool {
        self.priority <= 3
    }
}

/// Collects journald logs matching a [`LogQuery`].
#[derive(Debug, Default, Clone)]
pub struct LogsCollector {
    query: LogQuery,
}

impl LogsCollector {
    /// A collector with the default query (errors, last 200 lines).
    pub fn new() -> Self {
        Self::default()
    }

    /// A collector with a custom query.
    pub fn with_query(query: LogQuery) -> Self {
        Self { query }
    }
}

#[async_trait]
impl Collector for LogsCollector {
    type Output = Vec<LogEntry>;

    fn module(&self) -> ModuleId {
        ModuleId::Logs
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<Vec<LogEntry>> {
        let spec = CommandSpec::new("journalctl").args(build_args(&self.query));
        let output = transport.run(&spec).await?.into_result("journalctl")?;
        Ok(parse_journal_json(&output.stdout))
    }
}

/// Build the `journalctl` argument list for a query.
fn build_args(query: &LogQuery) -> Vec<String> {
    let mut args = vec![
        "-p".to_owned(),
        query.min_priority.to_string(),
        "-n".to_owned(),
        query.lines.to_string(),
        "-o".to_owned(),
        "json".to_owned(),
        "--no-pager".to_owned(),
    ];
    if let Some(unit) = &query.unit {
        args.push("-u".to_owned());
        args.push(unit.clone());
    }
    if let Some(since) = &query.since {
        args.push("--since".to_owned());
        args.push(since.clone());
    }
    args
}

fn parse_journal_json(s: &str) -> Vec<LogEntry> {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_journal_line)
        .collect()
}

fn parse_journal_line(line: &str) -> Option<LogEntry> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;

    let message = obj
        .get("MESSAGE")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    let priority = obj
        .get("PRIORITY")
        .and_then(|p| p.as_str())
        .and_then(|p| p.parse().ok())
        .unwrap_or(6);
    let identifier = obj
        .get("SYSLOG_IDENTIFIER")
        .or_else(|| obj.get("_SYSTEMD_UNIT"))
        .or_else(|| obj.get("_COMM"))
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_owned();
    let time = obj
        .get("__REALTIME_TIMESTAMP")
        .and_then(|x| x.as_str())
        .and_then(|us| us.parse::<u64>().ok())
        .map(format_time_us)
        .unwrap_or_default();

    Some(LogEntry {
        time,
        priority,
        identifier,
        message,
    })
}

/// Format microseconds-since-epoch as local `HH:MM:SS`.
fn format_time_us(us: u64) -> String {
    let secs = (us / 1_000_000) as i64;
    let nanos = ((us % 1_000_000) * 1_000) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_transport::MockTransport;

    const CMD: &str = "journalctl -p 3 -n 200 -o json --no-pager";

    #[test]
    fn build_args_defaults() {
        assert_eq!(
            build_args(&LogQuery::default()).join(" "),
            "-p 3 -n 200 -o json --no-pager"
        );
    }

    #[test]
    fn build_args_with_unit_and_since() {
        let query = LogQuery {
            min_priority: 4,
            unit: Some("nginx.service".to_owned()),
            since: Some("1 hour ago".to_owned()),
            lines: 50,
        };
        assert_eq!(
            build_args(&query).join(" "),
            "-p 4 -n 50 -o json --no-pager -u nginx.service --since 1 hour ago"
        );
    }

    #[test]
    fn parses_journal_entries() {
        let entries = parse_journal_json(include_str!("../fixtures/journalctl-json.txt"));
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].identifier, "nginx");
        assert_eq!(entries[0].priority, 3);
        assert_eq!(entries[0].priority_label(), "ERR");
        assert!(entries[0].message.contains("upstream timed out"));
        // time is a non-empty HH:MM:SS regardless of the machine timezone
        assert_eq!(entries[0].time.len(), 8);

        assert_eq!(entries[1].identifier, "sshd.service");
        assert_eq!(entries[1].priority_label(), "CRIT");
        assert!(entries[1].is_error());

        assert_eq!(entries[2].identifier, "kernel");
    }

    #[test]
    fn skips_malformed_lines() {
        let entries = parse_journal_json("not json\n{\"MESSAGE\":\"ok\",\"PRIORITY\":\"3\"}\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "ok");
        assert_eq!(entries[0].identifier, "?");
    }

    #[tokio::test]
    async fn collector_reads_logs() {
        let transport =
            MockTransport::new().with_stdout(CMD, include_str!("../fixtures/journalctl-json.txt"));
        let entries = LogsCollector::new().collect(&transport).await.unwrap();
        assert_eq!(entries.len(), 3);
    }
}
