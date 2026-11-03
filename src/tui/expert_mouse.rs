//! Expert loop mouse and clipboard helpers: the `[COPY]` token
//! and Setup-screen mouse routing.

use crate::tui::expert_loop::ExpertLoopState;
use crate::tui::expert_tty::now_ms;
use crate::tui::{app, render, setup};

impl ExpertLoopState {
    /// Returns the string a `[COPY]` button click should copy on
    /// `screen`: the current connection string while waiting, the
    /// reissued one on the monitor.
    pub(crate) fn copy_token(&mut self, screen: app::AppScreen) -> Option<String> {
        let active = self.runtime.as_mut()?;
        match screen {
            app::AppScreen::ConnectionWait => active
                .current_string(now_ms())
                .ok()
                .map(|current| current.encode()),
            app::AppScreen::Monitor => active.reissue_string().map(|fresh| fresh.encode()),
            _ => None,
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
