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

    /// Connect to a remote host over SSH (implemented in phase 5).
    Ssh {
        /// Target as `user@host`, or a host id from the inventory.
        target: String,
    },

    /// Operate on a fleet of hosts (implemented in phase 8).
    Fleet {
        /// Restrict to hosts carrying this tag.
        #[arg(long)]
        tag: Option<String>,
    },

    /// Generate a report (implemented in phase 6).
    Report {
        /// Host id to report on.
        #[arg(long)]
        host: Option<String>,
        /// Output format.
        #[arg(long, default_value = "markdown")]
        format: String,
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
            Some(Command::Report { host, format }) => {
                assert_eq!(host.as_deref(), Some("db-01"));
                assert_eq!(format, "markdown");
            }
            other => panic!("expected report command, got {other:?}"),
        }
    }
}
