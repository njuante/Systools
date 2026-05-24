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

/// CSS class for a severity badge in the HTML report.
pub fn severity_class(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "sev-critical",
        Severity::High => "sev-high",
        Severity::Medium => "sev-medium",
        Severity::Low => "sev-low",
        Severity::Info => "sev-info",
    }
}

/// Escape text for safe embedding in HTML. SysTUI renders strings collected from
/// untrusted remote hosts, so every host-derived value passes through this.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_metacharacters() {
        assert_eq!(
            escape_html("<script>alert(\"x\" & 'y')</script>"),
            "&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;"
        );
        assert_eq!(escape_html("plain text"), "plain text");
    }

    #[test]
    fn human_kb_scales_units() {
        assert_eq!(human_kb(512), "512 KiB");
        assert_eq!(human_kb(2048), "2.0 MiB");
    }
}
