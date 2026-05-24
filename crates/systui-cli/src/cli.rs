//! Command-line argument definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Fast, agentless TUI for Linux server administration.
#[derive(Debug, Parser)]
#[command(name = "systui", version, about)]
pub struct Cli {
    /// Force read-only mode: never modify the host, regardless of other settings.
    #[arg(long, global = true)]
    pub read_only: bool,

    /// Use an alternate configuration file instead of the default location.
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level operating modes (`Product.md` §1).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect the local machine. This is the default when no subcommand is given.
    Local,

    /// Connect to a remote host over SSH.
    Ssh {
        /// Target as `user@host`, or a host id from the inventory.
        target: String,
    },

    /// Operate on a fleet of hosts from the inventory (inspection only).
    Fleet {
        /// Restrict to hosts carrying this tag. Repeat for "any of" (OR).
        #[arg(long)]
        tag: Vec<String>,
        /// Restrict to hosts flagged as favorites.
        #[arg(long)]
        favorites: bool,
        /// Search the fleet for a service or port; list matching hosts and exit.
        #[arg(long, value_name = "TERM")]
        search: Option<String>,
        /// Compare two inventory hosts side by side: `--compare <A> <B>`.
        #[arg(long, num_args = 2, value_names = ["A", "B"])]
        compare: Vec<String>,
        /// Render a fleet report instead of the overview: `markdown`, `json` or `html`.
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
        /// Write the fleet report to a file instead of stdout.
        #[arg(long, short = 'o', value_name = "FILE")]
        output: Option<PathBuf>,
    },

    /// Generate a report of a host's state (local, or remote with `--host`).
    Report {
        /// Host to report on: an inventory id or `user@host`. Omit for local.
        #[arg(long)]
        host: Option<String>,
        /// Output format: `markdown`, `json` or `html`.
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Produce a security-focused report (affects markdown/html).
        #[arg(long)]
        security: bool,
        /// Write the report to a file instead of stdout.
        #[arg(long, short = 'o', value_name = "FILE")]
        output: Option<PathBuf>,
        /// Add a review note to the report (may be repeated).
        #[arg(long, value_name = "TEXT")]
        note: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_parses() {
        let cli = Cli::try_parse_from(["systui"]).unwrap();
        assert!(cli.command.is_none());
        assert!(!cli.read_only);
    }

    #[test]
    fn read_only_flag_is_global() {
        let cli = Cli::try_parse_from(["systui", "ssh", "admin@prod-01", "--read-only"]).unwrap();
        assert!(cli.read_only);
        match cli.command {
            Some(Command::Ssh { target }) => assert_eq!(target, "admin@prod-01"),
            other => panic!("expected ssh command, got {other:?}"),
        }
    }

    #[test]
    fn report_format_defaults_to_markdown() {
        let cli = Cli::try_parse_from(["systui", "report", "--host", "db-01"]).unwrap();
        match cli.command {
            Some(Command::Report {
                host,
                format,
                security,
                ..
            }) => {
                assert_eq!(host.as_deref(), Some("db-01"));
                assert_eq!(format, "markdown");
                assert!(!security);
            }
            other => panic!("expected report command, got {other:?}"),
        }
    }
}
