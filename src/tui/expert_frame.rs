//! Expert frame planning, event routing, and exit cleanup for the
//! TTY shell in `tui::mod`: decisions the shell used to inline.

use crate::tui::expert_keys::handle_paste;
use crate::tui::expert_loop::{tick_runtime, ExpertLoopState};
use crate::tui::expert_tty::{block_on, now_ms, run_preflight_check};
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

impl ExpertLoopState {
    /// One loop tick before drawing: first-visit preflight, finished
    /// background work, then the bounded runtime step.
    pub(crate) fn tick(&mut self, app: &mut app::ExpertApp, preflight_done: &mut bool) {
        if app.screen() == app::AppScreen::Preflight && !*preflight_done {
            *preflight_done = true;
            run_preflight_check(app);
        }
        self.poll_proposal(app);
        self.poll_runtime_build(app);
        tick_runtime(app, &mut self.runtime, &mut self.tokio_rt);
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
        let centered = matches!(
            app.screen(),
            app::AppScreen::RoleSelect
                | app::AppScreen::Preflight
                | app::AppScreen::Proposal
                | app::AppScreen::ConnectionWait
        ) || self.show_help
            || self.show_disclosure
            || loading;
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
