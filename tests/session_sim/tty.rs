use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::orchestration::{ApplyOutcome, IntentKind, SnapshotSelection};
use pairproof::tui::candidate_run::{CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{handle_live_key, CandidateRuntime, SshCandidateTransport};
use pairproof::tui::expert_runtime::ExpertRuntime;

use super::support::{chk, now_ms, META_WAIT_SECS, SYNC_WAIT_SECS};

type LiveCandidate = CandidateRuntime<SshCandidateTransport>;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn enter() -> KeyEvent {
    key(KeyCode::Enter)
}

fn settle(state: &mut CandidateRunState, candidate: &mut LiveCandidate) {
    let mut status = None;
    handle_live_key(state, candidate, key(KeyCode::Home), &mut status);
}

fn on_other_row(state: &CandidateRunState, option_count: usize) -> bool {
    option_count > 0
        && state
            .staging
            .current_stage()
            .is_some_and(|stage| stage.cursor >= option_count)
}

pub(super) fn leave_other_row(
    state: &mut CandidateRunState,
    candidate: &mut LiveCandidate,
) -> Result<(), String> {
    settle(state, candidate);
    chk(
        state.focus == CandidateFocus::Work,
        &format!("leave other: focus {:?} not Work", state.focus),
    )?;
    if on_other_row(state, candidate.view().options.len()) {
        press(state, candidate, KeyCode::Up, "leave other row")?;
    }
    chk(
        !on_other_row(state, candidate.view().options.len()),
        "leave other: cursor still on Other",
    )
}

pub(super) fn press(
    state: &mut CandidateRunState,
    candidate: &mut LiveCandidate,
    code: KeyCode,
    label: &str,
) -> Result<(), String> {
    let mut status = None;
    handle_live_key(state, candidate, key(code), &mut status);
    if let Some(status) = status {
        return Err(format!("{label}: {status}"));
    }
    Ok(())
}

fn type_chars(
    state: &mut CandidateRunState,
    candidate: &mut LiveCandidate,
    text: &str,
    label: &str,
) -> Result<(), String> {
    for ch in text.chars() {
        press(state, candidate, KeyCode::Char(ch), label)?;
    }
    Ok(())
}

pub(super) async fn pump_until(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    secs: u64,
    mut ready: impl FnMut(&mut ExpertRuntime, &mut LiveCandidate, &mut CandidateRunState) -> bool,
    msg: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let reports = expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| format!("{msg}: drain {e}"))?;
        let _ = candidate.poll_snapshot();
        settle(state, candidate);
        if state.staging.current_index().is_some_and(|index| {
            matches!(
                state.staging.flow_for(index),
                pairproof::tui::candidate_stage::QuestionFlowState::SyncingWorkspace { .. }
            )
        }) {
            let _ = candidate.flush();
        }
        if ready(expert, candidate, state) {
            return Ok(());
        }
        let qdir = candidate.view().question_index.map(|index| {
            let dir = candidate.workspace().join(format!("q{}", index + 1));
            let names = std::fs::read_dir(&dir).map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.file_name()))
                    .collect::<Vec<_>>()
            });
            format!("{dir:?}={names:?}")
        });
        chk(
            Instant::now() < deadline,
            &format!(
                "{msg}; reports={reports:?} index={:?} selected={:?} options={} title={:?} flow={:?} focus={:?} files={qdir:?} view={:?}",
                candidate.view().question_index,
                candidate.view().selected,
                candidate.view().options.len(),
                candidate.view().question_title,
                candidate
                    .view()
                    .question_index
                    .map(|index| state.staging.flow_for(index)),
                state.focus,
                candidate.view()
            ),
        )?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(super) async fn wait_options(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    index: usize,
) -> Result<(), String> {
    pump_until(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        |_, candidate, _| {
            candidate.view().question_index == Some(index)
                && candidate.view().options.len() == 4
                && candidate
                    .view()
                    .statement
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty())
        },
        &format!("question {index} never loaded live approaches"),
    )
    .await
}

pub(super) async fn wait_idle(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    secs: u64,
    msg: &str,
) -> Result<(), String> {
    pump_until(
        expert,
        candidate,
        state,
        secs,
        |_, candidate, state| {
            let Some(index) = candidate.view().question_index else {
                return false;
            };
            !state.staging.flow_for(index).is_busy()
                && !state.staging.flow_for(index).is_submitted_or_submitting()
        },
        msg,
    )
    .await
}

pub(super) async fn apply_kind(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    kind: IntentKind,
    msg: &str,
) -> Result<(), String> {
    let sent = candidate
        .send_intent(kind)
        .map_err(|e| format!("{msg}: send {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(META_WAIT_SECS);
    loop {
        let reports = expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| format!("{msg}: drain {e}"))?;
        let mine = reports
            .iter()
            .find(|report| report.intent_id.as_deref() == Some(sent.id.as_str()));
        if mine.is_some_and(|report| report.outcome == ApplyOutcome::Applied) {
            let _ = candidate.poll_snapshot();
            return Ok(());
        }
        if let Some(report) = mine {
            return Err(format!("{msg}: not applied: {report:?}"));
        }
        chk(
            Instant::now() < deadline,
            &format!("{msg}: intent {} never drained ({reports:?})", sent.id),
        )?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(super) async fn key_preset(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    preset: u8,
) -> Result<(), String> {
    wait_idle(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        &format!("preset {preset}: not idle before select"),
    )
    .await?;
    leave_other_row(state, candidate)?;
    chk(
        state.focus == CandidateFocus::Work,
        "preset: focus not Work",
    )?;
    let digit = char::from(b'1' + preset);
    press(state, candidate, KeyCode::Char(digit), "mark preset")?;
    press(state, candidate, KeyCode::Enter, "submit preset")?;
    pump_until(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        |_, candidate, _| candidate.view().selected == Some(SnapshotSelection::Preset(preset)),
        &format!("preset {preset} never applied"),
    )
    .await?;
    wait_idle(
        expert,
        candidate,
        state,
        SYNC_WAIT_SECS,
        &format!("preset {preset}: solution never settled"),
    )
    .await
}

pub(super) async fn key_other(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    text: &str,
) -> Result<(), String> {
    wait_idle(expert, candidate, state, META_WAIT_SECS, "other: not idle").await?;
    press(state, candidate, KeyCode::Char('o'), "open other")?;
    chk(
        state.focus == CandidateFocus::Other,
        "other: focus not Other",
    )?;
    type_chars(state, candidate, text, "type other")?;
    press(state, candidate, KeyCode::Enter, "submit other")?;
    pump_until(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        |_, candidate, _| {
            candidate.view().selected == Some(SnapshotSelection::Other(text.to_string()))
        },
        &format!("other {text:?} never applied"),
    )
    .await?;
    wait_idle(
        expert,
        candidate,
        state,
        SYNC_WAIT_SECS,
        "other: solution never settled",
    )
    .await
}

pub(super) async fn key_redirect(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    text: &str,
) -> Result<(), String> {
    wait_idle(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        "redirect: not idle",
    )
    .await?;
    leave_other_row(state, candidate)?;
    press(state, candidate, KeyCode::Char('c'), "open redirect")?;
    chk(
        state.focus == CandidateFocus::Redirect,
        "redirect: focus not Redirect",
    )?;
    type_chars(state, candidate, text, "type redirect")?;
    press(state, candidate, KeyCode::Enter, "submit redirect")?;
    wait_idle(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        "redirect: solution never settled",
    )
    .await
}

pub(super) async fn key_chat(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    text: &str,
) -> Result<(), String> {
    wait_idle(expert, candidate, state, META_WAIT_SECS, "chat: not idle").await?;
    leave_other_row(state, candidate)?;
    press(state, candidate, KeyCode::Tab, "focus chat")?;
    chk(state.focus == CandidateFocus::Chat, "chat: focus not Chat")?;
    type_chars(state, candidate, text, "type chat")?;
    press(state, candidate, KeyCode::Enter, "send chat")?;
    press(state, candidate, KeyCode::Tab, "leave chat")?;
    pump_until(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        |_, candidate, _| {
            candidate
                .view()
                .chat_replies
                .iter()
                .any(|reply| !reply.trim().is_empty())
        },
        "live chat reply never arrived",
    )
    .await
}

pub(super) async fn key_work(
    expert: &mut ExpertRuntime,
    candidate: &mut LiveCandidate,
    state: &mut CandidateRunState,
    ch: char,
    label: &str,
    wait_secs: u64,
) -> Result<(), String> {
    wait_idle(
        expert,
        candidate,
        state,
        META_WAIT_SECS,
        &format!("{label}: not idle"),
    )
    .await?;
    leave_other_row(state, candidate)?;
    press(state, candidate, KeyCode::Char(ch), label)?;
    wait_idle(
        expert,
        candidate,
        state,
        wait_secs,
        &format!("{label}: never settled"),
    )
    .await
}
