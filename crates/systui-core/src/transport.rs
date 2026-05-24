//! The fundamental transport contract.
//!
//! Every collector and action talks to a [`Transport`], never to the OS
//! directly, so the same code runs locally, over SSH or against a mock
//! (`Product.md` §2). Implementations live in `systui-transport`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::command::{CommandOutput, CommandSpec};
use crate::error::Result;

/// Abstracts command execution and filesystem reads on a target host.
#[async_trait]
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// A short human-readable label, e.g. `"local"` or `"ssh://admin@prod-01"`.
    fn label(&self) -> &str;

    /// Run a command and capture its output.
    async fn run(&self, command: &CommandSpec) -> Result<CommandOutput>;

    /// Read the full contents of a file.
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;

    /// Whether a path exists and is readable.
    async fn file_exists(&self, path: &str) -> Result<bool>;

    /// List the entries of a directory.
    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>>;
}

/// A single entry returned by [`Transport::list_dir`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// File name, without the parent path.
    pub name: String,
    /// What kind of filesystem object this is.
    pub file_type: FileType,
}

/// The kind of a filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    File,
    Dir,
    Symlink,
    Other,
}
