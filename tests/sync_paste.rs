use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::sync::{detect_bulk_insert, Snapshot, PASTE_BYTE_THRESHOLD, PASTE_LINE_THRESHOLD};

// One candidate PasteFlag must drive exactly one core paste flag; only
// expert-confirmed flags affect scoring, never raw detector output.
fn flat(n: usize) -> Vec<u8> {
    vec![b'x'; n]
}

fn lines(n: usize) -> Vec<u8> {
    "a\n".repeat(n).into_bytes()
}

#[test]
fn byte_boundary_flags_at_2048_with_few_lines() {
    let prev = Snapshot::from_bytes(&[]);
    assert_eq!(
        detect_bulk_insert(&prev, &Snapshot::from_bytes(&flat(2047))),
        None
    );
    let at = detect_bulk_insert(&prev, &Snapshot::from_bytes(&flat(2048))).expect("candidate");
    assert_eq!((at.inserted_bytes, at.inserted_lines), (2048, 0));
    assert!(detect_bulk_insert(&prev, &Snapshot::from_bytes(&flat(2049))).is_some());
}

#[test]
fn line_boundary_flags_at_50_with_few_bytes() {
    let prev = Snapshot::from_bytes(&[]);
    assert_eq!(
        detect_bulk_insert(&prev, &Snapshot::from_bytes(&lines(49))),
        None
    );
    let at = detect_bulk_insert(&prev, &Snapshot::from_bytes(&lines(50))).expect("candidate");
    assert_eq!((at.inserted_bytes, at.inserted_lines), (100, 50));
    assert!(detect_bulk_insert(&prev, &Snapshot::from_bytes(&lines(51))).is_some());
}

#[test]
fn shrink_identical_and_small_growth_are_not_candidates() {
    let big = Snapshot::from_bytes(&flat(5000));
    assert_eq!(
        detect_bulk_insert(&big, &Snapshot::from_bytes(&flat(10))),
        None
    );
    assert_eq!(detect_bulk_insert(&big, &big), None);
    let prev = Snapshot::from_bytes(b"def solve():\n    return 1\n");
    let grown = Snapshot::from_bytes(b"def solve():\n    return 2\n");
    assert_eq!(detect_bulk_insert(&prev, &grown), None);
}

#[test]
fn empty_to_large_is_a_candidate() {
    let flag = detect_bulk_insert(
        &Snapshot::from_bytes(&[]),
        &Snapshot::from_bytes(&flat(3000)),
    )
    .expect("candidate");
    assert_eq!((flag.inserted_bytes, flag.inserted_lines), (3000, 0));
}

#[test]
fn thresholds_count_bytes_not_chars() {
    // 1024 `é` are 2048 bytes but only 1024 chars; bytes must trigger.
    let text: String = "é".repeat(1024);
    let flag = detect_bulk_insert(
        &Snapshot::from_bytes(&[]),
        &Snapshot::from_bytes(text.as_bytes()),
    )
    .expect("candidate");
    assert_eq!(flag.inserted_bytes, 2048);
}

#[test]
fn ai_sized_rewrite_is_also_a_candidate() {
    // A large AI-sized insert (here ~3KB / 70 lines) is still only a
    // candidate: two snapshots cannot distinguish authorship, so the
    // detector reports size and leaves confirmation to expert judgment.
    let mut body = String::from("# solution\n");
    for i in 0..70 {
        body.push_str(&format!(
            "line_{i:02}_padding_to_grow_the_file_body = {i}\n"
        ));
    }
    assert!(body.len() >= 3000);
    let prev = Snapshot::from_bytes(b"# solution\n");
    let flag =
        detect_bulk_insert(&prev, &Snapshot::from_bytes(body.as_bytes())).expect("candidate");
    assert!(flag.inserted_bytes >= 2048 && flag.inserted_lines >= 50);
}

// Mirrors core::test_support::started via public API only (test_support is
// crate-private); the session below is a REAL core Session state machine.
fn started_session() -> Session {
    let easy: Vec<QuestionRef> = (0..5)
        .map(|i| QuestionRef {
            title_slug: format!("two-sum-{i}"),
            title: format!("Two Sum {i}"),
            difficulty: Difficulty::Easy,
            acceptance_pct: None,
        })
        .collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

#[test]
fn detector_output_drives_core_paste_flag() {
    let mut session = started_session();
    assert_eq!(session.paste_flag_count(), 0);
    let previous = Snapshot::from_bytes(b"def solve(nums):\n    pass\n");
    let mut grown = previous_content();
    grown.extend_from_slice(&flat(2500));
    let current = Snapshot::from_bytes(&grown);
    let flag = detect_bulk_insert(&previous, &current).expect("candidate");
    session
        .record_paste_flag(flag.inserted_bytes, 4)
        .expect("record");
    assert_eq!(session.paste_flag_count(), 1);
}

fn previous_content() -> Vec<u8> {
    b"def solve(nums):\n    pass\n".to_vec()
}

#[test]
fn snapshot_accessors_report_captured_counts() {
    let empty = Snapshot::from_bytes(&[]);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
    assert_eq!(empty.newline_count(), 0);
    let snap = Snapshot::from_bytes(b"a\nbb\n");
    assert_eq!(snap.len(), 5);
    assert!(!snap.is_empty());
    assert_eq!(snap.newline_count(), 2);
    assert_eq!(PASTE_BYTE_THRESHOLD, 2048);
    assert_eq!(PASTE_LINE_THRESHOLD, 50);
}
