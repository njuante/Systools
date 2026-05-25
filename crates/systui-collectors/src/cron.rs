//! Cron collectors: gather scheduled jobs from every common source — the current
//! user's crontab (`crontab -l`), `/etc/crontab`, `/etc/cron.d/*` and the
//! `/etc/cron.{hourly,daily,weekly,monthly}` run-parts directories.
//!
//! Parsing is a pure, fixture-tested function. System crontabs (`/etc/crontab`,
//! `cron.d`) carry a user field; user crontabs do not. Schedule validation,
//! next-run preview (S4.6) and the cron risk checks (S4.7) build on these
//! entries; here we only collect and normalise them. Enumerating *other* users'
//! crontabs needs root, so we degrade to the current user plus system locations.

use chrono::{Datelike, NaiveDateTime, TimeDelta, Timelike};
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
    /// A job from `/etc/anacrontab` (period-based, runs after boot if missed).
    Anacron,
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
    /// Whether the entry is active. User-crontab entries commented out with `#`
    /// are surfaced as disabled so they can be re-enabled from the TUI.
    pub enabled: bool,
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
        .filter_map(|(schedule, user, command, enabled)| {
            (enabled || source == CronSource::User).then_some(CronEntry {
                schedule,
                user,
                command,
                source,
                origin: origin.to_owned(),
                enabled,
            })
        })
        .collect()
}

fn parse_line(line: &str, has_user_field: bool) -> Option<(String, Option<String>, String, bool)> {
    let line = line.trim();
    let (line, enabled) = match line.strip_prefix('#') {
        Some(disabled) => (disabled.trim_start(), false),
        None => (line, true),
    };
    if line.is_empty() || is_env_assignment(line) {
        return None;
    }

    let tokens: Vec<&str> = line.split_whitespace().collect();
    let first = *tokens.first()?;
    if !enabled && !looks_like_cron_schedule(&tokens) {
        return None;
    }

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
    Some((schedule, user, tokens[idx..].join(" "), enabled))
}

fn looks_like_cron_schedule(tokens: &[&str]) -> bool {
    let Some(first) = tokens.first() else {
        return false;
    };
    if first.starts_with('@') {
        return tokens.len() >= 2;
    }
    tokens.len() >= 6
        && tokens[0]
            .chars()
            .any(|c| c.is_ascii_digit() || matches!(c, '*' | '/' | ','))
        && tokens
            .iter()
            .take(5)
            .all(|token| token.chars().all(is_cron_field_char))
}

fn is_cron_field_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '*' | '/' | ',' | '-' | '?' | 'L' | 'W' | '#')
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
    if let Some(text) = read_text(transport, "/etc/anacrontab").await {
        entries.extend(parse_anacrontab(&text));
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
                        enabled: true,
                    });
                }
            }
        }
    }

    entries
}

/// Parse `/etc/anacrontab`. Each job line is `period delay job-id command…`.
/// Environment assignments (`NAME=value`) and comments are skipped. The period
/// (days, or an `@macro`) is mapped to the nearest cron `@macro` so the schedule
/// reads sensibly; the command keeps its job-id stripped off.
pub fn parse_anacrontab(text: &str) -> Vec<CronEntry> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || is_env_assignment(line) {
                return None;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            // period, delay, job-id, then the command.
            if tokens.len() < 4 {
                return None;
            }
            let command = tokens[3..].join(" ");
            Some(CronEntry {
                schedule: anacron_period_to_macro(tokens[0]),
                user: Some("root".to_owned()),
                command,
                source: CronSource::Anacron,
                origin: "/etc/anacrontab".to_owned(),
                enabled: true,
            })
        })
        .collect()
}

/// Map an anacron period to a cron `@macro` where it corresponds cleanly
/// (`1`→`@daily`, `7`→`@weekly`, `30`/`31`→`@monthly`, `@macro` passed through);
/// otherwise keep the raw period so nothing is misrepresented.
fn anacron_period_to_macro(period: &str) -> String {
    match period {
        "1" => "@daily".to_owned(),
        "7" => "@weekly".to_owned(),
        "30" | "31" => "@monthly".to_owned(),
        other => other.to_owned(),
    }
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

// --- cron schedule: validation, description and next-run -------------------
//
// We evaluate cron expressions ourselves rather than pulling an external crate:
// it keeps offline builds simple, targets the standard 5-field Vixie format
// (many crates assume a leading seconds field), and lets the caller inject the
// reference time so behaviour is deterministic and timezone-explicit. Times are
// naive (host-local); the caller decides the zone.

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A parsed, validated cron schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronSchedule {
    /// `@reboot`: runs once at startup; it has no calendar next-run.
    Reboot,
    /// A calendar schedule (minute/hour/day-of-month/month/day-of-week).
    Calendar {
        minutes: Vec<u32>,
        hours: Vec<u32>,
        doms: Vec<u32>,
        months: Vec<u32>,
        dows: Vec<u32>,
        /// Whether the day-of-month field was `*` (controls DOM/DOW combining).
        dom_star: bool,
        dow_star: bool,
    },
}

/// Parse and validate a cron schedule (`m h dom mon dow` or an `@macro`).
pub fn parse_schedule(raw: &str) -> Result<CronSchedule, String> {
    let raw = raw.trim();
    if raw == "@reboot" {
        return Ok(CronSchedule::Reboot);
    }
    let expanded = match raw {
        "@yearly" | "@annually" => "0 0 1 1 *",
        "@monthly" => "0 0 1 * *",
        "@weekly" => "0 0 * * 0",
        "@daily" | "@midnight" => "0 0 * * *",
        "@hourly" => "0 * * * *",
        other if other.starts_with('@') => return Err(format!("unknown macro `{other}`")),
        other => other,
    };

    let fields: Vec<&str> = expanded.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!("expected 5 fields, got {}", fields.len()));
    }
    let (minutes, _) = parse_field(fields[0], 0, 59)?;
    let (hours, _) = parse_field(fields[1], 0, 23)?;
    let (doms, dom_star) = parse_field(fields[2], 1, 31)?;
    let (months, _) = parse_field(fields[3], 1, 12)?;
    let (dows_raw, dow_star) = parse_field(fields[4], 0, 7)?;

    // Cron treats both 0 and 7 as Sunday; normalise to 0.
    let mut dows: Vec<u32> = dows_raw
        .into_iter()
        .map(|d| if d == 7 { 0 } else { d })
        .collect();
    dows.sort_unstable();
    dows.dedup();

    Ok(CronSchedule::Calendar {
        minutes,
        hours,
        doms,
        months,
        dows,
        dom_star,
        dow_star,
    })
}

/// Parse one cron field over `[min, max]`, returning the allowed values plus
/// whether the field was a bare `*` (needed for DOM/DOW combining semantics).
fn parse_field(spec: &str, min: u32, max: u32) -> Result<(Vec<u32>, bool), String> {
    use std::collections::BTreeSet;
    let mut values = BTreeSet::new();

    for part in spec.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>()
                    .map_err(|_| format!("invalid step in `{part}`"))?,
            ),
            None => (part, 1),
        };
        if step == 0 {
            return Err(format!("step cannot be zero in `{part}`"));
        }

        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse()
                    .map_err(|_| format!("invalid value in `{part}`"))?,
                b.parse()
                    .map_err(|_| format!("invalid value in `{part}`"))?,
            )
        } else {
            let v = range
                .parse()
                .map_err(|_| format!("invalid value in `{part}`"))?;
            (v, v)
        };

        if lo < min || hi > max || lo > hi {
            return Err(format!("`{part}` out of range {min}-{max}"));
        }
        let mut v = lo;
        while v <= hi {
            values.insert(v);
            v += step;
        }
    }

    if values.is_empty() {
        return Err(format!("`{spec}` matches no values"));
    }
    Ok((values.into_iter().collect(), spec == "*"))
}

impl CronSchedule {
    /// The next time strictly after `after` that this schedule fires, searching
    /// up to ~366 days ahead. `@reboot` and impossible schedules return `None`.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let CronSchedule::Calendar { .. } = self else {
            return None;
        };
        let mut t = after
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .map(|t| t + TimeDelta::minutes(1))?;
        let limit = after + TimeDelta::days(366);
        while t <= limit {
            if self.matches(t) {
                return Some(t);
            }
            t += TimeDelta::minutes(1);
        }
        None
    }

    /// The next `count` fire times after `after`.
    pub fn upcoming(&self, after: NaiveDateTime, count: usize) -> Vec<NaiveDateTime> {
        let mut out = Vec::new();
        let mut cursor = after;
        for _ in 0..count {
            match self.next_after(cursor) {
                Some(t) => {
                    out.push(t);
                    cursor = t;
                }
                None => break,
            }
        }
        out
    }

    fn matches(&self, dt: NaiveDateTime) -> bool {
        let CronSchedule::Calendar {
            minutes,
            hours,
            months,
            ..
        } = self
        else {
            return false;
        };
        minutes.contains(&dt.minute())
            && hours.contains(&dt.hour())
            && months.contains(&dt.month())
            && self.day_matches(dt)
    }

    fn day_matches(&self, dt: NaiveDateTime) -> bool {
        let CronSchedule::Calendar {
            doms,
            dows,
            dom_star,
            dow_star,
            ..
        } = self
        else {
            return false;
        };
        let dom_ok = doms.contains(&dt.day());
        let dow_ok = dows.contains(&dt.weekday().num_days_from_sunday());
        match (dom_star, dow_star) {
            (true, true) => true,
            (false, true) => dom_ok,
            (true, false) => dow_ok,
            // Both restricted: classic cron ORs them.
            (false, false) => dom_ok || dow_ok,
        }
    }

    /// A human-readable description of the schedule.
    pub fn describe(&self) -> String {
        let (minutes, hours, doms, months, dows, dom_star, dow_star) = match self {
            CronSchedule::Reboot => return "At system startup".to_owned(),
            CronSchedule::Calendar {
                minutes,
                hours,
                doms,
                months,
                dows,
                dom_star,
                dow_star,
            } => (minutes, hours, doms, months, dows, *dom_star, *dow_star),
        };

        let every_minute = is_full(minutes, 0, 59);
        let every_hour = is_full(hours, 0, 23);
        let all_days = dom_star && dow_star && is_full(months, 1, 12);

        if every_minute && every_hour && all_days {
            return "Every minute".to_owned();
        }
        if let Some(step) = step_from_zero(minutes, 59)
            && every_hour
            && all_days
        {
            return format!("Every {step} minutes");
        }
        if minutes.len() == 1 && hours.len() == 1 && is_full(months, 1, 12) {
            let at = format!("at {:02}:{:02}", hours[0], minutes[0]);
            if dom_star && dow_star {
                return format!("Every day {at}");
            }
            if dom_star && !dow_star {
                return format!("{} {at}", weekday_phrase(dows));
            }
        }

        format!(
            "At minute {}, hour {}, day-of-month {}, month {}, day-of-week {}",
            field_text(minutes, 0, 59),
            field_text(hours, 0, 23),
            if dom_star { "*".to_owned() } else { join(doms) },
            field_text(months, 1, 12),
            if dow_star {
                "*".to_owned()
            } else {
                weekday_list(dows)
            },
        )
    }
}

fn is_full(values: &[u32], min: u32, max: u32) -> bool {
    values.len() as u32 == max - min + 1
}

/// If `values` is `0, step, 2*step, …` covering the range, return `step`.
fn step_from_zero(values: &[u32], max: u32) -> Option<u32> {
    if values.len() < 2 || values[0] != 0 {
        return None;
    }
    let step = values[1];
    if step == 0 {
        return None;
    }
    let expected: Vec<u32> = (0..=max).step_by(step as usize).collect();
    (values == expected.as_slice()).then_some(step)
}

fn field_text(values: &[u32], min: u32, max: u32) -> String {
    if is_full(values, min, max) {
        "*".to_owned()
    } else {
        join(values)
    }
}

fn join(values: &[u32]) -> String {
    values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn weekday_list(dows: &[u32]) -> String {
    dows.iter()
        .map(|&d| WEEKDAYS[(d % 7) as usize])
        .collect::<Vec<_>>()
        .join(", ")
}

fn weekday_phrase(dows: &[u32]) -> String {
    format!("on {}", weekday_list(dows))
}

// --- systemd timers --------------------------------------------------------

/// A systemd timer unit and what it activates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdTimer {
    pub unit: String,
    pub activates: String,
    /// The next elapse time as reported by systemd, or `-` when not scheduled.
    pub next: String,
}

/// List systemd timers via `systemctl list-timers`.
pub async fn collect_timers(transport: &dyn Transport) -> Vec<SystemdTimer> {
    match run_stdout(
        transport,
        "systemctl",
        &["list-timers", "--all", "--no-legend", "--no-pager"],
    )
    .await
    {
        Some(out) => out.lines().filter_map(parse_timer_line).collect(),
        None => Vec::new(),
    }
}

/// Parse one `systemctl list-timers` row. The NEXT/LAST date columns contain
/// spaces, so we anchor on the `*.timer` token: it is the unit, the token after
/// it is what it activates, and the leading date (if any) is the next elapse.
fn parse_timer_line(line: &str) -> Option<SystemdTimer> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let unit_idx = tokens.iter().position(|t| t.ends_with(".timer"))?;
    let head = &tokens[..unit_idx];
    let next = if head.first().is_none_or(|t| *t == "-") {
        "-".to_owned()
    } else {
        head.iter().take(4).copied().collect::<Vec<_>>().join(" ")
    };
    Some(SystemdTimer {
        unit: tokens[unit_idx].to_owned(),
        activates: tokens.get(unit_idx + 1).copied().unwrap_or("").to_owned(),
        next,
    })
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;
    use systui_testkit::fuzz::messy_output;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn cron_parsers_never_panic(s in messy_output()) {
            let _ = parse_crontab(&s, true, CronSource::User, "fuzz");
            let _ = parse_crontab(&s, false, CronSource::System, "fuzz");
            let _ = parse_anacrontab(&s);
            let _ = parse_schedule(&s);
            for line in s.lines() {
                let _ = parse_timer_line(line);
                let _ = parse_field(line, 0, 59);
            }
        }
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
    fn parses_disabled_user_crontab_entries() {
        let text = "# regular operator note with enough words\n#*/5 * * * * /opt/poll.sh\n";
        let entries = parse_crontab(text, false, CronSource::User, "crontab -l");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].schedule, "*/5 * * * *");
        assert_eq!(entries[0].command, "/opt/poll.sh");
        assert!(!entries[0].enabled);
    }

    #[test]
    fn parses_anacrontab_jobs() {
        let entries = parse_anacrontab(include_str!("../fixtures/anacrontab.txt"));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].schedule, "@daily"); // period 1
        assert_eq!(entries[0].command, "run-parts --report /etc/cron.daily");
        assert_eq!(entries[0].source, CronSource::Anacron);
        assert_eq!(entries[1].schedule, "@weekly"); // period 7
        assert_eq!(entries[2].schedule, "@monthly"); // @monthly passthrough
        // The job-id column is stripped; SHELL=/PATH= assignments are skipped.
        assert!(entries.iter().all(|e| !e.command.contains("cron.daily\t")));
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

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn validates_good_and_bad_expressions() {
        assert!(parse_schedule("0 2 * * *").is_ok());
        assert!(parse_schedule("*/15 * * * *").is_ok());
        assert!(parse_schedule("0 9 * * 1-5").is_ok());
        assert!(parse_schedule("@daily").is_ok());
        assert_eq!(parse_schedule("@reboot").unwrap(), CronSchedule::Reboot);

        assert!(parse_schedule("60 * * * *").is_err()); // minute out of range
        assert!(parse_schedule("* * * *").is_err()); // too few fields
        assert!(parse_schedule("* * * * 1/0").is_err()); // zero step
        assert!(parse_schedule("@bogus").is_err());
    }

    #[test]
    fn macros_expand_to_calendar_schedules() {
        // @daily == 0 0 * * *  → next is the upcoming midnight.
        let daily = parse_schedule("@daily").unwrap();
        let next = daily.next_after(dt(2026, 5, 24, 10, 0)).unwrap();
        assert_eq!(next, dt(2026, 5, 25, 0, 0));
    }

    #[test]
    fn next_after_handles_time_of_day() {
        let sched = parse_schedule("30 2 * * *").unwrap();
        assert_eq!(
            sched.next_after(dt(2026, 5, 24, 10, 0)).unwrap(),
            dt(2026, 5, 25, 2, 30)
        );
        // Earlier the same day, it fires today.
        assert_eq!(
            sched.next_after(dt(2026, 5, 24, 1, 0)).unwrap(),
            dt(2026, 5, 24, 2, 30)
        );
    }

    #[test]
    fn next_after_handles_step_and_weekday() {
        let every_15 = parse_schedule("*/15 * * * *").unwrap();
        assert_eq!(
            every_15.next_after(dt(2026, 5, 24, 10, 7)).unwrap(),
            dt(2026, 5, 24, 10, 15)
        );
        // 2026-05-24 is a Sunday; next Monday 09:00.
        let monday = parse_schedule("0 9 * * 1").unwrap();
        assert_eq!(
            monday.next_after(dt(2026, 5, 24, 12, 0)).unwrap(),
            dt(2026, 5, 25, 9, 0)
        );
    }

    #[test]
    fn upcoming_returns_successive_runs() {
        let daily = parse_schedule("0 0 * * *").unwrap();
        let runs = daily.upcoming(dt(2026, 5, 24, 10, 0), 3);
        assert_eq!(
            runs,
            vec![
                dt(2026, 5, 25, 0, 0),
                dt(2026, 5, 26, 0, 0),
                dt(2026, 5, 27, 0, 0)
            ]
        );
        // @reboot has no calendar next-run.
        assert!(
            CronSchedule::Reboot
                .next_after(dt(2026, 5, 24, 10, 0))
                .is_none()
        );
    }

    #[test]
    fn describes_common_schedules() {
        assert_eq!(
            parse_schedule("@reboot").unwrap().describe(),
            "At system startup"
        );
        assert_eq!(
            parse_schedule("* * * * *").unwrap().describe(),
            "Every minute"
        );
        assert_eq!(
            parse_schedule("*/15 * * * *").unwrap().describe(),
            "Every 15 minutes"
        );
        assert_eq!(
            parse_schedule("0 2 * * *").unwrap().describe(),
            "Every day at 02:00"
        );
        assert_eq!(
            parse_schedule("30 9 * * 1").unwrap().describe(),
            "on Mon at 09:30"
        );
    }

    #[tokio::test]
    async fn parses_systemd_timers() {
        let transport = MockTransport::new().with_stdout(
            "systemctl list-timers --all --no-legend --no-pager",
            include_str!("../fixtures/systemctl-timers.txt"),
        );
        let timers = collect_timers(&transport).await;
        assert_eq!(timers.len(), 3);
        assert_eq!(timers[0].unit, "logrotate.timer");
        assert_eq!(timers[0].activates, "logrotate.service");
        assert_eq!(timers[0].next, "Wed 2026-05-27 00:00:00 UTC");
        // A timer with no scheduled next elapse shows "-".
        let fstrim = timers.iter().find(|t| t.unit == "fstrim.timer").unwrap();
        assert_eq!(fstrim.next, "-");
        assert_eq!(fstrim.activates, "fstrim.service");
    }
}
