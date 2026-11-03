use std::collections::BTreeMap;
use std::path::PathBuf;

use pairproof::core::{
    read_events, ApproachChoice, Difficulty, EventKind, EventLogWriter, Language, Session,
    SessionEvent,
};
use pairproof::orchestration::evidence::{
    per_question_evidence, render_evidence, score_line_evidence, summary_evidence, Provenance,
};
use pairproof::orchestration::report::generate_report;
use pairproof::scoring::{
    final_rating, ApproachQuality, FinalBreakdown, GradeOutcome, QuestionGrade, ScoreComponent,
    ScoreLine,
};

mod common_mod;
use common_mod::qs;
fn line_for(b: &FinalBreakdown, c: ScoreComponent) -> &ScoreLine {
    b.lines.iter().find(|l| l.component == c).expect("line")
}

fn base(id: &str) -> Session {
    Session::new(id.into(), Language::Python, qs("e", 5, Difficulty::Easy), 0).expect("session")
}
fn started(id: &str) -> Session {
    let mut s = base(id);
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}
fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction).expect("grade"),
    )
}
fn write_events(session: &Session, name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pairproof-evidence-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("events.jsonl");
    let mut writer = EventLogWriter::create(&path).expect("writer");
    writer.write_pending(session).expect("write events");
    drop(writer);
    path
}
fn event_at_line(path: &PathBuf, line_no: u64) -> SessionEvent {
    let text = std::fs::read_to_string(path).expect("read events");
    let line = text.lines().nth((line_no - 1) as usize).expect("line");
    serde_json::from_str(line).expect("event parses")
}
fn extract_seqs(text: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("events.jsonl#seq=") {
        let after = &rest[pos + "events.jsonl#seq=".len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        out.push(digits.parse::<u64>().expect("seq digits"));
        rest = &after[digits.len()..];
    }
    out
}
fn assert_ref_resolves(path: &PathBuf, seq: u64) -> SessionEvent {
    let event = event_at_line(path, seq + 1);
    assert_eq!(event.seq, seq, "line {} must hold seq {seq}", seq + 1);
    event
}
fn rich_session() -> Session {
    let mut s = started("s-rich");
    s.select_approach(ApproachChoice::Preset(0), 100, 10)
        .expect("select");
    s.add_tokens(50).expect("tokens");
    s.redirect("use a heap", 11).expect("redirect");
    s.record_test_result(3, 1, 12).expect("test");
    s.record_paste_flag(500, 13).expect("paste");
    s.finish_question(110).expect("finish");
    s.select_approach(ApproachChoice::Preset(1), 100, 120)
        .expect("select 1");
    s.add_tokens(200).expect("tokens 1");
    s.skip_question(220).expect("skip");
    s
}
fn rich_breakdown(s: &Session) -> pairproof::scoring::FinalBreakdown {
    let map = BTreeMap::from([
        (0, vec![ApproachQuality::Best]),
        (1, vec![ApproachQuality::Workable]),
    ]);
    final_rating(
        s,
        &map,
        &[
            graded(0, 9.0, 0.2),
            GradeOutcome::NotScored { slot_index: 1 },
        ],
    )
    .expect("final")
}

#[test]
fn report_score_lines_link_to_matching_event_lines() {
    let s = rich_session();
    let b = rich_breakdown(&s);
    let path = write_events(&s, "score-lines");
    let report = generate_report(&s, &b, &[graded(0, 9.0, 0.2)], &[]);
    assert!(report.contains("[evidence:"), "{report}");
    assert!(!b.lines.is_empty());
    for line in &b.lines {
        let ev = score_line_evidence(&s, line);
        let rendered = render_evidence(&ev);
        assert!(
            report.contains(&format!("[evidence: {rendered}]")) || report.contains(&rendered),
            "report missing {rendered}"
        );
        for r in &ev.refs {
            assert_eq!(r.line_no(), r.seq + 1);
            let event = assert_ref_resolves(&path, r.seq);
            match line.component {
                ScoreComponent::Approach => assert!(
                    matches!(event.kind, EventKind::ApproachSelected { index, .. } if index == line.slot_index),
                    "{event:?}"
                ),
                ScoreComponent::Tokens => assert!(
                    matches!(event.kind, EventKind::TokensLocked { index, .. } | EventKind::QuestionFinished { index } | EventKind::QuestionSkipped { index } if index == line.slot_index),
                    "{event:?}"
                ),
                ScoreComponent::Intervention => assert!(
                    matches!(event.kind, EventKind::Redirected { index, .. } | EventKind::ApproachChanged { index } if index == line.slot_index),
                    "{event:?}"
                ),
                ScoreComponent::UnnecessaryChange => assert!(
                    matches!(event.kind, EventKind::ApproachChanged { index } if index == line.slot_index),
                    "{event:?}"
                ),
                ScoreComponent::Paste => assert!(
                    matches!(event.kind, EventKind::PasteFlagged { index, .. } if index == line.slot_index),
                    "{event:?}"
                ),
            }
        }
    }
    assert_eq!(
        score_line_evidence(&s, line_for(&b, ScoreComponent::Approach)).provenance,
        Provenance::Partial
    );
    assert_eq!(
        score_line_evidence(&s, line_for(&b, ScoreComponent::Paste)).provenance,
        Provenance::EventBacked
    );
    let summary = summary_evidence(&s, &b);
    assert_eq!(summary.finished.provenance, Provenance::EventBacked);
    assert_eq!(summary.skipped.provenance, Provenance::EventBacked);
    assert_eq!(summary.paste_flags.provenance, Provenance::EventBacked);
    assert_eq!(summary.total_time.provenance, Provenance::EventBacked);
    assert_eq!(summary.solved.provenance, Provenance::NoEventEvidence);
    assert_eq!(
        summary.ai_correction_total.provenance,
        Provenance::NoEventEvidence
    );
    assert_eq!(summary.rating.provenance, Provenance::Partial);
}

#[test]
fn evidence_reference_resolves_to_correct_event() {
    let mut s = started("s-resolve");
    s.select_approach(ApproachChoice::Preset(0), 100, 10)
        .expect("select");
    s.add_tokens(50).expect("tokens");
    s.finish_question(110).expect("finish");
    let map = BTreeMap::from([(0, vec![ApproachQuality::Best])]);
    let b = final_rating(&s, &map, &[graded(0, 8.0, 0.1)]).expect("final");
    let path = write_events(&s, "resolve");
    let read_back = read_events(&path).expect("read_events");
    assert_eq!(read_back.len(), s.events().len());
    let ev = score_line_evidence(&s, line_for(&b, ScoreComponent::Tokens));
    assert_eq!(ev.provenance, Provenance::Partial);
    assert_eq!(ev.refs.len(), 2);
    let kinds: Vec<EventKind> = ev
        .refs
        .iter()
        .map(|r| assert_ref_resolves(&path, r.seq).kind)
        .collect();
    assert!(kinds.iter().any(|k| matches!(
        k,
        EventKind::TokensLocked {
            index: 0,
            expected: 100
        }
    )));
    assert!(kinds
        .iter()
        .any(|k| matches!(k, EventKind::QuestionFinished { index: 0 })));
    let rendered = render_evidence(&ev);
    for seq in extract_seqs(&rendered) {
        let event = event_at_line(&path, seq + 1);
        assert_eq!(event.seq, seq);
    }
    let pq = per_question_evidence(&s, &b.per_question[0]);
    assert_eq!(pq.time.provenance, Provenance::EventBacked);
    assert_eq!(pq.time.refs.len(), 2);
    let time_kinds: Vec<EventKind> = pq
        .time
        .refs
        .iter()
        .map(|r| assert_ref_resolves(&path, r.seq).kind)
        .collect();
    assert!(time_kinds
        .iter()
        .any(|k| matches!(k, EventKind::QuestionStarted { index: 0 })));
    assert!(time_kinds
        .iter()
        .any(|k| matches!(k, EventKind::QuestionFinished { index: 0 })));
}

#[test]
fn score_line_supports_multiple_events() {
    let mut s = started("s-multi");
    s.select_approach(ApproachChoice::Preset(0), 100, 10)
        .expect("select");
    s.redirect("use a heap", 11).expect("redirect 1");
    s.redirect("handle empty input", 12).expect("redirect 2");
    s.change_approach(13).expect("change");
    s.select_approach(ApproachChoice::Preset(1), 100, 14)
        .expect("reselect");
    s.record_paste_flag(400, 15).expect("paste 1");
    s.record_paste_flag(600, 16).expect("paste 2");
    s.finish_question(110).expect("finish");
    let b = final_rating(&s, &BTreeMap::new(), &[]).expect("final");
    let path = write_events(&s, "multi");
    let ev = score_line_evidence(&s, line_for(&b, ScoreComponent::Intervention));
    assert_eq!(ev.provenance, Provenance::EventBacked);
    assert_eq!(ev.refs.len(), 3);
    let rendered = render_evidence(&ev);
    assert_eq!(extract_seqs(&rendered).len(), 3);
    for r in &ev.refs {
        let event = assert_ref_resolves(&path, r.seq);
        assert!(matches!(
            event.kind,
            EventKind::Redirected { index: 0, .. } | EventKind::ApproachChanged { index: 0 }
        ));
    }
    let pev = score_line_evidence(&s, line_for(&b, ScoreComponent::Paste));
    assert_eq!(pev.provenance, Provenance::EventBacked);
    assert_eq!(pev.refs.len(), 2);
    for r in &pev.refs {
        assert!(matches!(
            assert_ref_resolves(&path, r.seq).kind,
            EventKind::PasteFlagged { index: 0, .. }
        ));
    }
}

#[test]
fn evidence_links_are_relative_to_artifact_bundle() {
    let s = rich_session();
    let b = rich_breakdown(&s);
    let report = generate_report(&s, &b, &[graded(0, 9.0, 0.2)], &[]);
    assert!(report.contains("events.jsonl#seq="));
    for seq in extract_seqs(&report) {
        let needle = format!("events.jsonl#seq={seq} (line {})", seq + 1);
        assert!(report.contains(&needle), "missing {needle}");
    }
    // Refs must not carry an absolute path, a leading slash, or the session id.
    for line in report.lines().filter(|l| l.contains("events.jsonl#seq=")) {
        assert!(!line.contains("/tmp/"), "{line}");
        assert!(!line.contains("sessions/"), "{line}");
    }
    let ev = score_line_evidence(&s, &b.lines[0]);
    let rendered = render_evidence(&ev);
    assert!(
        rendered.starts_with("EventBacked ")
            || rendered.starts_with("Partial ")
            || rendered.starts_with("NoEventEvidence")
    );
    assert!(!rendered.contains("s-rich"), "{rendered}");
    assert!(!rendered.contains('/'), "{rendered}");
    assert!(!rendered.contains('\\'), "{rendered}");
    let summary = summary_evidence(&s, &b);
    for part in [
        &summary.finished,
        &summary.skipped,
        &summary.paste_flags,
        &summary.total_time,
        &summary.rating,
    ] {
        let text = render_evidence(part);
        assert!(!text.contains("s-rich"), "{text}");
        assert!(!text.contains("/tmp"), "{text}");
    }
}

#[test]
fn missing_score_evidence_is_not_fabricated() {
    let mut s = started("s-missing");
    s.finish_question(110).expect("finish");
    s.skip_question(220).expect("skip");
    let grades = vec![GradeOutcome::NotScored { slot_index: 0 }];
    let b = final_rating(&s, &BTreeMap::new(), &grades).expect("final");
    assert_eq!(b.per_question[0].grade, None);
    assert_eq!(
        b.per_question[1].status,
        pairproof::core::QuestionStatus::Skipped
    );
    let pq0 = per_question_evidence(&s, &b.per_question[0]);
    assert_eq!(pq0.grade.provenance, Provenance::NoEventEvidence);
    assert!(pq0.grade.refs.is_empty());
    assert_eq!(
        pq0.grade.note,
        Some("AI grading result; not recorded as an event")
    );
    assert_eq!(pq0.test.provenance, Provenance::NoEventEvidence);
    assert!(pq0.test.refs.is_empty());
    let pq1 = per_question_evidence(&s, &b.per_question[1]);
    assert_eq!(pq1.grade.provenance, Provenance::NoEventEvidence);
    assert_eq!(pq1.time.provenance, Provenance::EventBacked);
    // Fabricated paste line with no paste events must not be EventBacked.
    let fake = ScoreLine {
        slot_index: 0,
        component: ScoreComponent::Paste,
        delta: -0.30,
        detail: "paste flags 1 x-0.30".into(),
    };
    let fev = score_line_evidence(&s, &fake);
    assert_ne!(fev.provenance, Provenance::EventBacked);
    assert!(fev.refs.is_empty());
    let fake_intervention = ScoreLine {
        slot_index: 0,
        component: ScoreComponent::Intervention,
        delta: 0.20,
        detail: "interventions 2 redirects 2 changes 0".into(),
    };
    let iev = score_line_evidence(&s, &fake_intervention);
    assert_ne!(iev.provenance, Provenance::EventBacked);
    // No fabricated EventBacked with empty refs anywhere.
    for line in &b.lines {
        let ev = score_line_evidence(&s, line);
        assert!(!(ev.provenance == Provenance::EventBacked && ev.refs.is_empty()));
    }
    let summary = summary_evidence(&s, &b);
    assert_eq!(summary.solved.provenance, Provenance::NoEventEvidence);
    assert!(summary.solved.refs.is_empty());
    assert_eq!(
        summary.ai_correction_total.provenance,
        Provenance::NoEventEvidence
    );
    let report = generate_report(&s, &b, &grades, &[]);
    assert!(report.contains("NoEventEvidence"), "{report}");
    assert!(report.contains("not scored"), "{report}");
    let q2 = report
        .lines()
        .find(|l| l.starts_with("- Q2 "))
        .expect("q2 line");
    assert!(q2.contains("grade=NoEventEvidence"), "{q2}");
}

#[test]
fn evidence_generation_preserves_existing_event_log() {
    let s = rich_session();
    let b = rich_breakdown(&s);
    let path = write_events(&s, "preserve");
    let before = std::fs::read(&path).expect("read before");
    let events_before = s.events().to_vec();
    let report1 = generate_report(&s, &b, &[graded(0, 9.0, 0.2)], &[]);
    for line in &b.lines {
        let _ = render_evidence(&score_line_evidence(&s, line));
    }
    for pq in &b.per_question {
        let _ = per_question_evidence(&s, pq);
    }
    let _ = summary_evidence(&s, &b);
    let report2 = generate_report(&s, &b, &[graded(0, 9.0, 0.2)], &[]);
    assert_eq!(report1, report2);
    let after = std::fs::read(&path).expect("read after");
    assert_eq!(before, after);
    assert_eq!(s.events(), events_before.as_slice());
    let parsed = read_events(&path).expect("still valid");
    assert_eq!(parsed.len(), events_before.len());
    // Skipped slot in this session is present and honestly linked.
    let skipped = b
        .per_question
        .iter()
        .find(|q| q.slot_index == 1)
        .expect("skipped pq");
    assert_eq!(skipped.status, pairproof::core::QuestionStatus::Skipped);
    let ev = per_question_evidence(&s, skipped);
    assert_eq!(ev.grade.provenance, Provenance::NoEventEvidence);
    assert_eq!(ev.time.provenance, Provenance::EventBacked);
    for r in &ev.time.refs {
        assert_ref_resolves(&path, r.seq);
    }
}
