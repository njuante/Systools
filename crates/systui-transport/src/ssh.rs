//! Transport that executes against a remote host over SSH.
//!
//! v0.5 wraps the **system OpenSSH client** (`ssh`) rather than embedding an SSH
//! stack: it gives instant compatibility and reuses the operator's existing key
//! auth, ssh-agent, `known_hosts` and `~/.ssh/config` for free (`Product.md`
//! "Fase 9"). Everything SSH-specific is sealed inside this module and behind the
//! [`Transport`] trait, so a native Rust backend can replace it later without the
//! rest of the app noticing.
//!
//! The remote shell re-parses whatever we send, so each [`CommandSpec`] is turned
//! into a single, strictly POSIX-quoted command string in exactly one place
//! ([`build_remote_command`]) — the one spot where a quoting bug would reintroduce
//! shell injection, hence the dedicated tests. Authentication is non-interactive
//! (`BatchMode=yes`): a host without working key/agent auth fails fast instead of
//! blocking on a password prompt.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use systui_core::{CommandOutput, CommandSpec, CoreError, DirEntry, FileType, Result, Transport};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// OpenSSH's reserved exit status for its own (connection/auth) failures, as
/// opposed to the remote command's exit code.
const SSH_FAILURE_CODE: i32 = 255;

/// Default connection timeout when the caller does not override it.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs commands and reads files on a remote host via the system `ssh` client.
#[derive(Debug, Clone)]
pub struct SshTransport {
    host: String,
    user: Option<String>,
    port: u16,
    connect_timeout: Duration,
    label: String,
}

impl SshTransport {
    /// Create a transport for `host` (current user, port 22, default timeout).
    pub fn new(host: impl Into<String>) -> Self {
        let host = host.into();
        let label = make_label(&None, &host, 22);
        Self {
            host,
            user: None,
            port: 22,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            label,
        }
    }

    /// Set the SSH login user.
    #[must_use]
    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.relabel();
        self
    }

    /// Set the SSH port.
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self.relabel();
        self
    }

    /// Set the connection timeout (`-o ConnectTimeout`).
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    fn relabel(&mut self) {
        self.label = make_label(&self.user, &self.host, self.port);
    }

    /// The `ssh` destination, `user@host` when a user is set, else just `host`.
    fn destination(&self) -> String {
        match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        }
    }

    /// The full argument vector passed to `ssh` to run `remote_command`.
    ///
    /// Connection **multiplexing** (`ControlMaster=auto` + a persistent
    /// `ControlPath`) is the key to usable remote performance: a refresh runs
    /// dozens of commands, and without multiplexing each one pays a full SSH
    /// handshake. With it, the first command opens a master connection that the
    /// rest reuse, and `ControlPersist` keeps it warm between refreshes. The
    /// `%C` token makes the socket per-destination, so it falls back cleanly if
    /// the master can't be created.
    fn ssh_args(&self, remote_command: &str) -> Vec<String> {
        vec![
            "-o".to_owned(),
            "BatchMode=yes".to_owned(),
            "-o".to_owned(),
            format!("ConnectTimeout={}", self.connect_timeout.as_secs().max(1)),
            "-o".to_owned(),
            "ControlMaster=auto".to_owned(),
            "-o".to_owned(),
            format!("ControlPath={}", control_path()),
            "-o".to_owned(),
            "ControlPersist=60".to_owned(),
            "-p".to_owned(),
            self.port.to_string(),
            self.destination(),
            remote_command.to_owned(),
        ]
    }

    /// Run a remote command string through `ssh`, optionally feeding `stdin` and
    /// honouring `timeout`. Returns the raw process output; SSH-level vs remote
    /// command failures are disambiguated by the caller via the exit code.
    async fn exec(
        &self,
        remote_command: &str,
        stdin: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<std::process::Output> {
        let mut cmd = Command::new("ssh");
        cmd.args(self.ssh_args(remote_command));
        cmd.kill_on_drop(true);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let exec = run_child(&mut cmd, stdin);
        let output = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, exec)
                .await
                .map_err(|_| CoreError::Timeout(timeout))?,
            None => exec.await,
        };

        output.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                CoreError::Transport("the `ssh` client was not found on this machine".to_owned())
            }
            _ => CoreError::Io(e),
        })
    }

    /// Map the result of an SSH invocation that should have produced output: an
    /// SSH-level failure (code 255) becomes a [`CoreError::Transport`].
    fn ssh_failed(&self, output: &std::process::Output) -> Option<CoreError> {
        (output.status.code() == Some(SSH_FAILURE_CODE)).then(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                "no output from ssh"
            } else {
                detail
            };
            CoreError::Transport(format!(
                "ssh connection to {} failed: {detail} \
                 (SysTUI connects non-interactively — check your SSH key/agent and known_hosts)",
                self.label
            ))
        })
    }
}

#[async_trait]
impl Transport for SshTransport {
    fn label(&self) -> &str {
        &self.label
    }

    async fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
        let start = Instant::now();
        let remote = build_remote_command(command);
        let output = self
            .exec(&remote, command.stdin.as_deref(), command.timeout)
            .await?;

        // A 255 is an SSH/connection error; any other code is the remote
        // command's own exit status and is returned verbatim for the caller to
        // interpret (e.g. via `CommandOutput::into_result`).
        if let Some(err) = self.ssh_failed(&output) {
            return Err(err);
        }
        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration: start.elapsed(),
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let remote = build_remote_command(&CommandSpec::new("cat").arg("--").arg(path));
        let output = self.exec(&remote, None, None).await?;
        if let Some(err) = self.ssh_failed(&output) {
            return Err(err);
        }
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(classify_fs_error(
                &String::from_utf8_lossy(&output.stderr),
                path,
            ))
        }
    }

    async fn file_exists(&self, path: &str) -> Result<bool> {
        let remote = build_remote_command(&CommandSpec::new("test").arg("-e").arg(path));
        let output = self.exec(&remote, None, None).await?;
        if let Some(err) = self.ssh_failed(&output) {
            return Err(err);
        }
        // `test -e` exits 0 when the path exists, 1 otherwise.
        Ok(output.status.success())
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        // GNU find prints one `<type-char>\t<name>` line per entry; the format
        // string is quoted so find (not the shell) expands the escapes.
        let remote = build_remote_command(&CommandSpec::new("find").args([
            path,
            "-maxdepth",
            "1",
            "-mindepth",
            "1",
            "-printf",
            "%y\\t%f\\n",
        ]));
        let output = self.exec(&remote, None, None).await?;
        if let Some(err) = self.ssh_failed(&output) {
            return Err(err);
        }
        if output.status.success() {
            Ok(parse_find_listing(&String::from_utf8_lossy(&output.stdout)))
        } else {
            Err(classify_fs_error(
                &String::from_utf8_lossy(&output.stderr),
                path,
            ))
        }
    }
}

/// Spawn `cmd`, optionally writing `input` to its stdin, and collect its output.
async fn run_child(
    cmd: &mut Command,
    input: Option<&str>,
) -> std::io::Result<std::process::Output> {
    let mut child = cmd.spawn()?;
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(input.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    child.wait_with_output().await
}

/// Build a `user@host:port`-style label for display.
fn make_label(user: &Option<String>, host: &str, port: u16) -> String {
    let authority = match user {
        Some(user) => format!("{user}@{host}"),
        None => host.to_owned(),
    };
    if port == 22 {
        format!("ssh://{authority}")
    } else {
        format!("ssh://{authority}:{port}")
    }
}

/// The `ControlPath` socket for SSH connection multiplexing. The `%C` token is
/// expanded by `ssh` into a hash of the connection parameters, so the socket is
/// short, unique per destination and lives in the temp dir (not the repo).
fn control_path() -> String {
    format!("{}/systui-ssh-%C", std::env::temp_dir().display())
}

/// Turn a [`CommandSpec`] into a single remote command string, POSIX-quoting the
/// program and every argument so the remote login shell re-parses it back into
/// exactly the same argument vector — no shell injection, no quoting surprises.
pub fn build_remote_command(spec: &CommandSpec) -> String {
    let mut parts = Vec::with_capacity(spec.args.len() + 1);
    parts.push(quote_arg(&spec.program));
    for arg in &spec.args {
        parts.push(quote_arg(arg));
    }
    parts.join(" ")
}

/// POSIX-quote a single argument. Safe tokens pass through unquoted; everything
/// else is wrapped in single quotes with embedded `'` escaped as `'\''`.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && arg.bytes().all(is_shell_safe) {
        return arg.to_owned();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Characters that never need quoting in a POSIX shell.
fn is_shell_safe(b: u8) -> bool {
    matches!(b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
        | b'_' | b'-' | b'.' | b'/' | b',' | b':' | b'@' | b'=' | b'+')
}

/// Parse GNU `find -printf '%y\t%f\n'` output into directory entries.
fn parse_find_listing(stdout: &str) -> Vec<DirEntry> {
    stdout
        .lines()
        .filter_map(|line| {
            let (type_char, name) = line.split_once('\t')?;
            if name.is_empty() {
                return None;
            }
            let file_type = match type_char {
                "d" => FileType::Dir,
                "l" => FileType::Symlink,
                "f" => FileType::File,
                _ => FileType::Other,
            };
            Some(DirEntry {
                name: name.to_owned(),
                file_type,
            })
        })
        .collect()
}

/// Classify a remote filesystem error from a command's stderr.
fn classify_fs_error(stderr: &str, path: &str) -> CoreError {
    let lower = stderr.to_lowercase();
    if lower.contains("permission denied") {
        CoreError::PermissionDenied(path.to_owned())
    } else if lower.contains("no such file") || lower.contains("not found") {
        CoreError::FileNotFound(PathBuf::from(path))
    } else {
        CoreError::Transport(stderr.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_tokens_are_not_quoted() {
        assert_eq!(quote_arg("systemctl"), "systemctl");
        assert_eq!(quote_arg("/etc/ssh/sshd_config"), "/etc/ssh/sshd_config");
        assert_eq!(quote_arg("-tulpn"), "-tulpn");
        assert_eq!(quote_arg("ssh,sshd"), "ssh,sshd");
    }

    #[test]
    fn unsafe_tokens_are_single_quoted() {
        // A space forces quoting.
        assert_eq!(quote_arg("15 min ago"), "'15 min ago'");
        // An embedded single quote is escaped as '\''.
        assert_eq!(quote_arg("a'b"), "'a'\\''b'");
        // The empty string must still produce a token.
        assert_eq!(quote_arg(""), "''");
        // Shell metacharacters are neutralised.
        assert_eq!(quote_arg("a; rm -rf /"), "'a; rm -rf /'");
        assert_eq!(quote_arg("$(whoami)"), "'$(whoami)'");
    }

    #[test]
    fn builds_remote_command_preserving_arguments() {
        let spec = CommandSpec::new("stat").args(["-c", "%a %U %G %n", "/etc/passwd"]);
        assert_eq!(
            build_remote_command(&spec),
            "stat -c '%a %U %G %n' /etc/passwd"
        );
    }

    #[test]
    fn remote_command_neutralises_injection_in_arguments() {
        // A malicious "path" stays a single argument to cat, not extra shell.
        let spec = CommandSpec::new("cat")
            .arg("--")
            .arg("/etc/passwd; rm -rf /");
        assert_eq!(
            build_remote_command(&spec),
            "cat -- '/etc/passwd; rm -rf /'"
        );
    }

    #[test]
    fn ssh_args_carry_batchmode_port_and_destination() {
        let t = SshTransport::new("prod-01").user("admin").port(2222);
        let args = t.ssh_args("uname -a");
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(args.contains(&"-p".to_owned()));
        assert!(args.contains(&"2222".to_owned()));
        assert!(args.contains(&"admin@prod-01".to_owned()));
        // The remote command is the final argument.
        assert_eq!(args.last().unwrap(), "uname -a");
        // ConnectTimeout is present.
        assert!(args.iter().any(|a| a.starts_with("ConnectTimeout=")));
        // Connection multiplexing is enabled (the key to remote performance).
        assert!(args.windows(2).any(|w| w == ["-o", "ControlMaster=auto"]));
        assert!(args.iter().any(|a| a.starts_with("ControlPath=")));
        assert!(args.windows(2).any(|w| w == ["-o", "ControlPersist=60"]));
    }

    #[test]
    fn label_reflects_user_and_nondefault_port() {
        assert_eq!(SshTransport::new("h").label(), "ssh://h");
        assert_eq!(SshTransport::new("h").user("u").label(), "ssh://u@h");
        assert_eq!(
            SshTransport::new("h").user("u").port(2222).label(),
            "ssh://u@h:2222"
        );
    }

    #[test]
    fn destination_falls_back_to_host_without_user() {
        assert_eq!(SshTransport::new("10.0.0.1").destination(), "10.0.0.1");
        assert_eq!(
            SshTransport::new("10.0.0.1").user("root").destination(),
            "root@10.0.0.1"
        );
    }

    #[test]
    fn parses_find_listing_into_typed_entries() {
        let out = "d\tcron.d\nf\tbackup\nl\tlink\nf\t90-cloud-init\n";
        let entries = parse_find_listing(out);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].name, "cron.d");
        assert_eq!(entries[0].file_type, FileType::Dir);
        assert_eq!(entries[1].file_type, FileType::File);
        assert_eq!(entries[2].file_type, FileType::Symlink);
    }

    #[test]
    fn representative_collector_commands_round_trip() {
        // Real commands issued by the collectors must survive the SSH boundary
        // unchanged in meaning: safe tokens pass through, only those with shell
        // metacharacters (spaces, braces) get quoted — preserving the argv.
        let cases = [
            (
                CommandSpec::new("ps").args(["-eo", "pid,ppid,user,pcpu,pmem,comm"]),
                "ps -eo pid,ppid,user,pcpu,pmem,comm",
            ),
            (
                CommandSpec::new("systemctl").args([
                    "list-units",
                    "--type=service",
                    "--all",
                    "--no-legend",
                    "--no-pager",
                ]),
                "systemctl list-units --type=service --all --no-legend --no-pager",
            ),
            (CommandSpec::new("ss").arg("-tulpn"), "ss -tulpn"),
            (
                CommandSpec::new("docker").args([
                    "ps",
                    "-a",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                ]),
                "docker ps -a --no-trunc --format '{{json .}}'",
            ),
            (
                CommandSpec::new("journalctl").args(["-u", "ssh", "--no-pager", "-n", "2000"]),
                "journalctl -u ssh --no-pager -n 2000",
            ),
            (
                CommandSpec::new("stat").args(["-c", "%a %U %G %n", "/etc/passwd"]),
                "stat -c '%a %U %G %n' /etc/passwd",
            ),
        ];
        for (spec, expected) in cases {
            assert_eq!(build_remote_command(&spec), expected);
        }
    }

    #[test]
    fn classifies_remote_fs_errors() {
        assert!(matches!(
            classify_fs_error("cat: /x: Permission denied", "/x"),
            CoreError::PermissionDenied(_)
        ));
        assert!(matches!(
            classify_fs_error("cat: /x: No such file or directory", "/x"),
            CoreError::FileNotFound(_)
        ));
        assert!(matches!(
            classify_fs_error("some other failure", "/x"),
            CoreError::Transport(_)
        ));
    }
}
