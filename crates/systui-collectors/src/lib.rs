//! SysTUI collectors: read-only readers that turn command output and files into
//! typed domain models (system, processes, services, logs, network, ...).
//!
//! [`SystemCollector`] produces the [`SystemSnapshot`] shown on the dashboard.
//! Process, service, log and network collectors arrive in later v0.1 sessions.

pub mod health;
pub mod host_report;
pub mod logs;
pub mod process;
pub mod service;
pub mod system;

pub use health::{Check, HealthReport, evaluate_health};
pub use host_report::{HostReport, collect_host_report};
pub use logs::{LogEntry, LogsCollector};
pub use process::{Process, ProcessCollector};
pub use service::{FailedUnitsCollector, ServiceCollector, ServiceUnit, UnitDetail, unit_detail};
pub use system::{
    CpuUsage, Disk, LoadAverage, LoggedUser, Memory, Swap, SystemCollector, SystemSnapshot,
};
