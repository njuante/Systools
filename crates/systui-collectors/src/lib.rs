//! SysTUI collectors: read-only readers that turn command output and files into
//! typed domain models (system, processes, services, logs, network, ...).
//!
//! [`SystemCollector`] produces the [`SystemSnapshot`] shown on the dashboard.
//! Process, service, log and network collectors arrive in later v0.1 sessions.

pub mod capabilities;
pub mod connectivity;
pub mod cron;
pub mod database;
pub mod docker;
pub mod exposure;
pub mod firewall;
pub mod health;
pub mod host_report;
pub mod logs;
pub mod network;
pub mod packages;
pub mod process;
pub mod service;
pub mod system;
pub mod timing;

pub use capabilities::{HostCapabilities, probe_capabilities};
pub use connectivity::{DnsLookup, PingResult, TcpProbe, dns_lookup, ping, tcp_connect};
pub use cron::{
    CronEntry, CronSchedule, CronSource, SystemdTimer, collect_cron_entries, collect_timers,
    parse_anacrontab, parse_crontab, parse_schedule,
};
pub use database::{
    DatabaseCollector, DatabaseCredentialKind, DatabaseCredentialSource, DatabaseEngine,
    DatabaseInstance, DatabaseOperational, DatabaseService, DatabaseSnapshot,
    detect_database_instances,
};
pub use docker::{
    ComposeProject, Container, ContainerHealth, ContainerStats, DockerCollector, ImageHygiene,
    InspectSummary, Mount, PublishedPort, compose_projects, container_logs, container_stats,
    image_hygiene, inspect_container,
};
pub use exposure::{BindScope, ExposureEntry, exposure_map};
pub use firewall::{FirewallCollector, FirewallSnapshot};
pub use health::{Check, HealthReport, evaluate_health};
pub use host_report::{HostReport, collect_host_report};
pub use logs::{LogEntry, LogQuery, LogsCollector};
pub use network::{
    AddrFamily, Connection, DnsConfig, InterfaceAddr, Listener, NetInterface, NetStatics,
    NetworkCollector, NetworkSnapshot, ProcessRef, Protocol, Route, correlate_units,
};
pub use packages::{PackageUpdates, PackagesCollector};
pub use process::{
    Process, ProcessCollector, ProcessDetail, TreeRow, build_process_tree, process_detail,
};
pub use service::{
    FailedUnitsCollector, ServiceCollector, ServiceUnit, UnitDetail, UnitFile, UnitFilesCollector,
    unit_dependencies, unit_detail,
};
pub use system::{
    CpuUsage, Disk, HostStatics, LoadAverage, LoggedUser, Memory, Swap, SystemCollector,
    SystemSnapshot,
};
