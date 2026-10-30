//! Expert screens: setup, question selection, live monitoring, final score.

use crate::sandbox::preflight::{CheckStatus, PreflightReport};
use crate::sync::classify_ssh_failure;
use crate::tui::shared::sanitize_display;

/// Render-ready preflight lines plus start gating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightView {
    pub lines: Vec<String>,
    pub can_start: bool,
    pub failures: usize,
    pub warnings: usize,
}

fn status_prefix(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "[PASS]",
        CheckStatus::Warn => "[WARN]",
        CheckStatus::Fail => "[FAIL]",
    }
}

/// Builds the preflight view from a typed report without recomputing.
pub fn preflight_view(report: &PreflightReport) -> PreflightView {
    let lines = report
        .checks
        .iter()
        .map(|check| {
            let mut line = format!(
                "{} {:?}: {}",
                status_prefix(check.status),
                check.kind,
                sanitize_display(&check.detail)
            );
            if let Some(guidance) = &check.guidance {
                line.push_str(" \u{2014} ");
                line.push_str(&sanitize_display(guidance));
            }
            line
        })
        .collect();
    PreflightView {
        lines,
        can_start: report.can_start(),
        failures: report.failures(),
        warnings: report.warnings(),
    }
}

/// Returns whether the report allows starting.
pub fn can_proceed(report: &PreflightReport) -> bool {
    report.can_start()
}

/// Returns expert-only SSH guidance for a stderr message.
pub fn expert_ssh_hint(stderr: &str) -> String {
    classify_ssh_failure(stderr).expert_guidance
}
