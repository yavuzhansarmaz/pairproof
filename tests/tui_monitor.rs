use pairproof::core::{ApproachChoice, CoreError, Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::scoring::{final_rating, ApproachQuality, GradeOutcome, QuestionGrade};
use pairproof::tui::monitor::*;
use std::collections::BTreeMap;

fn q(slug: &str, d: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty: d,
        acceptance_pct: None,
    }
}
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{p}{i}"), d)).collect()
}
fn started() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}
fn ok_step() -> CleanupStep {
    Box::new(|| Box::pin(async { Ok(()) }))
}
fn fail_step(msg: &'static str) -> CleanupStep {
    Box::new(move || Box::pin(async move { Err(msg.to_string()) }))
}
fn cancel_step(supported: bool) -> CleanupStep {
    Box::new(move || {
        Box::pin(async move {
            if supported {
                Ok(())
            } else {
                Err("unsupported: transport cannot cancel (Phase 10 wire-up)".to_string())
            }
        })
    })
}
fn cleanup_seq(cancel: CleanupStep, stop_sync: CleanupStep) -> CleanupSequence {
    CleanupSequence::new(cancel, ok_step(), stop_sync, ok_step(), ok_step())
}
fn graded(slot: usize, grade: f64, corr: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), corr).expect("grade"),
    )
}

#[test]
fn expert_monitor_never_renders_chat_content() {
    let mut s = started();
    s.select_approach(ApproachChoice::Other("sliding window".into()), 100, 5)
        .expect("approach");
    s.redirect("SECRET_INSTRUCTION use a heap", 6)
        .expect("redirect");
    s.record_chat_boundary_violation(7).expect("violation");
    let view = monitor_view(&s, &BTreeMap::new(), 100);
    let rendered = render_monitor_lines(&view).join("\n");
    assert_eq!(view.approach_label.as_deref(), Some("sliding window"));
    assert!(!rendered.contains("SECRET_INSTRUCTION"), "{rendered}");
    assert!(!rendered.contains("use a heap"), "{rendered}");
    assert!(rendered.contains("sliding window"), "{rendered}");
    for line in view.rating_lines.iter().chain(view.paste_alerts.iter()) {
        assert!(!line.contains("SECRET_INSTRUCTION"), "{line}");
    }
}

#[test]
fn monitor_displays_only_recorded_test_summary() {
    let mut s = started();
    let before = monitor_view(&s, &BTreeMap::new(), 50);
    assert_eq!(before.latest_test, None);
    assert!(render_monitor_lines(&before)
        .join("\n")
        .contains("no test run yet"));
    s.record_test_result(3, 1, 60).expect("test");
    let after = monitor_view(&s, &BTreeMap::new(), 70);
    assert_eq!(after.latest_test.as_deref(), Some("3 passed, 1 failed"));
    assert!(render_monitor_lines(&after)
        .join("\n")
        .contains("3 passed, 1 failed"));
    s.record_test_result(5, 2, 80).expect("test2");
    assert_eq!(
        monitor_view(&s, &BTreeMap::new(), 90)
            .latest_test
            .as_deref(),
        Some("5 passed, 2 failed")
    );
}

#[test]
fn paste_flag_is_visible_to_expert() {
    let mut s = started();
    assert!(monitor_view(&s, &BTreeMap::new(), 10)
        .paste_alerts
        .is_empty());
    s.record_paste_flag(500, 11).expect("paste");
    s.record_paste_flag(900, 12).expect("paste");
    let view = monitor_view(&s, &BTreeMap::new(), 20);
    assert_eq!(
        view.paste_alerts,
        vec!["paste flag on Q1: 2 confirmed bulk paste(s)".to_string()]
    );
    assert!(render_monitor_lines(&view)
        .join("\n")
        .contains("paste flag on Q1: 2 confirmed bulk paste(s)"));
}

#[test]
fn accidental_disconnect_does_not_finalize_session() {
    let mut s = started();
    assert_eq!(s.status(), SessionStatus::InProgress);
    handle_disconnect(&mut s, 100).expect("disconnect");
    assert_eq!(s.status(), SessionStatus::InProgress);
    assert!(!is_final_after(s.status()));
    assert!(!s.is_connected());
    let err = handle_disconnect(&mut s, 101).expect_err("second disconnect");
    assert_eq!(err, MonitorError::Core(CoreError::CandidateNotConnected));
    assert_eq!(s.status(), SessionStatus::InProgress);
}

#[tokio::test]
async fn cleanup_sequence_attempts_all_after_one_failure() {
    let mut seq = cleanup_seq(ok_step(), fail_step("sync boom"));
    let report = seq.run().await;
    assert_eq!(report.attempted, CLEANUP_STEP_NAMES);
    assert!(report
        .succeeded
        .contains(&"terminate_connection".to_string()));
    assert!(report.succeeded.contains(&"destroy_sandbox".to_string()));
    assert!(report.succeeded.contains(&"delete_raw".to_string()));
    assert!(!report.succeeded.contains(&"stop_sync".to_string()));
    assert_eq!(
        report.failed,
        vec![("stop_sync".to_string(), "sync boom".to_string())]
    );
    assert!(!report.all_succeeded());
}

#[tokio::test]
async fn cleanup_sequence_cancels_supported_inflight_work() {
    let mut seq = cleanup_seq(cancel_step(true), ok_step());
    let report = seq.run().await;
    assert_eq!(report.attempted[0], "cancel_inflight");
    assert!(report.succeeded.contains(&"cancel_inflight".to_string()));
    assert_eq!(report.cancel_outcome(), Some(CancelOutcome::Cancelled));
    assert!(report.all_succeeded());
}

#[tokio::test]
async fn cleanup_sequence_is_idempotent() {
    let mut seq = cleanup_seq(ok_step(), ok_step());
    assert!(!seq.is_done());
    let first = seq.run().await;
    assert!(!first.already_done);
    assert_eq!(first.attempted.len(), 5);
    assert!(seq.is_done());
    let second = seq.run().await;
    assert!(second.already_done);
    assert!(second.attempted.is_empty());
    assert!(second.succeeded.is_empty());
}

#[test]
fn final_view_uses_real_breakdown_without_fabricated_report() {
    let mut s = started();
    s.finish_question(10).expect("finish");
    s.finish_question(11).expect("finish");
    let grades = vec![graded(0, 9.0, 0.2), graded(1, 8.0, 0.1)];
    let view = final_view(&s, &BTreeMap::new(), &grades).expect("final");
    let expected = final_rating(&s, &BTreeMap::new(), &grades).expect("expected");
    assert_eq!(view.rating, expected.rating);
    assert_eq!(view.solved_count, expected.solved_count);
    assert_eq!(view.finished_count, 2);
    assert!(
        view.per_question[0].contains("grade 9.0"),
        "{}",
        view.per_question[0]
    );
    assert!(
        !view.artefact_note.contains("Phase"),
        "{}",
        view.artefact_note
    );
    assert!(
        !view.artefact_note.contains("report.md exists"),
        "{}",
        view.artefact_note
    );
    assert!(
        !view
            .artefact_note
            .to_lowercase()
            .contains("report.md generated"),
        "{}",
        view.artefact_note
    );
}

#[test]
fn monitor_refresh_shows_current_question_and_rating() {
    let mut s = started();
    let v1 = monitor_view(&s, &BTreeMap::new(), 103);
    assert_eq!(v1.current_index, Some(0));
    assert_eq!(v1.current_question.as_deref(), Some("Title e0"));
    assert_eq!(v1.elapsed_ms, 100);
    assert_eq!(v1.per_question_ms, Some(100));
    assert_eq!(v1.status_label, "InProgress");
    s.finish_question(53).expect("finish");
    let v2 = monitor_view(&s, &BTreeMap::from([(0, vec![ApproachQuality::Best])]), 103);
    assert_eq!(v2.current_index, Some(1));
    assert_eq!(v2.current_question.as_deref(), Some("Title e1"));
    assert_eq!(v2.per_question_ms, Some(50));
    let lines = render_monitor_lines(&v2);
    assert_eq!(lines, render_monitor_lines(&v2));
    assert!(lines.join("\n").contains("live rating:"));
}

#[test]
fn disconnect_reconnect_resumes_same_question() {
    let mut s = started();
    s.finish_question(10).expect("finish");
    assert_eq!(s.current_index(), Some(1));
    handle_disconnect(&mut s, 20).expect("disconnect");
    assert_eq!(s.current_index(), Some(1));
    handle_reconnect(&mut s, 30).expect("reconnect");
    assert!(s.is_connected());
    assert_eq!(s.current_index(), Some(1));
    assert_eq!(s.status(), SessionStatus::InProgress);
    let mut s2 = started();
    handle_disconnect(&mut s2, 50).expect("disc");
    handle_reconnect(&mut s2, 60).expect("recon");
    assert_eq!(s2.current_index(), Some(0));
}

#[tokio::test]
async fn cancellation_unsupported_is_recorded_not_cancelled() {
    let mut seq = cleanup_seq(cancel_step(false), ok_step());
    let report = seq.run().await;
    assert!(report.attempted.contains(&"cancel_inflight".to_string()));
    assert!(!report.succeeded.contains(&"cancel_inflight".to_string()));
    assert!(
        report
            .failed
            .iter()
            .any(|(n, m)| n == "cancel_inflight" && m.contains("unsupported")),
        "{report:?}"
    );
    assert!(
        report.failed.iter().any(|(_, m)| m.contains("Phase 10")),
        "{report:?}"
    );
    assert_eq!(report.cancel_outcome(), Some(CancelOutcome::Unsupported));
    assert!(!report.all_succeeded());
}

#[tokio::test]
async fn cleanup_sequence_order_is_fixed_raw_deleted_last() {
    let mut seq = cleanup_seq(ok_step(), ok_step());
    let report = seq.run().await;
    assert_eq!(report.attempted, CLEANUP_STEP_NAMES);
    assert_eq!(
        report.attempted.last().map(String::as_str),
        Some("delete_raw")
    );
    assert_eq!(report.succeeded, report.attempted);
    assert!(report.all_succeeded());
    assert!(!report.already_done);
}

#[tokio::test]
async fn cleanup_sequence_done_blocks_rerun() {
    let mut seq = cleanup_seq(ok_step(), ok_step());
    assert!(!seq.is_done());
    let report = seq.run().await;
    assert!(report.all_succeeded());
    assert!(seq.is_done());
    assert!(seq.run().await.already_done);
    let mut s = started();
    handle_expert_end(&mut s, 99).expect("expert end");
    assert_eq!(s.status(), SessionStatus::EndedByExpert);
    assert!(is_final_after(s.status()));
    assert_ne!(EndKind::ExpertEnd, EndKind::AccidentalDisconnect);
}

#[test]
fn final_view_marks_not_scored_without_invented_grade() {
    let mut s = started();
    s.finish_question(10).expect("finish");
    s.finish_question(11).expect("finish");
    let grades = vec![
        graded(0, 7.0, 0.1),
        GradeOutcome::NotScored { slot_index: 1 },
    ];
    let view = final_view(&s, &BTreeMap::new(), &grades).expect("final");
    assert!(view.not_scored.contains(&1), "{view:?}");
    assert!(
        view.per_question[1].contains("not scored"),
        "{}",
        view.per_question[1]
    );
    assert!(
        view.per_question[1].contains("correction +0.000"),
        "{}",
        view.per_question[1]
    );
    assert!(
        !view.per_question[1].contains("grade 7"),
        "{}",
        view.per_question[1]
    );
    let empty = final_view(&s, &BTreeMap::new(), &[]).expect("empty final");
    assert!(empty.per_question[0].contains("not scored"));
    assert_eq!(empty.not_scored, vec![0, 1]);
}

#[test]
fn monitor_sanitizes_titles_and_approach_text() {
    let mut qs_evil = qs("e", 5, Difficulty::Easy);
    qs_evil[0].title = "Evil\u{1b}[31mTitle".into();
    let mut s = Session::new("s1".into(), Language::Python, qs_evil, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s.select_approach(ApproachChoice::Other("plan\u{1b}[0m X".into()), 100, 5)
        .expect("approach");
    s.select_approach(ApproachChoice::Preset(1), 100, 6)
        .unwrap_err();
    let view = monitor_view(&s, &BTreeMap::new(), 10);
    assert!(!view
        .current_question
        .as_deref()
        .unwrap_or("")
        .contains('\u{1b}'));
    assert!(!view
        .approach_label
        .as_deref()
        .unwrap_or("")
        .contains('\u{1b}'));
    assert_eq!(view.approach_label.as_deref(), Some("plan X"));
    let mut s2 = started();
    s2.select_approach(ApproachChoice::Preset(2), 100, 5)
        .expect("preset");
    assert_eq!(
        monitor_view(&s2, &BTreeMap::new(), 10)
            .approach_label
            .as_deref(),
        Some("preset 2")
    );
}

#[test]
fn monitor_durations_render_as_minutes_and_seconds() {
    assert_eq!(format_duration_ms(0), "0s");
    assert_eq!(format_duration_ms(45_000), "45s");
    assert_eq!(format_duration_ms(60_000), "1m 00s");
    assert_eq!(format_duration_ms(125_400), "2m 05s");
    let view = monitor_view(&started(), &BTreeMap::new(), 3_725_000);
    let rendered = render_monitor_lines(&view).join("\n");
    assert!(rendered.contains("elapsed: 62m 04s"), "{rendered}");
    assert!(rendered.contains("status: in progress"), "{rendered}");
    assert!(!rendered.contains(" ms"), "{rendered}");
}
