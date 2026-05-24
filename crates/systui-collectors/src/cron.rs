//! Cron collectors: gather scheduled jobs from every common source — the current
//! user's crontab (`crontab -l`), `/etc/crontab`, `/etc/cron.d/*` and the
//! `/etc/cron.{hourly,daily,weekly,monthly}` run-parts directories.
//!
//! Parsing is a pure, fixture-tested function. System crontabs (`/etc/crontab`,
//! `cron.d`) carry a user field; user crontabs do not. Schedule validation,
//! next-run preview (S4.6) and the cron risk checks (S4.7) build on these
//! entries; here we only collect and normalise them. Enumerating *other* users'
//! crontabs needs root, so we degrade to the current user plus system locations.

use serde::{Deserialize, Serialize};
use systui_core::{CommandSpec, Transport};

/// Where a cron entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CronSource {
    /// The current user's crontab (`crontab -l`).
    User,
    /// `/etc/crontab`.
    System,
    /// A file under `/etc/cron.d/`.
    CronD,
    /// A script in a `/etc/cron.{hourly,daily,weekly,monthly}` directory.
    Periodic,
}

/// A single scheduled job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronEntry {
    /// Raw schedule: five fields (`m h dom mon dow`) or an `@macro` such as
    /// `@reboot`/`@daily`. Parsed and validated in S4.6.
    pub schedule: String,
    /// The user the job runs as, when the source specifies one.
    pub user: Option<String>,
    pub command: String,
    pub source: CronSource,
    /// The originating path or command, e.g. `/etc/cron.d/backup` or `crontab -l`.
    pub origin: String,
}

/// Parse crontab-format text. `has_user_field` is true for system crontabs
/// (`/etc/crontab`, `/etc/cron.d/*`), where a user column sits between the
/// schedule and the command. Comments, blank lines and `NAME=value` environment
/// assignments are skipped.
pub fn parse_crontab(
    text: &str,
    has_user_field: bool,
    source: CronSource,
    origin: &str,
) -> Vec<CronEntry> {
    text.lines()
        .filter_map(|line| parse_line(line, has_user_field))
        .map(|(schedule, user, command)| CronEntry {
            schedule,
            user,
            command,
            source,
            origin: origin.to_owned(),
        })
        .collect()
}

fn parse_line(line: &str, has_user_field: bool) -> Option<(String, Option<String>, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || is_env_assignment(line) {
        return None;
    }

    let tokens: Vec<&str> = line.split_whitespace().collect();
    let first = *tokens.first()?;

    // `@macro` schedules are a single token; otherwise the first five fields.
    let (schedule, mut idx) = if first.starts_with('@') {
        (first.to_owned(), 1)
    } else {
        if tokens.len() < 5 {
            return None;
        }
        (tokens[0..5].join(" "), 5)
    };

    let user = if has_user_field {
        let u = (*tokens.get(idx)?).to_owned();
        idx += 1;
        Some(u)
    } else {
        None
    };

    if idx >= tokens.len() {
        return None;
    }
    Some((schedule, user, tokens[idx..].join(" ")))
}

/// A `NAME=value` (or `NAME = value`) environment line, not a job.
fn is_env_assignment(line: &str) -> bool {
    match line.split_once('=') {
        Some((name, _)) => {
            let name = name.trim();
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// The run-parts periodic directories and the schedule macro they represent.
const PERIODIC_DIRS: &[(&str, &str)] = &[
    ("/etc/cron.hourly", "@hourly"),
    ("/etc/cron.daily", "@daily"),
    ("/etc/cron.weekly", "@weekly"),
    ("/etc/cron.monthly", "@monthly"),
];

/// Collect cron entries from every available source (best-effort). Missing files,
/// an absent `crontab` binary or unreadable directories simply contribute nothing.
pub async fn collect_cron_entries(transport: &dyn Transport) -> Vec<CronEntry> {
    let mut entries = Vec::new();

    if let Some(text) = run_stdout(transport, "crontab", &["-l"]).await {
        entries.extend(parse_crontab(&text, false, CronSource::User, "crontab -l"));
    }
    if let Some(text) = read_text(transport, "/etc/crontab").await {
        entries.extend(parse_crontab(
            &text,
            true,
            CronSource::System,
            "/etc/crontab",
        ));
    }
    if let Ok(dir) = transport.list_dir("/etc/cron.d").await {
        for entry in dir {
            if is_runnable_name(&entry.name) {
                let path = format!("/etc/cron.d/{}", entry.name);
                if let Some(text) = read_text(transport, &path).await {
                    entries.extend(parse_crontab(&text, true, CronSource::CronD, &path));
                }
            }
        }
    }
    for (dir, schedule) in PERIODIC_DIRS {
        if let Ok(list) = transport.list_dir(dir).await {
            for entry in list {
                if is_runnable_name(&entry.name) {
                    entries.push(CronEntry {
                        schedule: (*schedule).to_owned(),
                        user: Some("root".to_owned()),
                        command: format!("{dir}/{}", entry.name),
                        source: CronSource::Periodic,
                        origin: (*dir).to_owned(),
                    });
                }
            }
        }
    }

    entries
}

/// run-parts ignores names containing a dot and the conventional placeholders.
fn is_runnable_name(name: &str) -> bool {
    !name.contains('.') && name != "README" && !name.is_empty()
}

async fn read_text(transport: &dyn Transport, path: &str) -> Option<String> {
    transport
        .read_file(path)
        .await
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

async fn run_stdout(transport: &dyn Transport, program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    match transport.run(&spec).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::{DirEntry, FileType};
    use systui_transport::MockTransport;

    #[test]
    fn parses_system_crontab_with_user_field() {
        let entries = parse_crontab(
            include_str!("../fixtures/etc-crontab.txt"),
            true,
            CronSource::System,
            "/etc/crontab",
        );
        // env lines, the comment and the blank line are skipped.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].schedule, "17 * * * *");
        assert_eq!(entries[0].user.as_deref(), Some("root"));
        assert!(entries[0].command.starts_with("cd / && run-parts"));
        // @macro schedule keeps its user + command.
        let reboot = entries.iter().find(|e| e.schedule == "@reboot").unwrap();
        assert_eq!(reboot.user.as_deref(), Some("root"));
        assert_eq!(reboot.command, "/opt/startup.sh");
    }

    #[test]
    fn parses_user_crontab_without_user_field() {
        let entries = parse_crontab(
            include_str!("../fixtures/user-crontab.txt"),
            false,
            CronSource::User,
            "crontab -l",
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].schedule, "0 2 * * *");
        assert_eq!(entries[0].user, None);
        assert_eq!(
            entries[0].command,
            "/opt/backup.sh >> /var/log/backup.log 2>&1"
        );
        assert_eq!(entries[1].schedule, "*/5 * * * *");
        let daily = entries.iter().find(|e| e.schedule == "@daily").unwrap();
        assert_eq!(daily.command, "/opt/cleanup.sh");
    }

    #[test]
    fn env_assignments_are_not_jobs() {
        assert!(is_env_assignment("SHELL=/bin/sh"));
        assert!(is_env_assignment("PATH = /usr/bin"));
        assert!(!is_env_assignment("0 2 * * * /script --flag=value"));
        assert!(!is_env_assignment("@daily /opt/cleanup.sh"));
    }

    #[tokio::test]
    async fn collects_from_all_sources() {
        let transport = MockTransport::new()
            .with_stdout("crontab -l", include_str!("../fixtures/user-crontab.txt"))
            .with_file(
                "/etc/crontab",
                include_bytes!("../fixtures/etc-crontab.txt").to_vec(),
            )
            .with_dir(
                "/etc/cron.d",
                vec![
                    DirEntry {
                        name: "backup".to_owned(),
                        file_type: FileType::File,
                    },
                    DirEntry {
                        name: "placeholder.dpkg-dist".to_owned(),
                        file_type: FileType::File,
                    },
                ],
            )
            .with_file(
                "/etc/cron.d/backup",
                include_bytes!("../fixtures/cron.d-backup.txt").to_vec(),
            )
            .with_dir(
                "/etc/cron.daily",
                vec![
                    DirEntry {
                        name: "logrotate".to_owned(),
                        file_type: FileType::File,
                    },
                    DirEntry {
                        name: "README".to_owned(),
                        file_type: FileType::File,
                    },
                ],
            );

        let entries = collect_cron_entries(&transport).await;
        // 3 user + 3 system + 1 cron.d (dpkg-dist skipped) + 1 daily (README skipped).
        assert_eq!(entries.len(), 8);

        let crond = entries
            .iter()
            .find(|e| e.source == CronSource::CronD)
            .unwrap();
        assert_eq!(crond.user.as_deref(), Some("deploy"));
        assert_eq!(crond.origin, "/etc/cron.d/backup");

        let periodic = entries
            .iter()
            .find(|e| e.source == CronSource::Periodic)
            .unwrap();
        assert_eq!(periodic.schedule, "@daily");
        assert_eq!(periodic.command, "/etc/cron.daily/logrotate");
        assert_eq!(periodic.user.as_deref(), Some("root"));
    }

    #[tokio::test]
    async fn degrades_when_nothing_available() {
        assert!(collect_cron_entries(&MockTransport::new()).await.is_empty());
    }
}
