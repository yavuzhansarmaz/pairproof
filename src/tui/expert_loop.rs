//! Expert TTY loop state: frame building, key/mouse routing, and
//! the bounded per-tick runtime step.

use crate::tui::expert_keys::selection_screen;
use crate::tui::expert_runtime::ExpertRuntime;
use crate::tui::expert_tty::now_ms;
use crate::tui::setup::ExpertSetupPastes;
use crate::tui::{app, help, monitor, setup};

/// One finished background pool fetch: the runtime handed back,
/// which selection screen it belongs to, and the offered count.
pub(crate) type SelectionFetchReceiver = std::sync::mpsc::Receiver<(
    ExpertRuntime,
    crate::tui::expert_runtime::SelectionScreen,
    Result<usize, String>,
)>;

/// One finished question-preview fetch: screen, slug, plain-text
/// statement (already converted from HTML by the worker).
pub(crate) type QuestionPreviewReceiver = std::sync::mpsc::Receiver<(
    crate::tui::expert_runtime::SelectionScreen,
    String,
    Result<String, String>,
)>;

/// Background work the TTY loop waits on: a proposal request and
/// a deferred runtime build, both surfaced as the loading screen.
#[derive(Default)]
pub(crate) struct PendingWork {
    pub(crate) proposal_rx:
        Option<std::sync::mpsc::Receiver<Result<crate::ingest::ProposedContext, String>>>,
    pub(crate) runtime_rx: Option<std::sync::mpsc::Receiver<Result<ExpertRuntime, String>>>,
    pub(crate) selection_rx: Option<SelectionFetchReceiver>,
    pub(crate) api_key_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    pub(crate) preview_rx: Option<QuestionPreviewReceiver>,
}

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
    pub(crate) show_question_preview: bool,
    pub(crate) show_random_confirm: bool,
    pub(crate) question_preview_scroll: usize,
    pub(crate) question_preview_max_scroll: usize,
    pub(crate) question_preview_page: usize,
    pub(crate) question_preview_tick: usize,
    pub(crate) pending: PendingWork,
    pub(crate) loading_tick: usize,
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
            show_question_preview: false,
            show_random_confirm: false,
            question_preview_scroll: 0,
            // Until the first preview frame measures the real range,
            // scrolling is unbounded here; `frame_plan` clamps it to
            // the wrapped content before anything is drawn.
            question_preview_max_scroll: usize::MAX,
            question_preview_page: 10,
            question_preview_tick: 0,
            pending: PendingWork::default(),
            loading_tick: 0,
        }
    }
}

impl ExpertLoopState {
    /// Builds the lines for one frame: app lines, live runtime lines,
    /// the masked API-key entry, then help/disclosure replacement.
    pub(crate) fn build_lines(&mut self, app: &app::ExpertApp) -> Vec<String> {
        // Content and footer arrive separately, so runtime lines are
        // spliced into the content and the footer is re-appended last
        // by construction, on every screen.
        let (mut lines, footer) = app::screen_parts(app);
        let mut held_footer = if !self.show_help && !self.show_disclosure {
            footer
        } else {
            None
        };
        if let Some(active) = self.runtime.as_mut() {
            let now = now_ms();
            match app.screen() {
                app::AppScreen::ConnectionWait => {
                    if let Ok(_current) = active.current_string(now) {
                        // The string itself is never displayed; the
                        // button copies it straight to the clipboard.
                        lines.push("connection string: [COPY] (click to copy)".to_string());
                        lines.push(format!(
                            "candidate SSH login expires in {}s.",
                            active.countdown_secs(now)
                        ));
                    }
                }
                app::AppScreen::SelectMedium => {
                    lines.extend(
                        active.selection_lines(crate::tui::expert_runtime::SelectionScreen::Medium),
                    );
                }
                app::AppScreen::SelectHard => {
                    lines.extend(
                        active.selection_lines(crate::tui::expert_runtime::SelectionScreen::Hard),
                    );
                }
                app::AppScreen::Monitor => {
                    lines.extend(monitor::render_monitor_lines(&active.monitor_view(now)));
                    lines.extend(active.recovery_lines());
                    if active.reissue_string().is_some() {
                        lines.push("connection string: [COPY] (click to copy)".to_string());
                        lines.push(format!(
                            "candidate SSH login expires in {}s.",
                            active.countdown_secs(now)
                        ));
                    }
                    if active.is_disconnected() {
                        lines.push("press c to reissue a fresh connection string.".to_string());
                    }
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
            // Last content line; the held footer is appended after it.
            lines.push(masked);
        }
        if !self.show_help && !self.show_disclosure {
            if let Some(screen) = selection_screen(app) {
                let loaded = self
                    .runtime
                    .as_ref()
                    .is_some_and(|active| active.has_options(screen));
                held_footer = app::selection_footer(app, loaded);
            }
        }
        if let Some(foot) = held_footer {
            lines.push(foot);
        }
        if self.show_disclosure {
            lines = help::disclosure_lines();
        } else if self.show_help {
            lines = help::expert_help_lines(app.screen());
            if let Some(screen) = selection_screen(app) {
                let loaded = self
                    .runtime
                    .as_ref()
                    .is_some_and(|active| active.has_options(screen));
                if !loaded {
                    lines.retain(|line| !line.contains("r pick at random"));
                }
            }
        }
        if self.show_random_confirm {
            if let (Some(active), Some(screen)) = (self.runtime.as_ref(), selection_screen(app)) {
                let (label, need) = match screen {
                    crate::tui::expert_runtime::SelectionScreen::Medium => ("medium", 3),
                    crate::tui::expert_runtime::SelectionScreen::Hard => ("hard", 2),
                };
                let need = need.min(active.option_count(screen));
                lines = vec![
                    "Random selection".to_string(),
                    String::new(),
                    format!(
                        "{need} {label} questions will be picked at random from the {} in this list.",
                        active.option_count(screen)
                    ),
                    "your current picks will be replaced.".to_string(),
                    String::new(),
                    "enter pick at random, b back to the list".to_string(),
                ];
            }
        }
        lines
    }

    /// True while a background request (proposal or runtime build)
    /// runs.
    pub(crate) fn is_loading(&self) -> bool {
        self.pending.proposal_rx.is_some()
            || self.pending.runtime_rx.is_some()
            || self.pending.selection_rx.is_some()
            || self.pending.api_key_rx.is_some()
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
        if self.is_loading() {
            // Loading: only quit keys are live; the rest is ignored
            // so the request cannot be re-entered or navigated away.
            let quit = key.code == crossterm::event::KeyCode::Esc
                || (key.code == crossterm::event::KeyCode::Char('c')
                    && key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL));
            if quit {
                let _ = app.handle(app::AppEvent::Quit);
            }
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
        if self.show_question_preview && selection_screen(app).is_none() {
            self.show_question_preview = false;
        }
        if self.show_question_preview {
            // Question preview: `b` returns to the list at the
            // question being previewed; left/right step to the
            // neighbouring question; up/down scroll the statement;
            // quit keys still quit; the rest is swallowed so picks
            // cannot drift behind it.
            match key.code {
                crossterm::event::KeyCode::Char('b') => {
                    self.show_question_preview = false;
                    self.question_preview_scroll = 0;
                }
                crossterm::event::KeyCode::Char(' ') => {
                    if let (Some(active), Some(screen)) =
                        (self.runtime.as_mut(), selection_screen(app))
                    {
                        active.toggle_selection_pick(screen);
                    }
                }
                crossterm::event::KeyCode::Left => self.step_preview_question(app, -1),
                crossterm::event::KeyCode::Right => self.step_preview_question(app, 1),
                crossterm::event::KeyCode::Up => self.scroll_preview_by(-1),
                crossterm::event::KeyCode::Down => self.scroll_preview_by(1),
                crossterm::event::KeyCode::PageUp => {
                    self.scroll_preview_by(-(self.question_preview_page as i32))
                }
                crossterm::event::KeyCode::PageDown => {
                    self.scroll_preview_by(self.question_preview_page as i32)
                }
                crossterm::event::KeyCode::Home => self.question_preview_scroll = 0,
                crossterm::event::KeyCode::End => {
                    self.question_preview_scroll = self.question_preview_max_scroll
                }
                crossterm::event::KeyCode::Esc => {
                    let _ = app.handle(app::AppEvent::Quit);
                }
                crossterm::event::KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    let _ = app.handle(app::AppEvent::Quit);
                }
                _ => {}
            }
            return;
        }
        if self.show_random_confirm && selection_screen(app).is_none() {
            self.show_random_confirm = false;
        }
        if self.show_random_confirm {
            // Random-pick confirmation: Enter draws the picks and
            // immediately acts as the list Enter, so Medium moves to
            // Hard and Hard commits to the Monitor; `b` returns
            // without touching the current picks; quit keys quit.
            match key.code {
                crossterm::event::KeyCode::Enter => {
                    let should_advance = if let (Some(active), Some(screen)) =
                        (self.runtime.as_mut(), selection_screen(app))
                    {
                        active.randomize_picks(screen);
                        true
                    } else {
                        false
                    };
                    self.show_random_confirm = false;
                    if should_advance {
                        self.dispatch_key(app, crossterm::event::KeyCode::Enter);
                    }
                }
                crossterm::event::KeyCode::Char('b') => self.show_random_confirm = false,
                crossterm::event::KeyCode::Esc => {
                    let _ = app.handle(app::AppEvent::Quit);
                }
                crossterm::event::KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    let _ = app.handle(app::AppEvent::Quit);
                }
                _ => {}
            }
            return;
        }
        if key.code == crossterm::event::KeyCode::Char('r')
            && !self.show_help
            && !self.show_disclosure
            && !self.show_question_preview
        {
            if let (Some(screen), Some(active)) = (selection_screen(app), self.runtime.as_ref()) {
                if active.has_options(screen) {
                    self.show_random_confirm = true;
                    return;
                }
            }
        }
        if key.code == crossterm::event::KeyCode::Tab && !self.show_help && !self.show_disclosure {
            if let (Some(screen), Some(active)) = (selection_screen(app), self.runtime.as_ref()) {
                if active.has_options(screen) {
                    self.show_question_preview = true;
                    self.question_preview_scroll = 0;
                    // Reopening the preview retries a failed load.
                    if let Some(active) = self.runtime.as_mut() {
                        active.clear_preview_error(screen);
                    }
                    self.start_question_preview_fetch(screen);
                    return;
                }
            }
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
}
