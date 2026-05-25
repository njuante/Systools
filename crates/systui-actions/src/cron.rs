//! User-crontab actions: add / edit / delete / enable / disable a job in the
//! connected user's crontab (`Product.md` §4.8).
//!
//! Every operation reads the current crontab (`crontab -l`), produces the new
//! crontab as text (pure, tested transforms), **backs up** the prior crontab, then
//! installs the new one via `crontab -` with the content piped through
//! [`CommandSpec::stdin`] — so user input is never shell-interpolated. System cron
//! (`/etc/crontab`, `/etc/cron.d`) and systemd timers are out of scope (read-only).
//! Driven by the action engine like every other mutation.

use async_trait::async_trait;
use systui_collectors::{CronSource, parse_crontab, parse_schedule};
use systui_core::{
    Action, ActionOutcome, ActionPreview, CommandSpec, CoreError, ModuleId, Result, RiskLevel,
    Transport,
};

/// Where the prior crontab is copied before a write, so a bad edit is recoverable.
const CRON_BACKUP_PATH: &str = "/tmp/systui-crontab.bak";

/// An operation on the user crontab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronOp {
    /// Append a new job.
    Add,
    /// Replace a job's schedule/command.
    Edit {
        new_schedule: String,
        new_command: String,
    },
    /// Remove a job.
    Delete,
    /// Uncomment a previously disabled job.
    Enable,
    /// Comment out a job (keeps it in the crontab, stops it running).
    Disable,
    /// Run the job's command immediately, outside its schedule. Does not modify
    /// the crontab.
    RunNow,
}

/// An action on a single user-crontab entry, identified by its schedule + command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronAction {
    pub op: CronOp,
    /// The target entry's schedule (for `Add`, the schedule of the new job).
    pub schedule: String,
    /// The target entry's command (for `Add`, the command of the new job).
    pub command: String,
}

impl CronAction {
    pub fn add(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            op: CronOp::Add,
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    pub fn delete(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            op: CronOp::Delete,
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    pub fn enable(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            op: CronOp::Enable,
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    pub fn disable(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            op: CronOp::Disable,
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    pub fn run_now(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            op: CronOp::RunNow,
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    /// The command to run immediately for [`CronOp::RunNow`]. The job's command
    /// is a shell line (it may contain redirections/pipes), so it is run through
    /// `sh -c` exactly as cron would — never re-parsed or interpolated by us.
    fn run_now_command(&self) -> CommandSpec {
        CommandSpec::new("sh").args(["-c", &self.command])
    }

    pub fn edit(
        schedule: impl Into<String>,
        command: impl Into<String>,
        new_schedule: impl Into<String>,
        new_command: impl Into<String>,
    ) -> Self {
        Self {
            op: CronOp::Edit {
                new_schedule: new_schedule.into(),
                new_command: new_command.into(),
            },
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    fn verb(&self) -> &'static str {
        match self.op {
            CronOp::Add => "Add",
            CronOp::Edit { .. } => "Edit",
            CronOp::Delete => "Delete",
            CronOp::Enable => "Enable",
            CronOp::Disable => "Disable",
            CronOp::RunNow => "Run",
        }
    }

    fn summary(&self) -> String {
        format!("{} cron job", self.verb())
    }

    /// The schedule whose validity matters for this op (the new one when editing).
    fn schedule_to_validate(&self) -> Option<&str> {
        match &self.op {
            CronOp::Add => Some(&self.schedule),
            CronOp::Edit { new_schedule, .. } => Some(new_schedule),
            CronOp::Delete | CronOp::Enable | CronOp::Disable | CronOp::RunNow => None,
        }
    }

    /// Compute the new crontab text from the current one for this op.
    fn apply(&self, current: &str) -> std::result::Result<String, String> {
        match &self.op {
            CronOp::Add => Ok(append_job(current, &self.schedule, &self.command)),
            CronOp::Delete => remove_job(current, &self.schedule, &self.command),
            CronOp::Enable => set_enabled(current, &self.schedule, &self.command, true),
            CronOp::Disable => set_enabled(current, &self.schedule, &self.command, false),
            CronOp::Edit {
                new_schedule,
                new_command,
            } => replace_job(
                current,
                &self.schedule,
                &self.command,
                new_schedule,
                new_command,
            ),
            // RunNow never rewrites the crontab; execute() handles it directly.
            CronOp::RunNow => Err("run-now does not modify the crontab".to_owned()),
        }
    }

    /// Read the current user crontab; an absent crontab reads as empty.
    async fn read_crontab(&self, transport: &dyn Transport) -> String {
        let spec = CommandSpec::new("crontab").arg("-l");
        match transport.run(&spec).await {
            Ok(output) if output.success() => output.stdout,
            _ => String::new(),
        }
    }
}

#[async_trait]
impl Action for CronAction {
    fn module(&self) -> ModuleId {
        ModuleId::Crons
    }

    fn risk(&self) -> RiskLevel {
        match self.op {
            // Re-enabling restores a job the user already authored: low risk.
            CronOp::Enable => RiskLevel::Low,
            // Delete is destructive; running a job executes arbitrary side
            // effects immediately — both warrant a typed confirmation.
            CronOp::Delete | CronOp::RunNow => RiskLevel::High,
            _ => RiskLevel::Medium,
        }
    }

    fn requires_privilege(&self) -> bool {
        // The user crontab is owned by the connected user; no elevation needed.
        false
    }

    fn target(&self) -> String {
        self.command.clone()
    }

    async fn preview(&self, _transport: &dyn Transport) -> Result<ActionPreview> {
        if self.op == CronOp::RunNow {
            return Ok(ActionPreview {
                summary: "Run cron job now".to_owned(),
                details: vec![
                    format!("command: {}", self.command),
                    "Runs the command immediately via `sh -c`, outside its schedule.".to_owned(),
                    "The crontab is not modified.".to_owned(),
                ],
                command: Some(self.run_now_command()),
                reversible: false,
                creates_backup: false,
            });
        }
        // Validate the schedule up front so an invalid expression never reaches the
        // crontab.
        if let Some(schedule) = self.schedule_to_validate() {
            parse_schedule(schedule).map_err(|e| {
                CoreError::InvalidInput(format!("invalid schedule `{schedule}`: {e}"))
            })?;
        }

        let mut details = Vec::new();
        match &self.op {
            CronOp::Edit {
                new_schedule,
                new_command,
            } => {
                details.push(format!("from: {} {}", self.schedule, self.command));
                details.push(format!("to:   {new_schedule} {new_command}"));
            }
            _ => details.push(format!("{} {}", self.schedule, self.command)),
        }
        details.push("Targets the current user's crontab.".to_owned());
        if !matches!(self.op, CronOp::Add) {
            details.push(format!("Prior crontab is backed up to {CRON_BACKUP_PATH}."));
        }

        Ok(ActionPreview {
            summary: self.summary(),
            details,
            command: Some(CommandSpec::new("crontab").arg("-")),
            reversible: !matches!(self.op, CronOp::Delete),
            creates_backup: !matches!(self.op, CronOp::Add),
        })
    }

    async fn execute(&self, transport: &dyn Transport) -> Result<ActionOutcome> {
        if self.op == CronOp::RunNow {
            let output = transport.run(&self.run_now_command()).await?;
            let trimmed = output.stderr.trim();
            return Ok(ActionOutcome {
                success: output.success(),
                message: if output.success() {
                    format!("ran: {}", self.command)
                } else {
                    format!(
                        "run failed: {}",
                        if trimmed.is_empty() {
                            "non-zero exit"
                        } else {
                            trimmed
                        }
                    )
                },
            });
        }

        let current = self.read_crontab(transport).await;

        let new_text = match self.apply(&current) {
            Ok(text) => text,
            Err(reason) => {
                return Ok(ActionOutcome {
                    success: false,
                    message: format!("{}: {reason}", self.summary()),
                });
            }
        };

        // Back up the prior crontab before replacing it (skip when there is none).
        if !current.is_empty() {
            let backup = CommandSpec::new("tee")
                .arg(CRON_BACKUP_PATH)
                .stdin(current.clone());
            let out = transport.run(&backup).await?;
            if !out.success() {
                return Ok(ActionOutcome {
                    success: false,
                    message: format!("aborted: could not back up crontab ({})", out.stderr.trim()),
                });
            }
        }

        let install = CommandSpec::new("crontab").arg("-").stdin(new_text);
        let out = transport.run(&install).await?;
        if !out.success() {
            return Ok(ActionOutcome {
                success: false,
                message: format!("{} failed: {}", self.summary(), out.stderr.trim()),
            });
        }

        let note = if current.is_empty() {
            String::new()
        } else {
            format!(" (backup at {CRON_BACKUP_PATH})")
        };
        Ok(ActionOutcome {
            success: true,
            message: format!(
                "{} — {} {}{note}",
                self.summary(),
                self.schedule,
                self.command
            ),
        })
    }
}

// --- Pure crontab-text transforms (tested independently of any transport) ---

/// Parse a single line as a user-crontab job, returning `(schedule, command)`.
fn parse_job_line(line: &str) -> Option<(String, String)> {
    parse_crontab(line, false, CronSource::User, "")
        .into_iter()
        .next()
        .map(|e| (e.schedule, e.command))
}

/// Whether an active (uncommented) line is the job `(schedule, command)`.
fn is_active_match(line: &str, schedule: &str, command: &str) -> bool {
    if line.trim_start().starts_with('#') {
        return false;
    }
    parse_job_line(line).is_some_and(|(s, c)| s == schedule && c == command)
}

/// Whether a commented line is the disabled job `(schedule, command)`.
fn is_disabled_match(line: &str, schedule: &str, command: &str) -> bool {
    line.trim_start()
        .strip_prefix('#')
        .and_then(parse_job_line)
        .is_some_and(|(s, c)| s == schedule && c == command)
}

/// Join lines back into crontab text with a trailing newline.
fn join_lines<S: AsRef<str>>(lines: &[S]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}

fn append_job(crontab: &str, schedule: &str, command: &str) -> String {
    let mut lines: Vec<String> = crontab.lines().map(str::to_owned).collect();
    lines.push(format!("{schedule} {command}"));
    join_lines(&lines)
}

fn remove_job(crontab: &str, schedule: &str, command: &str) -> std::result::Result<String, String> {
    let mut removed = false;
    let kept: Vec<&str> = crontab
        .lines()
        .filter(|line| {
            if !removed
                && (is_active_match(line, schedule, command)
                    || is_disabled_match(line, schedule, command))
            {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect();
    if removed {
        Ok(join_lines(&kept))
    } else {
        Err("entry not found".to_owned())
    }
}

fn replace_job(
    crontab: &str,
    old_schedule: &str,
    old_command: &str,
    new_schedule: &str,
    new_command: &str,
) -> std::result::Result<String, String> {
    let mut replaced = false;
    let lines: Vec<String> = crontab
        .lines()
        .map(|line| {
            if !replaced && is_active_match(line, old_schedule, old_command) {
                replaced = true;
                format!("{new_schedule} {new_command}")
            } else if !replaced && is_disabled_match(line, old_schedule, old_command) {
                replaced = true;
                format!("#{new_schedule} {new_command}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    if replaced {
        Ok(join_lines(&lines))
    } else {
        Err("entry not found".to_owned())
    }
}

fn set_enabled(
    crontab: &str,
    schedule: &str,
    command: &str,
    enabled: bool,
) -> std::result::Result<String, String> {
    let mut changed = false;
    let lines: Vec<String> = crontab
        .lines()
        .map(|line| {
            if changed {
                return line.to_owned();
            }
            if enabled && is_disabled_match(line, schedule, command) {
                changed = true;
                // Drop the leading `#` (and any space after it).
                line.trim_start()
                    .strip_prefix('#')
                    .unwrap_or(line)
                    .trim_start()
                    .to_owned()
            } else if !enabled && is_active_match(line, schedule, command) {
                changed = true;
                format!("#{line}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    if changed {
        Ok(join_lines(&lines))
    } else if enabled {
        Err("no disabled entry to enable".to_owned())
    } else {
        Err("no active entry to disable".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CommandOutput;
    use systui_transport::MockTransport;

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(0),
            stdout: stdout.to_owned(),
            stderr: String::new(),
            duration: std::time::Duration::ZERO,
        }
    }

    const SAMPLE: &str = "# header\n0 3 * * * /backup.sh\n*/5 * * * * /poll.sh\n";

    #[test]
    fn append_adds_a_trailing_job() {
        let out = append_job(SAMPLE, "30 4 * * *", "/new.sh");
        assert!(out.ends_with("30 4 * * * /new.sh\n"));
        // Existing jobs are kept.
        assert!(out.contains("0 3 * * * /backup.sh"));
    }

    #[test]
    fn append_into_empty_crontab() {
        let out = append_job("", "@daily", "/x.sh");
        assert_eq!(out, "@daily /x.sh\n");
    }

    #[test]
    fn remove_drops_the_matching_job_only() {
        let out = remove_job(SAMPLE, "0 3 * * *", "/backup.sh").unwrap();
        assert!(!out.contains("/backup.sh"));
        assert!(out.contains("/poll.sh"));
        assert!(out.contains("# header"));
    }

    #[test]
    fn remove_missing_job_errors() {
        assert!(remove_job(SAMPLE, "0 9 * * *", "/nope.sh").is_err());
    }

    #[test]
    fn replace_swaps_schedule_and_command() {
        let out = replace_job(
            SAMPLE,
            "0 3 * * *",
            "/backup.sh",
            "0 4 * * *",
            "/backup2.sh",
        )
        .unwrap();
        assert!(out.contains("0 4 * * * /backup2.sh"));
        assert!(!out.contains("/backup.sh"));
    }

    #[test]
    fn replace_keeps_disabled_jobs_disabled() {
        let out = replace_job(
            "#0 3 * * * /backup.sh\n",
            "0 3 * * *",
            "/backup.sh",
            "0 4 * * *",
            "/backup2.sh",
        )
        .unwrap();
        assert_eq!(out, "#0 4 * * * /backup2.sh\n");
    }

    #[test]
    fn disable_then_enable_round_trips() {
        let disabled = set_enabled(SAMPLE, "*/5 * * * *", "/poll.sh", false).unwrap();
        assert!(disabled.contains("#*/5 * * * * /poll.sh"));
        // The disabled job is no longer an active match.
        assert!(remove_job(&disabled, "*/5 * * * *", "/poll.sh").is_ok());

        let enabled = set_enabled(&disabled, "*/5 * * * *", "/poll.sh", true).unwrap();
        assert!(enabled.contains("*/5 * * * * /poll.sh"));
        assert!(!enabled.contains("#*/5"));
    }

    #[test]
    fn enable_without_a_disabled_entry_errors() {
        assert!(set_enabled(SAMPLE, "0 3 * * *", "/backup.sh", true).is_err());
    }

    #[tokio::test]
    async fn preview_rejects_an_invalid_schedule() {
        let action = CronAction::add("not a schedule", "/x.sh");
        assert!(action.preview(&MockTransport::new()).await.is_err());
    }

    #[tokio::test]
    async fn add_reads_backs_up_and_installs() {
        let transport = MockTransport::new()
            .with_stdout("crontab -l", "0 3 * * * /backup.sh\n")
            .with_command("tee /tmp/systui-crontab.bak", ok(""))
            .with_command("crontab -", ok(""));
        let action = CronAction::add("30 4 * * *", "/new.sh");
        let outcome = action.execute(&transport).await.unwrap();
        assert!(outcome.success, "{}", outcome.message);
        assert!(outcome.message.contains("/new.sh"));
    }

    #[tokio::test]
    async fn delete_missing_entry_is_a_clean_failure() {
        let transport = MockTransport::new().with_stdout("crontab -l", "0 3 * * * /backup.sh\n");
        let action = CronAction::delete("9 9 * * *", "/missing.sh");
        let outcome = action.execute(&transport).await.unwrap();
        assert!(!outcome.success);
        assert!(outcome.message.contains("not found"));
    }

    #[tokio::test]
    async fn run_now_executes_via_shell() {
        let action = CronAction::run_now("0 3 * * *", "/backup.sh --full");
        let preview = action.preview(&MockTransport::new()).await.unwrap();
        assert_eq!(
            preview.command.unwrap().to_string(),
            "sh -c /backup.sh --full"
        );
        assert!(!preview.reversible);
        assert_eq!(action.risk(), RiskLevel::High);

        let transport = MockTransport::new().with_command("sh -c /backup.sh --full", ok("done\n"));
        let outcome = action.execute(&transport).await.unwrap();
        assert!(outcome.success, "{}", outcome.message);
        assert!(outcome.message.contains("/backup.sh"));
    }

    #[test]
    fn risk_classes_match_destructiveness() {
        assert_eq!(
            CronAction::delete("0 0 * * *", "/x").risk(),
            RiskLevel::High
        );
        assert_eq!(CronAction::enable("0 0 * * *", "/x").risk(), RiskLevel::Low);
        assert_eq!(CronAction::add("0 0 * * *", "/x").risk(), RiskLevel::Medium);
        assert!(!CronAction::add("0 0 * * *", "/x").requires_privilege());
    }
}
