//! Expert screens: setup, question selection, live monitoring, final score.

use crate::sandbox::preflight::{CheckKind, CheckStatus, PreflightReport};
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

/// Human-readable label for one check; never the Debug enum name.
fn kind_label(kind: CheckKind) -> &'static str {
    match kind {
        CheckKind::DockerInstalled => "Docker",
        CheckKind::DockerDaemon => "Docker daemon",
        CheckKind::SandboxImage => "Sandbox image",
        CheckKind::PortAvailable => "Port",
        CheckKind::Network => "Connection",
        CheckKind::MutagenInstalled => "Mutagen",
    }
}

/// Word-wraps `text` under `first_prefix`; continuation lines align
/// under the text. Width stays narrow so lines fit centred panels.
fn wrapped(prefix: &str, indent: &str, text: &str) -> Vec<String> {
    const WIDTH: usize = 84;
    let mut lines = Vec::new();
    let mut current = prefix.to_string();
    for word in text.split_whitespace() {
        if current.trim().is_empty() || current.chars().count() + 1 + word.chars().count() <= WIDTH
        {
            if !current.ends_with(' ') && !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            lines.push(current);
            current = format!("{indent}{word}");
        }
    }
    lines.push(current);
    lines
}

/// Builds the preflight view from a typed report without recomputing.
/// Each check is one short headed line; guidance, when present, goes
/// on its own indented `->` line so warnings stay readable.
pub fn preflight_view(report: &PreflightReport) -> PreflightView {
    let mut lines = Vec::new();
    for check in &report.checks {
        let prefix = format!(
            "{} {}: ",
            status_prefix(check.status),
            kind_label(check.kind)
        );
        let indent = " ".repeat(prefix.chars().count());
        lines.extend(wrapped(&prefix, &indent, &sanitize_display(&check.detail)));
        if let Some(guidance) = &check.guidance {
            lines.extend(wrapped(
                "       -> ",
                "          ",
                &sanitize_display(guidance),
            ));
        }
    }
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
