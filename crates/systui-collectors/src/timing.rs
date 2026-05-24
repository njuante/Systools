//! Lightweight refresh timing for the optimization phase (v0.8.3).
//!
//! Each collector and the overall refresh are wrapped in [`timed`], which
//! records how long the work took as a `tracing` event under the
//! [`PERF_TARGET`] target. The instrumentation is dormant unless that target is
//! enabled, so it costs nothing in normal runs. Enable it with, e.g.:
//!
//! ```sh
//! SYSTUI_LOG=systui::perf=info systui            # local
//! SYSTUI_LOG=systui::perf=info systui ssh prod-01 # over SSH
//! ```

use std::future::Future;
use std::time::Instant;

/// `tracing` target carrying refresh/collector timing events.
pub const PERF_TARGET: &str = "systui::perf";

/// Await `fut`, emitting how long it took as a `systui::perf` event labelled
/// `label`. Returns the future's output unchanged.
pub async fn timed<F: Future>(label: &'static str, fut: F) -> F::Output {
    let start = Instant::now();
    let out = fut.await;
    tracing::info!(
        target: PERF_TARGET,
        collector = label,
        elapsed_ms = start.elapsed().as_secs_f64() * 1000.0,
    );
    out
}
