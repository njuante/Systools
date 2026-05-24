//! SysTUI command-line entry point.
//!
//! Wires argument parsing, logging and configuration loading, then resolves the
//! execution mode and dispatches to a mode handler. Local and SSH launch the TUI;
//! report is local-only for now and fleet lands in a later phase.

mod cli;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use systui_core::{Config, ExecutionMode, Transport};
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    init_tracing();

    let config = load_config(&args).context("failed to load configuration")?;
    let config_path = config_path(&args);
    let mode = resolve_mode(&args);
    tracing::info!(%mode, "starting systui");

    dispatch(
        args.command.unwrap_or(Command::Local),
        mode,
        &config,
        &config_path,
    )
}

/// The config file path to persist edits to: `--config` if given, else the default
/// location (falling back to `config.toml` in the cwd if the home dir is unknown).
fn config_path(args: &Cli) -> PathBuf {
    match &args.config {
        Some(path) => path.clone(),
        None => {
            systui_storage::paths::config_file().unwrap_or_else(|_| PathBuf::from("config.toml"))
        }
    }
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

fn dispatch(
    command: Command,
    mode: ExecutionMode,
    config: &Config,
    config_path: &Path,
) -> anyhow::Result<()> {
    match command {
        Command::Local => {
            let transport: Box<dyn systui_core::Transport> =
                Box::new(systui_transport::LocalTransport::new());
            systui_ui::run(transport, systui_core::HostId::LOCAL, mode, config)?;
        }
        Command::Ssh { target } => {
            run_ssh(&target, mode, config)?;
        }
        Command::Fleet {
            tag,
            favorites,
            search,
            compare,
            format,
            output,
        } => {
            run_fleet(
                tag,
                favorites,
                search,
                compare,
                format,
                output,
                mode,
                config,
                config_path,
            )?;
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

/// Review a fleet of inventory hosts and show a worst-first overview.
///
/// Hosts are selected from the inventory by the tag/favorites filter, then their
/// state is gathered **concurrently** over SSH (bounded by [`FLEET_CONCURRENCY`],
/// each capped by [`FLEET_HOST_TIMEOUT`]). Every host is isolated: an unreachable
/// host, an auth failure or a timeout becomes a "failed" row and never aborts the
/// run. Fleet mode is inspection-only — no actions are taken.
///
/// On a terminal this opens the interactive fleet TUI (with drill-in to a host);
/// when piped/redirected it prints the overview once, so the command is scriptable.
#[allow(clippy::too_many_arguments)]
fn run_fleet(
    tags: Vec<String>,
    favorites: bool,
    search: Option<String>,
    compare: Vec<String>,
    format: Option<String>,
    output: Option<PathBuf>,
    mode: ExecutionMode,
    config: &Config,
    config_path: &Path,
) -> anyhow::Result<()> {
    if config.hosts.is_empty() && compare.len() != 2 {
        eprintln!("No hosts in the inventory. Add `[hosts.<id>]` entries to your config.");
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new().context("failed to start async runtime")?;

    // Comparison mode: gather exactly the two requested hosts and diff them.
    if let [left, right] = compare.as_slice() {
        return run_fleet_compare(left, right, mode, config, &runtime);
    }

    let filter = systui_core::FleetFilter {
        tags,
        favorites_only: favorites,
    };

    // The headless modes (search, report, piped overview) need one eager gather of
    // the current selection.
    let interactive = search.is_none() && format.is_none() && std::io::stdout().is_terminal();
    if !interactive {
        let selected = config.select_fleet(&filter);
        if selected.is_empty() {
            eprintln!("No inventory hosts match the given filters.");
            return Ok(());
        }
        eprintln!("Reviewing {} host(s)…", selected.len());
        let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let review = runtime.block_on(gather_fleet(selected, mode, config, generated_at));

        if let Some(term) = search {
            print_fleet_search(&review, &term);
        } else if let Some(format) = format {
            render_fleet_report(&review, &format, output)?;
        } else {
            print_fleet_overview(&review.overview());
        }
        return Ok(());
    }

    // Interactive TUI: an editable inventory + drill-in. The fleet view re-gathers
    // through the closure after edits, so changes appear without leaving the screen.
    let mut config = config.clone();
    let read_only = mode == ExecutionMode::ReadOnly;
    loop {
        let gather_overview = |cfg: &Config| {
            let selected = cfg.select_fleet(&filter);
            let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            runtime
                .block_on(gather_fleet(selected, mode, cfg, generated_at))
                .overview()
        };
        match systui_ui::run_fleet(&mut config, config_path, read_only, gather_overview)? {
            systui_ui::FleetExit::Quit => break,
            systui_ui::FleetExit::Enter(id) => {
                let selected = config.select_fleet(&filter);
                if let Some(host) = selected.iter().find(|h| h.id == id) {
                    drill_in_host(host, mode, &config)?;
                }
            }
        }
    }
    Ok(())
}

/// Render a fleet report from a gathered review and write it to a file or stdout.
fn render_fleet_report(
    review: &systui_report::FleetReview,
    format: &str,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let report = systui_report::FleetReport::from_review(review);
    let rendered = match format {
        "markdown" | "md" => systui_report::fleet_to_markdown(&report),
        "json" => systui_report::fleet_to_json(&report),
        "html" => systui_report::fleet_to_html(&report),
        other => anyhow::bail!("unknown report format `{other}`; use markdown, json or html"),
    };
    match output {
        Some(path) => {
            std::fs::write(&path, rendered)
                .with_context(|| format!("failed to write report to {}", path.display()))?;
            eprintln!("Fleet report written to {}", path.display());
        }
        None => print!("{rendered}"),
    }
    Ok(())
}

/// Gather two inventory hosts and print a side-by-side comparison with drift.
fn run_fleet_compare(
    left_id: &str,
    right_id: &str,
    mode: ExecutionMode,
    config: &Config,
    runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let all = config.select_fleet(&systui_core::FleetFilter::all());
    let find = |id: &str| all.iter().find(|h| h.id == id).cloned();
    let (Some(left), Some(right)) = (find(left_id), find(right_id)) else {
        anyhow::bail!("both hosts must be inventory ids; unknown: {left_id} or {right_id}");
    };

    eprintln!("Reviewing {left_id} and {right_id}…");
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let review = runtime.block_on(gather_fleet(vec![left, right], mode, config, generated_at));

    let (Some(lr), Some(rr)) = (review.report(left_id), review.report(right_id)) else {
        anyhow::bail!("could not review both hosts (see the rows below for failures)");
    };
    let comparison = systui_report::HostComparison::new(left_id, lr, right_id, rr);
    print_fleet_comparison(&comparison);
    Ok(())
}

/// Drill from the fleet overview into a single host: launch the per-host TUI over
/// SSH, honouring the host's `read_only` profile. Returns when the operator exits
/// that host, so the caller can re-show the fleet overview.
fn drill_in_host(
    host: &systui_core::FleetHost,
    mode: ExecutionMode,
    config: &Config,
) -> anyhow::Result<()> {
    let mut transport = systui_transport::SshTransport::new(host.host.clone()).port(host.port);
    if let Some(user) = &host.user {
        transport = transport.user(user.clone());
    }
    let effective_mode = if host.read_only {
        ExecutionMode::ReadOnly
    } else {
        mode
    };
    systui_ui::run(Box::new(transport), host.id.clone(), effective_mode, config)?;
    Ok(())
}

/// Gather every selected host concurrently (bounded) into a [`FleetReview`],
/// keeping each host's full report so the overview, search and comparison can all
/// work from one gather.
async fn gather_fleet(
    hosts: Vec<systui_core::FleetHost>,
    mode: ExecutionMode,
    config: &Config,
    generated_at: String,
) -> systui_report::FleetReview {
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

    let mut hosts = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(host_report) => hosts.push(host_report),
            // A panicking review task should not lose the rest of the fleet.
            Err(err) => tracing::error!(%err, "fleet review task failed to join"),
        }
    }

    systui_report::FleetReview {
        generated_at,
        hosts,
    }
}

/// Review a single host over SSH, mapping any failure (connection, auth, timeout)
/// to a failed report so it still shows up as an error row.
async fn review_one_host(
    host: systui_core::FleetHost,
    mode: ExecutionMode,
    config: &Config,
    generated_at: String,
) -> systui_report::FleetHostReport {
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

    let report = match tokio::time::timeout(FLEET_HOST_TIMEOUT, gather).await {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err(format!("timed out after {}s", FLEET_HOST_TIMEOUT.as_secs())),
    };

    systui_report::FleetHostReport {
        id: host.id,
        tags: host.tags,
        favorite: host.favorite,
        report,
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
            } => format!(
                "{health:>3}/100  {}",
                systui_report::findings_summary(finding_counts)
            ),
            FleetOutcome::Failed { error } => format!("  ERR  {error}"),
        };
        println!("{marker} {:<16} {:<16} {status}", host.id, tags);
    }
}

/// Print the hosts whose ports/services match a global search term.
fn print_fleet_search(review: &systui_report::FleetReview, term: &str) {
    let matches = review.search(term);
    if matches.is_empty() {
        println!("No hosts match `{term}`.");
        return;
    }
    println!("Hosts matching `{term}`:\n");
    for host in &matches {
        let mut parts = Vec::new();
        if !host.ports.is_empty() {
            let ports: Vec<String> = host.ports.iter().map(u16::to_string).collect();
            parts.push(format!("ports: {}", ports.join(", ")));
        }
        if !host.services.is_empty() {
            parts.push(format!("services: {}", host.services.join(", ")));
        }
        println!("  {:<16} {}", host.id, parts.join("  ·  "));
    }
}

/// Print a side-by-side comparison of two hosts and their drift deltas.
fn print_fleet_comparison(cmp: &systui_report::HostComparison) {
    println!("Comparing {} vs {}:\n", cmp.left_id, cmp.right_id);
    let row = |label: &str, left: &str, right: &str| {
        println!("  {label:<14} {left:<28} {right}");
    };
    row("", &cmp.left_id, &cmp.right_id);
    row("os", &cmp.left.os, &cmp.right.os);
    row("kernel", &cmp.left.kernel, &cmp.right.kernel);
    row(
        "health",
        &format!("{}/100", cmp.left.health),
        &format!("{}/100", cmp.right.health),
    );
    row(
        "open ports",
        &cmp.left.open_ports.len().to_string(),
        &cmp.right.open_ports.len().to_string(),
    );
    row(
        "services",
        &cmp.left.services.len().to_string(),
        &cmp.right.services.len().to_string(),
    );

    if !cmp.has_drift() {
        println!("\nNo port or service drift.");
        return;
    }
    println!("\nDrift:");
    print_drift_line(
        &format!("only on {}", cmp.left_id),
        &ports_and_services(cmp, true),
    );
    print_drift_line(
        &format!("only on {}", cmp.right_id),
        &ports_and_services(cmp, false),
    );
}

/// Format the ports + services unique to one side of a comparison.
fn ports_and_services(cmp: &systui_report::HostComparison, left: bool) -> Vec<String> {
    let (ports, services) = if left {
        (cmp.ports_only_left(), cmp.services_only_left())
    } else {
        (cmp.ports_only_right(), cmp.services_only_right())
    };
    let mut out: Vec<String> = ports.iter().map(|p| format!("port {p}")).collect();
    out.extend(services);
    out
}

fn print_drift_line(label: &str, items: &[String]) {
    let value = if items.is_empty() {
        "—".to_owned()
    } else {
        items.join(", ")
    };
    println!("  {label:<16} {value}");
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
