//! SysTUI collectors: read-only readers that turn command output and files into
//! typed domain models (system, processes, services, logs, network, ...).
//!
//! [`SystemCollector`] produces the [`SystemSnapshot`] shown on the dashboard.
//! Process, service, log and network collectors arrive in later v0.1 sessions.

pub mod process;
pub mod system;

pub use process::{Process, ProcessCollector};
pub use system::{
    CpuUsage, Disk, LoadAverage, LoggedUser, Memory, Swap, SystemCollector, SystemSnapshot,
};
