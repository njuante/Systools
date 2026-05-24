//! Cron risk checks: turn collected [`CronEntry`]s into evidence-based
//! `Finding`s — jobs whose script is missing or not executable, jobs that
//! discard their output (no logging), duplicate entries, risky root jobs (a root
//! cron running a world-writable or temp-located script) and suspiciously
//! high-frequency jobs. The schedule is also validated.
//!
//! Detection only — v0.4 never edits crontabs. The existence/permission checks
//! need each script's `stat`; everything else is pure over the parsed entries.
//! [`cron_findings`] reads the stats once through a [`Transport`] and hands them
//! to the pure, fixture-tested [`check_crons`].

use std::collections::HashMap;

use systui_collectors::{CronEntry, CronSchedule, CronSource, parse_schedule};
use systui_core::{CommandSpec, Finding, ModuleId, Severity, Transport};

use crate::system::{StatInfo, parse_stat};

/// World-writable locations a root cron should not be running scripts out of.
const TEMP_DIRS: &[&str] = &["/tmp/", "/var/tmp/", "/dev/shm/"];

/// Below this gap (a job firing this many times per hour or more) a schedule
/// looks suspiciously frequent. 12/hour == every 5 minutes.
const HIGH_FREQUENCY_PER_HOUR: usize = 12;

/// All cron findings for the collected entries, worst-first. The `stat` for each
/// referenced script is read once through the transport.
pub async fn cron_findings(transport: &dyn Transport, entries: &[CronEntry]) -> Vec<Finding> {
    let paths = script_paths(entries);
    let stats = stat_scripts(transport, &paths).await;
    let mut findings = check_crons(entries, &stats);
    findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
    findings
}

/// The script an entry runs, when its command begins with an absolute path.
/// Commands that start with a shell builtin or interpreter (`cd ...`,
/// `run-parts ...`, `[ -x ... ]`) have no directly checkable script → `None`.
pub fn script_path(command: &str) -> Option<&str> {
    let first = command.split_whitespace().next()?;
    first.starts_with('/').then_some(first)
}

/// The distinct absolute script paths referenced by these entries, sorted so the
/// resulting `stat` command is deterministic.
fn script_paths(entries: &[CronEntry]) -> Vec<String> {
    let mut paths: Vec<String> = entries
        .iter()
        .filter_map(|e| script_path(&e.command).map(str::to_owned))
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// `stat` the given paths in one call, returning a path→[`StatInfo`] map. Paths
/// that do not exist simply do not appear (the check treats that as "missing").
async fn stat_scripts(transport: &dyn Transport, paths: &[String]) -> HashMap<String, StatInfo> {
    if paths.is_empty() {
        return HashMap::new();
    }
    let mut args: Vec<&str> = vec!["-c", "%a %U %G %n"];
    args.extend(paths.iter().map(String::as_str));
    let spec = CommandSpec::new("stat").args(args.iter().copied());
    // `stat` prints the files it finds even when others are missing, so use the
    // stdout regardless of the (non-zero) exit code.
    let out = match transport.run(&spec).await {
        Ok(output) => output.stdout,
        Err(_) => return HashMap::new(),
    };
    parse_stat(&out)
        .into_iter()
        .map(|s| (s.path.clone(), s))
        .collect()
}

/// Pure cron checks over the parsed entries and the scripts' `stat` info.
pub fn check_crons(entries: &[CronEntry], scripts: &HashMap<String, StatInfo>) -> Vec<Finding> {
    let mut findings: Vec<Finding> = entries
        .iter()
        .flat_map(|entry| check_entry(entry, scripts))
        .collect();
    findings.extend(duplicate_findings(entries));
    findings
}

/// Risk findings for a single cron entry (duplicates are handled across the set).
fn check_entry(entry: &CronEntry, scripts: &HashMap<String, StatInfo>) -> Vec<Finding> {
    let mut out = Vec::new();
    let is_root = entry.user.as_deref() == Some("root");

    match parse_schedule(&entry.schedule) {
        Ok(schedule) => {
            if let Some(finding) = high_frequency(entry, &schedule) {
                out.push(finding);
            }
        }
        Err(reason) => out.push(invalid_schedule(entry, &reason)),
    }

    if let Some(path) = script_path(&entry.command) {
        match scripts.get(path) {
            None => out.push(missing_script(entry, path)),
            Some(stat) => {
                if stat.mode & 0o111 == 0 {
                    out.push(not_executable(entry, path, stat));
                }
                if is_root && stat.mode & 0o002 != 0 {
                    out.push(world_writable_script(entry, path, stat));
                }
            }
        }
        if is_root && TEMP_DIRS.iter().any(|dir| path.starts_with(dir)) {
            out.push(root_temp_script(entry, path));
        }
    }

    if needs_logging(entry) && !has_logging(&entry.command) {
        out.push(no_logging(entry));
    }

    out
}

fn missing_script(entry: &CronEntry, path: &str) -> Finding {
    Finding::new(
        format!("cron.missing-script.{path}"),
        Severity::Medium,
        ModuleId::Crons,
        "Cron job runs a script that does not exist",
    )
    .with_evidence(format!("{}: {path} not found", entry.origin))
    .impact("The job fails every time it fires, often silently.")
    .recommendation("Fix the path or remove the stale cron entry.")
}

fn not_executable(entry: &CronEntry, path: &str, stat: &StatInfo) -> Finding {
    Finding::new(
        format!("cron.not-executable.{path}"),
        Severity::Medium,
        ModuleId::Crons,
        "Cron job script is not executable",
    )
    .with_evidence(format!("{}: {path} is mode {:o}", entry.origin, stat.mode))
    .impact("Cron cannot run the script, so the job never succeeds.")
    .recommendation(format!("Make it executable, e.g. `chmod +x {path}`."))
}

fn world_writable_script(entry: &CronEntry, path: &str, stat: &StatInfo) -> Finding {
    Finding::new(
        format!("cron.world-writable-script.{path}"),
        Severity::High,
        ModuleId::Crons,
        "Root cron runs a world-writable script",
    )
    .with_evidence(format!(
        "{}: {path} is mode {:o} and runs as root",
        entry.origin, stat.mode
    ))
    .impact("Any local user can edit the script and have it run as root.")
    .recommendation(format!("Remove world-write, e.g. `chmod o-w {path}`."))
}

fn root_temp_script(entry: &CronEntry, path: &str) -> Finding {
    Finding::new(
        format!("cron.root-temp-script.{path}"),
        Severity::High,
        ModuleId::Crons,
        "Root cron runs a script from a world-writable location",
    )
    .with_evidence(format!("{}: root runs {path}", entry.origin))
    .impact("Scripts under /tmp can be replaced by any user before the job runs.")
    .recommendation("Move the script to a root-owned directory such as /usr/local/sbin.")
}

fn no_logging(entry: &CronEntry) -> Finding {
    Finding::new(
        format!("cron.no-logging.{}", entry.command),
        Severity::Info,
        ModuleId::Crons,
        "Cron job output is not captured",
    )
    .with_evidence(format!("{}: {}", entry.origin, entry.command))
    .impact("With no redirection, failures rely on local cron mail and are easily missed.")
    .recommendation("Redirect output to a log file, e.g. `>> /var/log/job.log 2>&1`.")
}

fn invalid_schedule(entry: &CronEntry, reason: &str) -> Finding {
    Finding::new(
        format!("cron.invalid-schedule.{}", entry.command),
        Severity::Low,
        ModuleId::Crons,
        "Cron job has an invalid schedule",
    )
    .with_evidence(format!("{}: `{}` — {reason}", entry.origin, entry.schedule))
    .impact("Cron may skip the job or interpret it differently than intended.")
    .recommendation("Correct the schedule expression.")
}

fn high_frequency(entry: &CronEntry, schedule: &CronSchedule) -> Option<Finding> {
    let CronSchedule::Calendar { minutes, hours, .. } = schedule else {
        return None;
    };
    if hours.len() != 24 || minutes.len() < HIGH_FREQUENCY_PER_HOUR {
        return None;
    }
    Some(
        Finding::new(
            format!("cron.high-frequency.{}", entry.command),
            Severity::Low,
            ModuleId::Crons,
            "Cron job runs very frequently",
        )
        .with_evidence(format!(
            "{}: `{}` fires {} times per hour",
            entry.origin,
            entry.schedule,
            minutes.len()
        ))
        .impact("A very frequent job adds steady load and can pile up if it overruns.")
        .recommendation("Confirm the cadence is intended; consider a lock or a systemd timer."),
    )
}

/// One finding per group of identical `(schedule, command)` entries seen more
/// than once across all sources.
fn duplicate_findings(entries: &[CronEntry]) -> Vec<Finding> {
    let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
    for entry in entries {
        *counts
            .entry((entry.schedule.as_str(), entry.command.as_str()))
            .or_default() += 1;
    }
    let mut out: Vec<Finding> = counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|((schedule, command), n)| {
            Finding::new(
                format!("cron.duplicate.{command}"),
                Severity::Low,
                ModuleId::Crons,
                "Duplicate cron job",
            )
            .with_evidence(format!("`{schedule} {command}` appears {n} times"))
            .impact("The same work runs several times, wasting resources or colliding.")
            .recommendation("Remove the redundant entries.")
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Periodic (run-parts) jobs log through cron/syslog already, so the no-logging
/// check only applies to crontab-style entries.
fn needs_logging(entry: &CronEntry) -> bool {
    entry.source != CronSource::Periodic
}

/// Whether the command redirects its output somewhere durable.
fn has_logging(command: &str) -> bool {
    command.contains('>') || command.contains("logger")
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_collectors::CronSource;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    fn entry(schedule: &str, user: Option<&str>, command: &str, source: CronSource) -> CronEntry {
        CronEntry {
            schedule: schedule.to_owned(),
            user: user.map(str::to_owned),
            command: command.to_owned(),
            source,
            origin: "/etc/crontab".to_owned(),
        }
    }

    fn stat(path: &str, mode: u32) -> StatInfo {
        StatInfo {
            mode,
            owner: "root".to_owned(),
            group: "root".to_owned(),
            path: path.to_owned(),
        }
    }

    fn ids(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn extracts_only_absolute_script_paths() {
        assert_eq!(
            script_path("/opt/backup.sh >> /var/log/backup.log 2>&1"),
            Some("/opt/backup.sh")
        );
        assert_eq!(
            script_path("/etc/cron.daily/logrotate"),
            Some("/etc/cron.daily/logrotate")
        );
        assert_eq!(script_path("cd / && run-parts /etc/cron.hourly"), None);
        assert_eq!(script_path("[ -x /x ] && /x"), None);
    }

    #[test]
    fn flags_missing_and_non_executable_scripts() {
        let entries = vec![
            entry(
                "0 2 * * *",
                None,
                "/opt/missing.sh >> /var/log/m.log",
                CronSource::User,
            ),
            entry(
                "0 3 * * *",
                None,
                "/opt/present.sh >> /var/log/p.log",
                CronSource::User,
            ),
        ];
        let mut scripts = HashMap::new();
        scripts.insert("/opt/present.sh".to_owned(), stat("/opt/present.sh", 0o644));

        let findings = check_crons(&entries, &scripts);
        assert!(ids(&findings).contains(&"cron.missing-script./opt/missing.sh"));
        assert!(ids(&findings).contains(&"cron.not-executable./opt/present.sh"));
    }

    #[test]
    fn flags_world_writable_root_script_as_high() {
        let entries = vec![entry(
            "0 1 * * *",
            Some("root"),
            "/opt/job.sh >> /var/log/job.log",
            CronSource::System,
        )];
        let mut scripts = HashMap::new();
        scripts.insert("/opt/job.sh".to_owned(), stat("/opt/job.sh", 0o777));

        let findings = check_crons(&entries, &scripts);
        let f = findings
            .iter()
            .find(|f| f.id == "cron.world-writable-script./opt/job.sh")
            .unwrap();
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn flags_root_script_in_temp_dir() {
        let entries = vec![entry(
            "@daily",
            Some("root"),
            "/tmp/run.sh >> /var/log/run.log",
            CronSource::CronD,
        )];
        let mut scripts = HashMap::new();
        scripts.insert("/tmp/run.sh".to_owned(), stat("/tmp/run.sh", 0o755));

        let findings = check_crons(&entries, &scripts);
        assert!(ids(&findings).contains(&"cron.root-temp-script./tmp/run.sh"));
    }

    #[test]
    fn flags_no_logging_but_not_for_periodic() {
        let entries = vec![
            entry("0 4 * * *", None, "/opt/quiet.sh", CronSource::User),
            entry(
                "@daily",
                Some("root"),
                "/etc/cron.daily/logrotate",
                CronSource::Periodic,
            ),
        ];
        let scripts = HashMap::new();
        let findings = check_crons(&entries, &scripts);
        assert!(ids(&findings).contains(&"cron.no-logging./opt/quiet.sh"));
        // The periodic job is not flagged for logging.
        assert!(!ids(&findings).contains(&"cron.no-logging./etc/cron.daily/logrotate"));
    }

    #[test]
    fn flags_invalid_schedule_and_high_frequency() {
        let entries = vec![
            entry("99 * * * *", None, "/opt/a.sh > /l", CronSource::User),
            entry("* * * * *", None, "/opt/b.sh > /l", CronSource::User),
        ];
        let scripts = HashMap::new();
        let findings = check_crons(&entries, &scripts);
        assert!(ids(&findings).contains(&"cron.invalid-schedule./opt/a.sh > /l"));
        assert!(ids(&findings).contains(&"cron.high-frequency./opt/b.sh > /l"));
    }

    #[test]
    fn flags_duplicates_once() {
        let entries = vec![
            entry("0 2 * * *", None, "/opt/dup.sh > /l", CronSource::User),
            entry("0 2 * * *", None, "/opt/dup.sh > /l", CronSource::CronD),
        ];
        let scripts = HashMap::new();
        let findings = check_crons(&entries, &scripts);
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.id == "cron.duplicate./opt/dup.sh > /l")
                .count(),
            1
        );
    }

    #[test]
    fn clean_jobs_yield_no_findings() {
        let entries = vec![entry(
            "0 5 * * *",
            Some("root"),
            "/usr/local/sbin/ok.sh >> /var/log/ok.log 2>&1",
            CronSource::System,
        )];
        let mut scripts = HashMap::new();
        scripts.insert(
            "/usr/local/sbin/ok.sh".to_owned(),
            stat("/usr/local/sbin/ok.sh", 0o755),
        );
        assert!(check_crons(&entries, &scripts).is_empty());
    }

    #[tokio::test]
    async fn cron_findings_stats_scripts_and_sorts_worst_first() {
        let entries = vec![entry(
            "0 1 * * *",
            Some("root"),
            "/opt/job.sh",
            CronSource::System,
        )];
        // /opt/job.sh is world-writable (High) and has no logging (Info).
        let transport = MockTransport::new().with_command(
            "stat -c %a %U %G %n /opt/job.sh",
            CommandOutput {
                exit_code: Some(0),
                stdout: "777 root root /opt/job.sh\n".to_owned(),
                stderr: String::new(),
                duration: std::time::Duration::ZERO,
            },
        );
        let findings = cron_findings(&transport, &entries).await;
        assert_eq!(findings.first().unwrap().severity, Severity::High);
        assert!(ids(&findings).contains(&"cron.world-writable-script./opt/job.sh"));
        assert!(ids(&findings).contains(&"cron.no-logging./opt/job.sh"));
    }
}
