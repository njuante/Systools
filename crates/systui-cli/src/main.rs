//! SysTUI command-line entry point.
//!
//! Wires argument parsing, logging and configuration loading, then resolves the
//! execution mode and dispatches to a mode handler. Local and SSH launch the TUI;
//! report is local-only for now and fleet lands in a later phase.

mod cli;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use systui_core::{Config, ExecutionMode, Transport};
use systui_report::FleetHostSummary;
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
        Command::Fleet { tag, favorites } => {
            run_fleet(tag, favorites, mode, config)?;
        }
        Command::Report {
            host,
            format,
            security,
            output,
            note,
        } => {
            run_report(host, &format, security, output, note, mode, config)?;
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

/// Maximum number of hosts reviewed concurrently. Bounds the in-flight `ssh`
/// connections so a large fleet does not exhaust local resources.
const FLEET_CONCURRENCY: usize = 8;
/// Per-host review timeout. A slow or hung host degrades to an error row instead
/// of stalling the whole fleet.
const FLEET_HOST_TIMEOUT: Duration = Duration::from_secs(30);

/// Review a fleet of inventory hosts and print a worst-first overview.
///
/// Hosts are selected from the inventory by the tag/favorites filter, then their
/// state is gathered **concurrently** over SSH (bounded by [`FLEET_CONCURRENCY`],
/// each capped by [`FLEET_HOST_TIMEOUT`]). Every host is isolated: an unreachable
/// host, an auth failure or a timeout becomes a "failed" row and never aborts the
/// run. Fleet mode is inspection-only — no actions are taken.
fn run_fleet(
    tags: Vec<String>,
    favorites: bool,
    mode: ExecutionMode,
    config: &Config,
) -> anyhow::Result<()> {
    let filter = systui_core::FleetFilter {
        tags,
        favorites_only: favorites,
    };
    let selected = config.select_fleet(&filter);

    if config.hosts.is_empty() {
        eprintln!("No hosts in the inventory. Add `[hosts.<id>]` entries to your config.");
        return Ok(());
    }
    if selected.is_empty() {
        eprintln!("No inventory hosts match the given filters.");
        return Ok(());
    }

    let runtime = tokio::runtime::Runtime::new().context("failed to start async runtime")?;
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    eprintln!("Reviewing {} host(s)…", selected.len());

    let overview = runtime.block_on(gather_fleet(selected, mode, config, generated_at));
    print_fleet_overview(&overview);
    Ok(())
}

/// Gather every selected host concurrently (bounded) into a [`FleetOverview`].
async fn gather_fleet(
    hosts: Vec<systui_core::FleetHost>,
    mode: ExecutionMode,
    config: &Config,
    generated_at: String,
) -> systui_report::FleetOverview {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(FLEET_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();

    for host in hosts {
        let semaphore = Arc::clone(&semaphore);
        let config = config.clone();
        let generated_at = generated_at.clone();
        tasks.spawn(async move {
            // Permit is held for the duration of this host's review, bounding
            // concurrent ssh connections. The semaphore is never closed.
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("fleet semaphore is never closed");
            review_one_host(host, mode, &config, generated_at).await
        });
    }

    let mut summaries = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(summary) => summaries.push(summary),
            // A panicking review task should not lose the rest of the fleet.
            Err(err) => tracing::error!(%err, "fleet review task failed to join"),
        }
    }

    systui_report::FleetOverview::build(generated_at, summaries)
}

/// Review a single host over SSH, mapping any failure (connection, auth, timeout)
/// to a failed summary row.
async fn review_one_host(
    host: systui_core::FleetHost,
    mode: ExecutionMode,
    config: &Config,
    generated_at: String,
) -> systui_report::FleetHostSummary {
    let mut transport = systui_transport::SshTransport::new(host.host.clone()).port(host.port);
    if let Some(user) = &host.user {
        transport = transport.user(user.clone());
    }
    let effective_mode = if host.read_only {
        ExecutionMode::ReadOnly
    } else {
        mode
    };

    let gather = systui_report::gather_report(
        &transport,
        config,
        host.id.clone(),
        effective_mode,
        generated_at,
        Vec::new(),
    );

    match tokio::time::timeout(FLEET_HOST_TIMEOUT, gather).await {
        Ok(Ok(report)) => FleetHostSummary::reviewed(host.id, host.tags, host.favorite, &report),
        Ok(Err(err)) => {
            FleetHostSummary::failed(host.id, host.tags, host.favorite, err.to_string())
        }
        Err(_) => FleetHostSummary::failed(
            host.id,
            host.tags,
            host.favorite,
            format!("timed out after {}s", FLEET_HOST_TIMEOUT.as_secs()),
        ),
    }
}

/// Print the fleet overview as a worst-first table.
fn print_fleet_overview(overview: &systui_report::FleetOverview) {
    use systui_report::FleetOutcome;

    println!(
        "Fleet overview — {} reviewed, {} unreachable (generated {}):\n",
        overview.reviewed_count(),
        overview.failed_count(),
        overview.generated_at,
    );
    for host in &overview.hosts {
        let marker = if host.favorite { "*" } else { " " };
        let tags = if host.tags.is_empty() {
            "-".to_owned()
        } else {
            host.tags.join(",")
        };
        let status = match &host.outcome {
            FleetOutcome::Reviewed {
                health,
                finding_counts,
                ..
            } => format!("{health:>3}/100  {}", findings_summary(finding_counts)),
            FleetOutcome::Failed { error } => format!("  ERR  {error}"),
        };
        println!("{marker} {:<16} {:<16} {status}", host.id, tags);
    }
}

/// A one-line findings summary from `[critical, high, medium, low, info]`.
fn findings_summary(counts: &[usize; 5]) -> String {
    let mut parts = Vec::new();
    for (count, label) in [(counts[0], "crit"), (counts[1], "high"), (counts[2], "med")] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if parts.is_empty() {
        "OK".to_owned()
    } else {
        parts.join(", ")
    }
}

/// Generate a report of a host's state, locally or over SSH (`--host`), in
/// Markdown, JSON or HTML, optionally security-scoped and written to a file.
fn run_report(
    host: Option<String>,
    format: &str,
    security: bool,
    output: Option<PathBuf>,
    notes: Vec<String>,
    mode: ExecutionMode,
    config: &Config,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("failed to start async runtime")?;
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let report = match host {
        Some(target) => {
            let resolved = config.resolve_target(&target);
            let mut transport =
                systui_transport::SshTransport::new(resolved.host.clone()).port(resolved.port);
            if let Some(user) = &resolved.user {
                transport = transport.user(user.clone());
            }
            let label = if resolved.from_inventory {
                resolved.id.clone()
            } else {
                transport.label().to_owned()
            };
            let effective_mode = if resolved.read_only {
                ExecutionMode::ReadOnly
            } else {
                mode
            };
            eprintln!("Connecting to {label} …");
            runtime
                .block_on(systui_report::gather_report(
                    &transport,
                    config,
                    label,
                    effective_mode,
                    generated_at,
                    notes,
                ))
                .context("failed to gather report over ssh")?
        }
        None => {
            let transport = systui_transport::LocalTransport::new();
            runtime
                .block_on(systui_report::gather_report(
                    &transport,
                    config,
                    systui_core::HostId::LOCAL,
                    mode,
                    generated_at,
                    notes,
                ))
                .context("failed to gather host report")?
        }
    };

    let scope = if security {
        systui_report::ReportScope::Security
    } else {
        systui_report::ReportScope::Full
    };
    let rendered = match format {
        "markdown" | "md" => systui_report::to_markdown(&report, scope),
        "json" => systui_report::to_json(&report),
        "html" => systui_report::to_html(&report, scope),
        other => anyhow::bail!("unknown report format `{other}`; use markdown, json or html"),
    };

    match output {
        Some(path) => {
            std::fs::write(&path, rendered)
                .with_context(|| format!("failed to write report to {}", path.display()))?;
            eprintln!("Report written to {}", path.display());
        }
        None => print!("{rendered}"),
    }
    Ok(())
}
