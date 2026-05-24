//! Shared formatting helpers for the report renderers.

use systui_core::Severity;

/// Upper-case severity label, e.g. `CRITICAL`.
pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "CRITICAL",
        Severity::High => "HIGH",
        Severity::Medium => "MEDIUM",
        Severity::Low => "LOW",
        Severity::Info => "INFO",
    }
}

/// Format seconds as `Xd Yh Zm`.
pub fn human_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    format!("{days}d {hours}h {mins}m")
}

/// Format a kB amount (1024-based) as KiB/MiB/GiB/TiB.
pub fn human_kb(kb: u64) -> String {
    const MIB: f64 = 1024.0;
    const GIB: f64 = 1024.0 * 1024.0;
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let kb_f = kb as f64;
    if kb_f >= TIB {
        format!("{:.1} TiB", kb_f / TIB)
    } else if kb_f >= GIB {
        format!("{:.1} GiB", kb_f / GIB)
    } else if kb_f >= MIB {
        format!("{:.1} MiB", kb_f / MIB)
    } else {
        format!("{kb} KiB")
    }
}

/// The owner of a listener for display: `process (unit)`, `process`, or `—`.
pub fn listener_owner(listener: &systui_collectors::Listener) -> String {
    match (&listener.process, &listener.unit) {
        (Some(p), Some(unit)) => format!("{} ({unit})", p.name),
        (Some(p), None) => p.name.clone(),
        (None, _) => "—".to_owned(),
    }
}

/// Unique, order-preserving recommendations from a set of findings.
pub fn unique_recommendations(findings: &[systui_core::Finding]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    findings
        .iter()
        .filter(|f| !f.recommendation.is_empty())
        .filter(|f| seen.insert(f.recommendation.clone()))
        .map(|f| f.recommendation.clone())
        .collect()
}
