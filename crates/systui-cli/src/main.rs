//! SysTUI command-line entry point.
//!
//! Wires argument parsing, logging and configuration loading, then resolves the
//! execution mode and dispatches to a mode handler. The handlers are stubs until
//! the TUI shell (S0.6) and the remote/report/fleet phases land.

mod cli;

use anyhow::Context;
use clap::Parser;
use systui_core::{Config, ExecutionMode};
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    init_tracing();

    let config = load_config(&args).context("failed to load configuration")?;
    let mode = resolve_mode(&args);
    tracing::info!(%mode, "starting systui");

    dispatch(args.command.unwrap_or(Command::Local), mode, &config);
    Ok(())
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

fn dispatch(command: Command, mode: ExecutionMode, _config: &Config) {
    match command {
        Command::Local => {
            println!("systui: local mode ({mode}) — interactive TUI arrives in S0.6");
        }
        Command::Ssh { target } => {
            println!("systui: ssh mode -> {target} ({mode}) — implemented in phase 5");
        }
        Command::Fleet { tag } => {
            let scope = tag.as_deref().unwrap_or("all hosts");
            println!("systui: fleet mode [{scope}] ({mode}) — implemented in phase 8");
        }
        Command::Report { host, format } => {
            let host = host.as_deref().unwrap_or("local");
            println!("systui: report for {host} as {format} ({mode}) — implemented in phase 6");
        }
    }
}
