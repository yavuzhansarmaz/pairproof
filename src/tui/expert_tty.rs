//! Expert TTY plumbing: terminal guard, preflight kick-off, and the
//! bounded async step helper shared by the interactive loop.

use std::future::Future;

use crate::orchestration::{Clock, SystemClock};
use crate::tui::{app, expert, expert_runtime, help};

/// Restores the terminal on drop for every exit or panic path.
pub struct TtyGuard;

impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        #[rustfmt::skip]
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen, crossterm::event::DisableBracketedPaste, crossterm::event::DisableMouseCapture);
    }
}

/// Runs preflight once and feeds the gated outcome into the app.
/// Checks the port `build_expert_runtime` will select (first free in
/// `SANDBOX_PORT_RANGE`); when none is free, the range start is reported
/// busy so preflight still fails on the port check. Also refreshes
/// the API-key status line shown on the same Expert Setup screen.
pub fn run_preflight_check(app: &mut app::ExpertApp) {
    let port =
        expert_runtime::select_sandbox_port().unwrap_or(expert_runtime::SANDBOX_PORT_RANGE.start);
    let report = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .map(|runtime| runtime.block_on(crate::sandbox::preflight::gather_preflight(port)));
    match report {
        Some(report) => {
            // Single formatting source: detail and guidance are kept.
            let view = expert::preflight_view(&report);
            app.set_preflight_passed(view.can_start);
            app.set_preflight_lines(view.lines);
        }
        None => {
            app.set_preflight_passed(false);
            app.set_preflight_lines(vec!["Environment checks could not be completed.".into()]);
        }
    }
    let (status, ready) = api_key_status_line();
    app.set_api_key_status(status);
    app.set_api_key_ready(ready);
}

fn api_key_status_line() -> (String, bool) {
    if crate::tui::config::env_override_active() {
        return ("Meta API key: configured (environment)".to_string(), true);
    }
    if let Some(path) = crate::tui::config::default_api_key_path() {
        if let Ok(Some(key)) = crate::tui::config::stored_api_key(&path) {
            return (
                format!(
                    "Meta API key: configured ({})",
                    crate::tui::config::mask_api_key(&key)
                ),
                true,
            );
        }
    }
    ("Meta API key: not configured".to_string(), false)
}

/// Path of the persisted disclosure acknowledgement marker, next to
/// the stored API key (`~/.config/pairproof/disclosure_acknowledged`).
pub fn disclosure_ack_path() -> Option<std::path::PathBuf> {
    crate::tui::config::default_api_key_path()?
        .parent()
        .map(|dir| dir.join("disclosure_acknowledged"))
}

/// Returns true when a disclosure acknowledgement was persisted.
pub fn load_disclosure_ack(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.trim() == "acknowledged")
        .unwrap_or(false)
}

/// Persists a disclosure acknowledgement so it is not shown again.
pub fn save_disclosure_ack(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, "acknowledged\n")
}

/// Saves a key typed on Expert Setup to the stored key file and
/// updates the app status. The raw input is cleared on success and
/// never stored on the app itself. An empty input (0 characters)
/// closes the entry without saving, so a key change can be backed
/// out with Enter (`b` types into the key, so it cannot go back).
pub fn save_entered_api_key(app: &mut app::ExpertApp, input: &mut String) {
    if input.trim().is_empty() {
        input.clear();
        app.set_api_key_entry(false);
        app.set_status_message("Meta API key unchanged.");
        return;
    }
    let Some(path) = crate::tui::config::default_api_key_path() else {
        set_error(app, "API key save failed", "no API key path");
        return;
    };
    match crate::tui::config::save_api_key(&path, input) {
        Ok(()) => {
            app.set_api_key_entry(false);
            app.set_api_key_ready(true);
            app.set_api_key_status(format!(
                "Meta API key: configured ({})",
                crate::tui::config::mask_api_key(input)
            ));
            input.clear();
            app.set_status_message("Meta API key saved.");
        }
        Err(err) => set_error(app, "API key save failed", &err.to_string()),
    }
}

/// Health-checks a candidate Meta API key with one bounded
/// `list_models` call; nothing is saved when this fails.
pub async fn check_api_key(key: &str) -> Result<(), String> {
    let client = crate::llm::MetaClient::new(crate::llm::DEFAULT_BASE_URL, key.trim());
    match tokio::time::timeout(std::time::Duration::from_secs(15), client.list_models()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("Meta API check timed out after 15 seconds".to_string()),
    }
}

/// Saves the entered key only after its health check passed; a
/// failed check keeps the entry open and the typed key in place.
pub fn save_checked_api_key(
    app: &mut app::ExpertApp,
    input: &mut String,
    checked: Result<(), String>,
) {
    match checked {
        Ok(()) => save_entered_api_key(app, input),
        Err(err) => set_error(app, "API key check failed; key not saved", &err),
    }
}

/// Loads the persisted disclosure acknowledgement, if any.
pub fn load_disclosure_ack_default() -> bool {
    disclosure_ack_path()
        .map(|path| load_disclosure_ack(&path))
        .unwrap_or(false)
}

/// Persists the disclosure acknowledgement at the default path.
pub fn save_disclosure_ack_default() {
    if let Some(path) = disclosure_ack_path() {
        let _ = save_disclosure_ack(&path);
    }
}

pub fn now_ms() -> u64 {
    SystemClock.now_ms()
}

/// Sets a sanitized expert error status with a next step.
#[rustfmt::skip]
pub fn set_error(app: &mut app::ExpertApp, context: &str, detail: &str) { app.set_status_message(help::expert_error_message(context, detail)); }

/// Runs one bounded async step on a lazily built current-thread runtime.
#[rustfmt::skip]
pub fn block_on<T>(slot: &mut Option<tokio::runtime::Runtime>, future: impl Future<Output = T>) -> Option<T> { if slot.is_none() { *slot = tokio::runtime::Builder::new_current_thread().enable_all().build().ok(); } slot.as_ref().map(|runtime| runtime.block_on(future)) }
