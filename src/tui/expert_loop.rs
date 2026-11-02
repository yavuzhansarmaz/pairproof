//! Expert TTY loop state: frame building, key/mouse routing, and
//! the bounded per-tick runtime step.

use crate::tui::expert_keys::handle_key;
use crate::tui::expert_runtime::{ExpertRuntime, PollOutcome};
use crate::tui::expert_tty::{block_on, now_ms};
use crate::tui::setup::ExpertSetupPastes;
use crate::tui::{app, help, monitor, render, setup};

/// Mutable state owned by the expert TTY loop.
pub(crate) struct ExpertLoopState {
    pub(crate) runtime: Option<ExpertRuntime>,
    pub(crate) tokio_rt: Option<tokio::runtime::Runtime>,
    pub(crate) pastes: ExpertSetupPastes,
    pub(crate) api_key_input: String,
    pub(crate) setup_jd_height: u16,
    pub(crate) setup_notes_height: u16,
    pub(crate) setup_drag: Option<setup::ExpertSetupFocus>,
    pub(crate) show_help: bool,
    pub(crate) show_disclosure: bool,
}

impl Default for ExpertLoopState {
    fn default() -> Self {
        Self {
            runtime: None,
            tokio_rt: None,
            pastes: ExpertSetupPastes::new(),
            api_key_input: String::new(),
            setup_jd_height: 5,
            setup_notes_height: 2,
            setup_drag: None,
            show_help: false,
            show_disclosure: false,
        }
    }
}

impl ExpertLoopState {
    /// Builds the lines for one frame: app lines, live runtime lines,
    /// the masked API-key entry, then help/disclosure replacement.
    pub(crate) fn build_lines(&mut self, app: &app::ExpertApp) -> Vec<String> {
        let mut lines = app::screen_lines(app);
        if let Some(active) = self.runtime.as_mut() {
            let now = now_ms();
            match app.screen() {
                app::AppScreen::ConnectionWait => {
                    if let Ok(current) = active.current_string(now) {
                        lines.push(format!("Connection: {}", current.encode()));
                        lines.push(help::countdown_line(active.countdown_secs(now)));
                    }
                    lines.push("Waiting for candidate SSH login (no manual lock).".to_string());
                }
                app::AppScreen::Monitor => {
                    lines.extend(monitor::render_monitor_lines(&active.monitor_view(now)));
                    lines.extend(active.recovery_lines());
                    if let Some(fresh) = active.reissue_string() {
                        lines.push(format!("reissue: {}", fresh.encode()));
                        lines.push(help::countdown_line(active.countdown_secs(now)));
                    }
                    if active.is_disconnected() {
                        lines.push("press R to reissue a fresh connection string.".to_string());
                    }
                    lines.push("press e to end the session.".to_string());
                }
                _ => {}
            }
        }
        if app.api_key_entry && app.screen() == app::AppScreen::Preflight {
            let masked = format!(
                "  Key: {} ({} characters)",
                "\u{2022}".repeat(self.api_key_input.chars().count()),
                self.api_key_input.chars().count()
            );
            let at = lines.len().saturating_sub(1);
            lines.insert(at, masked);
        }
        if self.show_disclosure {
            lines = help::disclosure_lines();
        } else if self.show_help {
            lines = help::expert_help_lines(app.screen());
        }
        lines
    }

    fn dispatch_key(&mut self, app: &mut app::ExpertApp, code: crossterm::event::KeyCode) {
        handle_key(
            app,
            &mut self.runtime,
            &mut self.tokio_rt,
            &mut self.pastes,
            &mut self.api_key_input,
            code,
        );
    }

    /// Routes one key event through API-key entry and help overlays.
    pub(crate) fn handle_key_event(
        &mut self,
        app: &mut app::ExpertApp,
        key: crossterm::event::KeyEvent,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        if app.api_key_entry {
            self.dispatch_key(app, key.code);
            return;
        }
        if self.show_help
            && !self.show_disclosure
            && help::help_overlay_dispatches(key.code, app.screen())
        {
            self.show_help = false;
            self.dispatch_key(app, key.code);
            return;
        }
        match if self.show_disclosure {
            help::expert_disclosure_action(key)
        } else {
            help::expert_event_action(key, self.show_help)
        } {
            help::ExpertKeyAction::ToggleHelp => {
                if self.show_disclosure {
                    self.show_disclosure = false;
                    self.show_help = false;
                } else {
                    self.show_help = !self.show_help;
                }
            }
            help::ExpertKeyAction::ShowDisclosure => self.show_disclosure = !self.show_disclosure,
            help::ExpertKeyAction::ReturnToHelp => self.show_disclosure = false,
            help::ExpertKeyAction::Ignore => {}
            help::ExpertKeyAction::Quit => {
                let _ = app.handle(app::AppEvent::Quit);
            }
            help::ExpertKeyAction::Dispatch => self.dispatch_key(app, key.code),
        }
    }

    /// Setup-screen mouse: focus, drag-resize, and scroll the two boxes.
    pub(crate) fn handle_setup_mouse(
        &mut self,
        terminal_size: ratatui::layout::Rect,
        mouse: crossterm::event::MouseEvent,
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let (jd_box, notes_box) =
            render::setup_box_areas(terminal_size, self.setup_jd_height, self.setup_notes_height);
        let in_box = |rect: ratatui::layout::Rect| {
            mouse.column >= rect.x
                && mouse.column < rect.x + rect.width
                && mouse.row >= rect.y
                && mouse.row < rect.y + rect.height
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if in_box(jd_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::JobDescription);
                    if mouse.row == jd_box.y + jd_box.height - 1 {
                        self.setup_drag = Some(setup::ExpertSetupFocus::JobDescription);
                    }
                } else if in_box(notes_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                    if mouse.row == notes_box.y + notes_box.height - 1 {
                        self.setup_drag = Some(setup::ExpertSetupFocus::AdditionalNotes);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.setup_drag {
                Some(setup::ExpertSetupFocus::JobDescription) => {
                    self.setup_jd_height = mouse.row.saturating_sub(jd_box.y + 1).clamp(5, 20)
                }
                Some(setup::ExpertSetupFocus::AdditionalNotes) => {
                    self.setup_notes_height = mouse.row.saturating_sub(notes_box.y + 1).clamp(2, 12)
                }
                None => {}
            },
            MouseEventKind::Up(MouseButton::Left) => self.setup_drag = None,
            MouseEventKind::ScrollDown => {
                if in_box(jd_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::JobDescription);
                } else if in_box(notes_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                }
                if in_box(jd_box) || in_box(notes_box) {
                    self.pastes.scroll_focused(1);
                }
            }
            MouseEventKind::ScrollUp => {
                if in_box(jd_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::JobDescription);
                } else if in_box(notes_box) {
                    self.pastes
                        .set_focus(setup::ExpertSetupFocus::AdditionalNotes);
                }
                if in_box(jd_box) || in_box(notes_box) {
                    self.pastes.scroll_focused(-1);
                }
            }
            _ => {}
        }
    }
}

/// Bounded per-tick runtime step: poll for the candidate hello on
/// the connection screen, drain intents once the session is live.
pub(crate) fn tick_runtime(
    app: &mut app::ExpertApp,
    runtime: &mut Option<ExpertRuntime>,
    tokio_rt: &mut Option<tokio::runtime::Runtime>,
) {
    use app::{AppEvent, AppScreen};
    let Some(active) = runtime.as_mut() else {
        return;
    };
    match app.screen() {
        AppScreen::ConnectionWait => {
            let _ = block_on(tokio_rt, active.refresh_pre_lock_secret(now_ms()));
            let _ = block_on(tokio_rt, active.poll_recovery(now_ms()));
            match block_on(tokio_rt, active.poll_candidate(now_ms())) {
                Some(PollOutcome::Connected) => {
                    app.set_connected(true);
                    let _ = app.handle(AppEvent::ConnectionLocked);
                }
                Some(PollOutcome::VersionMismatch) => {
                    if let Some((c, e)) = active.version_mismatch() {
                        app.set_status_message(format!("Version mismatch: candidate {c}, you {e}. Candidate must update/restart PairProof; session not started."));
                    }
                }
                _ => {}
            }
        }
        AppScreen::SelectMedium | AppScreen::SelectHard | AppScreen::Monitor => {
            let _ = block_on(tokio_rt, active.refresh_pre_lock_secret(now_ms()));
            let outcome = block_on(tokio_rt, active.poll_candidate(now_ms()));
            let _ = block_on(tokio_rt, active.poll_recovery(now_ms()));
            let _ = block_on(tokio_rt, active.drain_intents(now_ms()));
            let _ = block_on(tokio_rt, active.check_heartbeat(now_ms()));
            if active.is_disconnected() {
                let _ = app.handle(AppEvent::AccidentalDisconnect);
            }
            if outcome == Some(PollOutcome::Connected) && app.is_disconnected() {
                let _ = app.handle(AppEvent::CandidateReconnected);
                app.clear_status_message();
            }
        }
        _ => {}
    }
}
