//! SysTUI report generation. A [`Report`] (assembled by [`gather_report`] over any
//! `Transport`) is rendered to JSON (the full structured model), Markdown or HTML.
//! Gathering does the I/O; the renderers are pure functions of [`Report`].

pub mod collect;
pub mod fleet;
pub mod fleet_report;
pub mod gather;
pub mod html;
pub mod json;
pub mod markdown;
pub mod model;
pub mod util;

pub use fleet::{
    FleetHostReport, FleetHostSummary, FleetOutcome, FleetOverview, FleetReview, HostComparison,
    HostFacts, HostMatches, findings_summary,
};
pub use fleet_report::{
    FleetReport, FleetReportHost, fleet_to_html, fleet_to_json, fleet_to_markdown,
};
pub use gather::gather_report;
pub use html::to_html;
pub use json::to_json;
pub use markdown::to_markdown;
pub use model::{Report, ReportMeta, ReportScope};
