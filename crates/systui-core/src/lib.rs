//! SysTUI core: shared models, typed errors, configuration and the contracts
//! (transport / collector / action) that every other crate builds on.
//!
//! Nothing here talks to the OS directly. Collectors and actions are defined as
//! traits parameterised over a [`Transport`], so the same logic runs locally,
//! over SSH, or against a mock in tests.

pub mod action;
pub mod collector;
pub mod command;
pub mod config;
pub mod error;
pub mod mode;
pub mod model;
pub mod transport;

pub use action::{Action, ActionOutcome, ActionPreview};
pub use collector::{Collector, ModuleId};
pub use command::{CommandOutput, CommandSpec};
pub use config::Config;
pub use error::{CoreError, Result};
pub use mode::{ExecutionMode, RiskLevel};
pub use model::{HostId, Severity};
pub use transport::{DirEntry, FileType, Transport};
