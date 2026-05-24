//! Detect what the connected user can actually do on the host, so the UI can
//! explain limited data and SysTUI can degrade its execution mode honestly.
//!
//! This is transport-agnostic: it runs the same `id` / `sudo -n` probes locally
//! or over SSH. A non-privileged user (not root, no passwordless sudo) cannot run
//! privileged actions, so [`HostCapabilities::effective_mode`] downgrades a
//! requested `Privileged` mode to `SafeActions`. Missing `id`/`sudo` binaries
//! degrade to "unknown / non-privileged" rather than crashing.

use systui_core::{CommandSpec, ExecutionMode, Transport};

/// What the connected user is able to do on the target host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilities {
    /// Login name, e.g. `admin`; `"unknown"` if `id` could not be read.
    pub user: String,
    /// Numeric uid; `None` when unknown.
    pub uid: Option<u32>,
    /// Whether `sudo -n` works without a password prompt.
    pub can_sudo: bool,
}

impl HostCapabilities {
    /// Whether the user is root (uid 0).
    pub fn is_root(&self) -> bool {
        self.uid == Some(0)
    }

    /// Whether privileged operations are likely to succeed (root or sudo).
    pub fn is_privileged(&self) -> bool {
        self.is_root() || self.can_sudo
    }

    /// Downgrade a requested mode to what this user can actually do. A
    /// non-privileged user cannot run privileged actions, so `Privileged` becomes
    /// `SafeActions`. `ReadOnly` and `SafeActions` are never upgraded.
    pub fn effective_mode(&self, requested: ExecutionMode) -> ExecutionMode {
        match requested {
            ExecutionMode::Privileged if !self.is_privileged() => ExecutionMode::SafeActions,
            other => other,
        }
    }

    /// Short label for the UI, e.g. `admin (root)`, `admin (sudo)` or
    /// `admin (no sudo)`.
    pub fn label(&self) -> String {
        let suffix = if self.is_root() {
            "root"
        } else if self.can_sudo {
            "sudo"
        } else {
            "no sudo"
        };
        format!("{} ({suffix})", self.user)
    }
}

/// Probe the host for the connected user's identity and sudo capability.
pub async fn probe_capabilities(transport: &dyn Transport) -> HostCapabilities {
    let (uid, user) = match run_stdout(transport, "id", &[]).await {
        Some(out) => parse_id(&out),
        None => (None, None),
    };
    let is_root = uid == Some(0);
    // Root implicitly has every privilege; otherwise test passwordless sudo.
    let can_sudo = is_root || command_succeeds(transport, "sudo", &["-n", "true"]).await;

    HostCapabilities {
        user: user.unwrap_or_else(|| "unknown".to_owned()),
        uid,
        can_sudo,
    }
}

/// Parse the uid and login name out of `id` output
/// (`uid=1000(admin) gid=1000(admin) groups=...`).
fn parse_id(output: &str) -> (Option<u32>, Option<String>) {
    let Some(field) = output
        .split_whitespace()
        .find_map(|t| t.strip_prefix("uid="))
    else {
        return (None, None);
    };
    match field.split_once('(') {
        Some((num, rest)) => (num.parse().ok(), rest.strip_suffix(')').map(str::to_owned)),
        None => (field.parse().ok(), None),
    }
}

async fn run_stdout(transport: &dyn Transport, program: &str, args: &[&str]) -> Option<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    match transport.run(&spec).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

async fn command_succeeds(transport: &dyn Transport, program: &str, args: &[&str]) -> bool {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    matches!(transport.run(&spec).await, Ok(out) if out.success())
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

    fn fail() -> CommandOutput {
        CommandOutput {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
            duration: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn parses_id_output() {
        assert_eq!(
            parse_id("uid=1000(admin) gid=1000(admin) groups=27(sudo)"),
            (Some(1000), Some("admin".to_owned()))
        );
        assert_eq!(
            parse_id("uid=0(root) gid=0(root) groups=0(root)"),
            (Some(0), Some("root".to_owned()))
        );
        assert_eq!(parse_id("garbage"), (None, None));
    }

    #[test]
    fn effective_mode_downgrades_only_unprivileged_privileged() {
        let priv_user = HostCapabilities {
            user: "admin".into(),
            uid: Some(1000),
            can_sudo: true,
        };
        let plain_user = HostCapabilities {
            user: "joe".into(),
            uid: Some(1001),
            can_sudo: false,
        };

        // Privileged is preserved for a sudo-capable user, downgraded otherwise.
        assert_eq!(
            priv_user.effective_mode(ExecutionMode::Privileged),
            ExecutionMode::Privileged
        );
        assert_eq!(
            plain_user.effective_mode(ExecutionMode::Privileged),
            ExecutionMode::SafeActions
        );
        // Lower modes are never upgraded.
        assert_eq!(
            priv_user.effective_mode(ExecutionMode::ReadOnly),
            ExecutionMode::ReadOnly
        );
        assert_eq!(
            plain_user.effective_mode(ExecutionMode::SafeActions),
            ExecutionMode::SafeActions
        );
    }

    #[tokio::test]
    async fn probes_root_without_calling_sudo() {
        // Only `id` is configured; root must still report privileged.
        let transport =
            MockTransport::new().with_command("id", ok("uid=0(root) gid=0(root) groups=0(root)"));
        let caps = probe_capabilities(&transport).await;
        assert!(caps.is_root());
        assert!(caps.is_privileged());
        assert_eq!(caps.label(), "root (root)");
    }

    #[tokio::test]
    async fn probes_sudo_capable_user() {
        let transport = MockTransport::new()
            .with_command("id", ok("uid=1000(admin) gid=1000(admin) groups=27(sudo)"))
            .with_command("sudo -n true", ok(""));
        let caps = probe_capabilities(&transport).await;
        assert!(!caps.is_root());
        assert!(caps.can_sudo);
        assert!(caps.is_privileged());
        assert_eq!(caps.label(), "admin (sudo)");
    }

    #[tokio::test]
    async fn probes_unprivileged_user() {
        let transport = MockTransport::new()
            .with_command("id", ok("uid=1001(joe) gid=1001(joe) groups=1001(joe)"))
            .with_command("sudo -n true", fail());
        let caps = probe_capabilities(&transport).await;
        assert!(!caps.is_privileged());
        assert_eq!(caps.label(), "joe (no sudo)");
    }

    #[tokio::test]
    async fn degrades_to_unknown_when_id_is_missing() {
        // Nothing configured: `id` fails, `sudo` fails → unknown, non-privileged.
        let caps = probe_capabilities(&MockTransport::new()).await;
        assert_eq!(caps.user, "unknown");
        assert_eq!(caps.uid, None);
        assert!(!caps.is_privileged());
    }
}
