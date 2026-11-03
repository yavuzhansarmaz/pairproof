//! Expert frame planning, event routing, and exit cleanup for the
//! TTY shell in `tui::mod`: decisions the shell used to inline.

use crate::tui::expert_keys::{handle_key, handle_paste};
use crate::tui::expert_loop::ExpertLoopState;
use crate::tui::expert_poll::tick_runtime;
use crate::tui::expert_tty::{
    block_on, now_ms, run_preflight_check, save_checked_api_key, set_error,
};
use crate::tui::{app, render, setup, viewport};

/// Which renderer one frame needs, decided without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameKind {
    Loading,
    Setup,
    Screen,
}

/// One frame's visible lines plus how the TTY shell should draw them.
/// Built by [`ExpertLoopState::frame_plan`]; the shell only draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FramePlan {
    pub(crate) visible: Vec<String>,
    pub(crate) kind: FrameKind,
    pub(crate) centered: bool,
}

/// Centring decision for one frame: intro screens sit centred, an
/// open selection list expands to full area, overlays stay centred.
fn frame_centered(
    screen: app::AppScreen,
    show_help: bool,
    show_disclosure: bool,
    loading: bool,
    list_open: bool,
) -> bool {
    if show_help || show_disclosure || loading {
        return true;
    }
    matches!(
        screen,
        app::AppScreen::RoleSelect
            | app::AppScreen::Preflight
            | app::AppScreen::Proposal
            | app::AppScreen::ConnectionWait
            | app::AppScreen::SelectMedium
            | app::AppScreen::SelectHard
    ) && !list_open
}

impl ExpertLoopState {
    /// True while a question list is open on a selection screen.
    fn selection_list_open(&self, screen: app::AppScreen) -> bool {
        let Some(active) = self.runtime.as_ref() else {
            return false;
        };
        match screen {
            app::AppScreen::SelectMedium => {
                active.has_options(crate::tui::expert_runtime::SelectionScreen::Medium)
            }
            app::AppScreen::SelectHard => {
                active.has_options(crate::tui::expert_runtime::SelectionScreen::Hard)
            }
            _ => false,
        }
    }

    pub(crate) fn dispatch_key(
        &mut self,
        app: &mut app::ExpertApp,
        code: crossterm::event::KeyCode,
    ) {
        handle_key(
            app,
            &mut self.runtime,
            &mut self.tokio_rt,
            &mut self.pastes,
            &mut self.pending,
            &mut self.api_key_input,
            code,
        );
    }

    /// Animated centred loading lines for one frame.
    pub(crate) fn loading_lines(&mut self) -> Vec<String> {
        self.loading_tick += 1;
        render::loading_lines(self.loading_tick)
    }

    /// One loop tick before drawing: first-visit preflight, finished
    /// background work, then the bounded runtime step.
    pub(crate) fn tick(&mut self, app: &mut app::ExpertApp, preflight_done: &mut bool) {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        self.poll_proposal(app);
        self.poll_runtime_build(app);
        self.poll_selection_fetch(app);
        self.poll_api_key_check(app);
        tick_runtime(app, &mut self.runtime, &mut self.tokio_rt);
    }

    /// Applies a finished background pool fetch; a fully auto-picked
    /// small pool is announced so it is not mistaken for breakage.
    pub(crate) fn poll_selection_fetch(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.selection_rx else {
            return;
        };
        match rx.try_recv() {
            Ok((active, screen, result)) => {
                self.pending.selection_rx = None;
                match result {
                    Ok(_) => {
                        if active.auto_selected(screen) {
                            let label = match screen {
                                crate::tui::expert_runtime::SelectionScreen::Medium => "medium",
                                crate::tui::expert_runtime::SelectionScreen::Hard => "hard",
                            };
                            app.set_status_message(format!(
                                "only {} {label} questions were found for this candidate, so all of them are selected automatically.",
                                active.option_count(screen)
                            ));
                        } else if let Some(note) = active.selection_note() {
                            app.set_status_message(note);
                        } else {
                            app.clear_status_message();
                        }
                    }
                    Err(err) => set_error(app, "selection failed", &err),
                }
                self.runtime = Some(active);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.selection_rx = None;
                set_error(app, "selection failed", "selection worker stopped");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Applies a finished background API-key health check.
    pub(crate) fn poll_api_key_check(&mut self, app: &mut app::ExpertApp) {
        let Some(rx) = &self.pending.api_key_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.pending.api_key_rx = None;
                save_checked_api_key(app, &mut self.api_key_input, result);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending.api_key_rx = None;
                set_error(app, "API key check failed; key not saved", "worker stopped");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Decides one frame without touching the terminal: which lines
    /// are visible at `terminal_height`, and which renderer draws them.
    pub(crate) fn frame_plan(&mut self, app: &app::ExpertApp, terminal_height: usize) -> FramePlan {
        let loading = self.is_loading();
        let lines = if loading {
            self.loading_lines()
        } else {
            self.build_lines(app)
        };
        let visible = viewport::visible_with_pinned(&lines, terminal_height.saturating_sub(2));
        let centered = frame_centered(
            app.screen(),
            self.show_help,
            self.show_disclosure,
            loading,
            self.selection_list_open(app.screen()),
        ) || self.show_question_preview
            || self.show_random_confirm;
        let show_setup = app.screen() == app::AppScreen::Setup
            && !self.show_help
            && !self.show_disclosure
            && !loading;
        FramePlan {
            visible,
            kind: if loading {
                FrameKind::Loading
            } else if show_setup {
                FrameKind::Setup
            } else {
                FrameKind::Screen
            },
            centered,
        }
    }

    /// Setup-screen panel data for the current pastes and box heights.
    pub(crate) fn setup_panel(&self) -> render::SetupPanel<'_> {
        render::SetupPanel {
            job_description: self.pastes.jd(),
            additional_notes: self.pastes.notes(),
            job_focused: self.pastes.focus() == setup::ExpertSetupFocus::JobDescription,
            job_scroll: self.pastes.jd_scroll(),
            notes_scroll: self.pastes.notes_scroll(),
            job_height: self.setup_jd_height,
            notes_height: self.setup_notes_height,
        }
    }

    /// Routes one terminal event. Returns the string a `[COPY]` click
    /// should copy; the caller performs the clipboard write itself.
    pub(crate) fn handle_event(
        &mut self,
        app: &mut app::ExpertApp,
        event: crossterm::event::Event,
        area: ratatui::layout::Rect,
        visible: &[String],
    ) -> Option<String> {
        use crossterm::event::{Event, MouseButton, MouseEventKind};
        match event {
            Event::Key(key) => {
                self.handle_key_event(app, key);
                None
            }
            Event::Paste(text) => {
                if !self.is_loading() {
                    handle_paste(app, &mut self.pastes, &mut self.api_key_input, &text);
                }
                None
            }
            Event::Mouse(mouse)
                if app.screen() == app::AppScreen::Setup
                    && !self.show_help
                    && !self.is_loading() =>
            {
                self.handle_setup_mouse(area, mouse);
                None
            }
            Event::Mouse(mouse)
                if matches!(
                    app.screen(),
                    app::AppScreen::ConnectionWait | app::AppScreen::Monitor
                ) && !self.show_help
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) =>
            {
                if render::is_copy_button_at(visible, area, mouse.column, mouse.row) {
                    self.copy_token(app.screen())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Quitting with a built runtime still ends the session (without
    /// waiting for the final ack) so sandbox cleanup always runs.
    pub(crate) fn finish(&mut self) {
        if let Some(active) = self.runtime.as_mut() {
            let _ = block_on(
                &mut self.tokio_rt,
                active.end_session_with_ack_deadline(now_ms(), 0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_on_setup() -> app::ExpertApp {
        let mut app = app::ExpertApp::new();
        app.set_preflight_passed(true);
        app.set_disclosure_acknowledged(true);
        app.set_api_key_ready(true);
        let _ = app.handle(app::AppEvent::SelectExpert);
        let _ = app.handle(app::AppEvent::PreflightCompleted);
        assert_eq!(app.screen(), app::AppScreen::Setup);
        app
    }

    #[test]
    fn frame_plan_centres_role_select_as_screen() {
        let app = app::ExpertApp::new();
        let mut state = ExpertLoopState::default();
        let plan = state.frame_plan(&app, 24);
        assert_eq!(plan.kind, FrameKind::Screen);
        assert!(plan.centered);
        assert!(plan.visible.iter().any(|line| line == "Main Menu"));
    }

    #[test]
    fn frame_plan_uses_setup_kind_only_on_setup_without_overlays() {
        let app = app_on_setup();
        let mut state = ExpertLoopState::default();
        assert_eq!(state.frame_plan(&app, 24).kind, FrameKind::Setup);
        state.show_help = true;
        let plan = state.frame_plan(&app, 24);
        assert_eq!(plan.kind, FrameKind::Screen);
        assert!(plan.centered);
    }

    #[test]
    fn frame_plan_centres_select_medium_as_screen() {
        let mut app = app_on_setup();
        app.set_has_proposal(true);
        let _ = app.handle(app::AppEvent::ProposalReady);
        let _ = app.handle(app::AppEvent::ProposalConfirmed);
        app.set_connected(true);
        let _ = app.handle(app::AppEvent::ConnectionLocked);
        assert_eq!(app.screen(), app::AppScreen::SelectMedium);
        let mut state = ExpertLoopState::default();
        let plan = state.frame_plan(&app, 24);
        assert_eq!(plan.kind, FrameKind::Screen);
        assert!(plan.centered);
    }

    #[test]
    fn frame_plan_centres_select_hard_before_list_opens() {
        let mut app = app_on_setup();
        app.set_has_proposal(true);
        let _ = app.handle(app::AppEvent::ProposalReady);
        let _ = app.handle(app::AppEvent::ProposalConfirmed);
        app.set_connected(true);
        let _ = app.handle(app::AppEvent::ConnectionLocked);
        let _ = app.handle(app::AppEvent::MediumSelected);
        assert_eq!(app.screen(), app::AppScreen::SelectHard);
        let mut state = ExpertLoopState::default();
        let plan = state.frame_plan(&app, 24);
        assert_eq!(plan.kind, FrameKind::Screen);
        assert!(plan.centered);
    }

    #[test]
    fn frame_centered_expands_when_selection_list_open() {
        use app::AppScreen as S;
        // Intro selection screens are centred until the list opens,
        // then the frame goes full-area for picking.
        assert!(frame_centered(S::SelectMedium, false, false, false, false));
        assert!(frame_centered(S::SelectHard, false, false, false, false));
        assert!(!frame_centered(S::SelectMedium, false, false, false, true));
        assert!(!frame_centered(S::SelectHard, false, false, false, true));
        // Help and loading stay centred even over an open list, and
        // non-intro screens are never centred by this rule.
        assert!(frame_centered(S::SelectHard, true, false, false, true));
        assert!(frame_centered(S::Monitor, false, false, true, false));
        assert!(!frame_centered(S::Monitor, false, false, false, false));
    }

    #[test]
    fn frame_plan_loading_wins_over_setup() {
        let app = app_on_setup();
        let mut state = ExpertLoopState::default();
        let (_tx, rx) = std::sync::mpsc::channel();
        state.pending.proposal_rx = Some(rx);
        let plan = state.frame_plan(&app, 24);
        assert_eq!(plan.kind, FrameKind::Loading);
        assert!(plan.centered);
    }

    #[test]
    fn paste_event_is_ignored_while_loading() {
        let mut app = app_on_setup();
        let mut state = ExpertLoopState::default();
        let (_tx, rx) = std::sync::mpsc::channel();
        state.pending.proposal_rx = Some(rx);
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let copied = state.handle_event(
            &mut app,
            crossterm::event::Event::Paste("job text".to_string()),
            area,
            &[],
        );
        assert_eq!(copied, None);
        assert!(state.pastes.jd().is_empty());
    }

    #[test]
    fn build_lines_keeps_footer_last_behind_masked_key() {
        let mut app = app::ExpertApp::new();
        let _ = app.handle(app::AppEvent::SelectExpert);
        app.set_api_key_entry(true);
        let mut state = ExpertLoopState {
            api_key_input: "secret123".to_string(),
            ..ExpertLoopState::default()
        };
        let lines = state.build_lines(&app);
        assert_eq!(
            lines.last().map(String::as_str),
            app::footer(&app).as_deref()
        );
        let masked = lines
            .iter()
            .position(|line| line.contains("Key:"))
            .expect("masked line");
        assert_eq!(masked, lines.len() - 2, "{lines:?}");
        assert!(!lines.join("\n").contains("secret123"));
    }
}
