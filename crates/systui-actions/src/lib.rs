//! SysTUI action engine: the single path every mutation goes through —
//! permission check, read-only check, risk classification, preview, confirmation,
//! backup, execute, verify and audit. The UI requests actions; this crate decides.
//!
//! v0.2 introduces the concrete actions (service operations); the engine that
//! drives them through the full safety pipeline arrives in session S2.5.

pub mod docker;
pub mod engine;
pub mod process;
pub mod service;

pub use docker::{DockerAction, DockerOp};
pub use engine::{ActionDecision, ActionEngine};
pub use process::{Signal, SignalAction};
pub use service::{ServiceAction, ServiceOp};
