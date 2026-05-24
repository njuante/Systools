//! SysTUI report generation. A [`Report`] (assembled by [`gather_report`] over any
//! `Transport`) is rendered to JSON (the full structured model), Markdown or HTML.
//! Gathering does the I/O; the renderers are pure functions of [`Report`].

pub mod gather;
pub mod html;
pub mod json;
pub mod markdown;
pub mod model;
pub mod util;

pub use gather::gather_report;
pub use html::to_html;
pub use json::to_json;
pub use markdown::to_markdown;
pub use model::{Report, ReportMeta, ReportScope};
