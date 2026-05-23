//! SysTUI collectors: read-only readers that turn command output and files into
//! typed domain models (system, processes, services, logs, network, ...).
//!
//! [`HostInfoCollector`] is the foundation's end-to-end example (phase 0, S0.7).
//! The full system/process/service collectors arrive in v0.1+ (phase 1).

pub mod host;

pub use host::{HostInfo, HostInfoCollector};
