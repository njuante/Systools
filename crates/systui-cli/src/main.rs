//! SysTUI command-line entry point.
//!
//! Wires argument parsing, logging and configuration loading, then resolves the
//! execution mode and dispatches to a mode handler. Local and SSH launch the TUI;
//! report is local-only for now and fleet lands in a later phase.

mod cli;

use anyhow::Context;
use clap::Parser;
use systui_core::{Config, ExecutionMode, Transport};
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    init_tracing();

    let config = load_config(&args).context("failed to load configuration")?;
    let mode = resolve_mode(&args);
    tracing::info!(%mode, "starting systui");

    dispatch(args.command.unwrap_or(Command::Local), mode, &config)
}

/// Initialise tracing to stderr. Verbosity is controlled by `SYSTUI_LOG`
/// (e.g. `SYSTUI_LOG=debug`); defaults to `warn`.
fn init_tracing() {
    let filter = EnvFilter::try_from_env("SYSTUI_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Load config from `--config` if given, otherwise the default location.
fn load_config(args: &Cli) -> systui_core::Result<Config> {
    match &args.config {
        Some(path) => systui_storage::load_config_from(path),
        None => systui_storage::load_config(),
    }
}

/// Resolve the execution mode from CLI flags.
///
/// `--read-only` forces [`ExecutionMode::ReadOnly`]. Otherwise an interactive
/// session runs as [`ExecutionMode::Privileged`], where dangerous actions are
/// gated by confirmations rather than by the mode. Per-host `read_only` profiles
/// and finer modes arrive with the action engine (phase 2).
fn resolve_mode(args: &Cli) -> ExecutionMode {
    if args.read_only {
        ExecutionMode::ReadOnly
    } else {
        ExecutionMode::Privileged
    }
}

fn dispatch(command: Command, mode: ExecutionMode, config: &Config) -> anyhow::Result<()> {
    match command {
        Command::Local => {
            let transport: Box<dyn systui_core::Transport> =
                Box::new(systui_transport::LocalTransport::new());
            systui_ui::run(transport, systui_core::HostId::LOCAL, mode, config)?;
        }
        Command::Ssh { target } => {
            run_ssh(&target, mode, config)?;
        }
        Command::Fleet { tag } => {
            let scope = tag.as_deref().unwrap_or("all hosts");
            println!("systui: fleet mode [{scope}] ({mode}) — implemented in phase 8");
        }
        Command::Report { host, format } => {
            run_report(host, &format, mode, config)?;
        }
    }
    Ok(())
}

/// Resolve an SSH target (an inventory id or `user@host`) and launch the TUI
/// against it over [`SshTransport`].
///
/// A per-host `read_only` profile forces read-only mode regardless of CLI flags.
/// The transport is stateless — each command opens its own `ssh` connection — so
/// a dropped link self-heals on the next refresh without explicit reconnect
/// logic; host-key verification and auth are delegated to the system ssh client
/// (`known_hosts`, `~/.ssh/config`, ssh-agent).
fn run_ssh(target: &str, mode: ExecutionMode, config: &Config) -> anyhow::Result<()> {
    let resolved = config.resolve_target(target);
    let effective_mode = if resolved.read_only {
        ExecutionMode::ReadOnly
    } else {
        mode
    };

    let mut transport =
        systui_transport::SshTransport::new(resolved.host.clone()).port(resolved.port);
    if let Some(user) = &resolved.user {
        transport = transport.user(user.clone());
    }

    // Friendly title: the inventory id when known, else the ssh:// destination.
    let label = if resolved.from_inventory {
        resolved.id.clone()
    } else {
        transport.label().to_owned()
    };
    tracing::info!(host = %label, %effective_mode, "connecting over ssh");
    // The first refresh opens the SSH connection (up to the connect timeout)
    // before the TUI takes the screen, so give immediate feedback.
    eprintln!("Connecting to {label} …");

    systui_ui::run(Box::new(transport), label, effective_mode, config)?;
    Ok(())
}

/// Generate a report for the local host. Remote (`--host`), JSON/HTML and section
/// flags are wired up in S6.5; for now this renders a local Markdown report.
fn run_report(
    host: Option<String>,
    format: &str,
    mode: ExecutionMode,
    config: &Config,
) -> anyhow::Result<()> {
    if let Some(host) = host {
        anyhow::bail!(
            "remote reports for `{host}` require the report CLI wiring (S6.5); only local is available now"
        );
    }
    if format != "markdown" {
        anyhow::bail!("report format `{format}` is not wired yet; use --format markdown");
    }

    let runtime = tokio::runtime::Runtime::new().context("failed to start async runtime")?;
    let transport = systui_transport::LocalTransport::new();
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let report = runtime
        .block_on(systui_report::gather_report(
            &transport,
            config,
            systui_core::HostId::LOCAL,
            mode,
            generated_at,
            Vec::new(),
        ))
        .context("failed to gather host report")?;

    print!("{}", systui_report::to_markdown(&report));
    Ok(())
}
