use pairproof::ingest::{bound_text, prepare_documents, IngestError, MAX_TEXT_CHARS};

#[test]
fn prepare_rejects_blank_job_description() {
    for jd in ["", "   ", "\n\t  \n"] {
        let err = prepare_documents(jd, "notes").expect_err("blank JD must fail");
        assert!(
            matches!(err, IngestError::EmptyJobDescription),
            "jd {jd:?} got {err:?}"
        );
    }
}

#[test]
fn prepare_allows_blank_additional() {
    let docs = prepare_documents("Build things", "   ").expect("blank additional ok");
    assert_eq!(docs.jd.text, "Build things");
    assert_eq!(docs.additional.text, "");
    assert!(!docs.additional.truncated);
    assert_eq!(docs.additional.original_chars, 0);
    assert_eq!(docs.additional_text(), None);
}

#[test]
fn prepare_trims_outer_whitespace() {
    let docs = prepare_documents("  \n hello jd \n ", "\t extra notes  ").expect("prepare");
    assert_eq!(docs.jd.text, "hello jd");
    assert_eq!(docs.jd.original_chars, "hello jd".chars().count());
    assert!(!docs.jd.truncated);
    assert_eq!(docs.additional.text, "extra notes");
    assert_eq!(
        docs.additional.original_chars,
        "extra notes".chars().count()
    );
    assert_eq!(docs.additional_text(), Some("extra notes"));
}

#[test]
fn bound_leaves_short_text_untouched() {
    let bounded = bound_text("short text");
    assert_eq!(bounded.text, "short text");
    assert!(!bounded.truncated);
    assert_eq!(bounded.original_chars, 10);
    // bound_text itself does not trim.
    let spaced = bound_text("  keep  ");
    assert_eq!(spaced.text, "  keep  ");
    assert_eq!(spaced.original_chars, 8);
}

#[test]
fn bound_at_exact_limit_is_not_truncated() {
    let raw = "a".repeat(MAX_TEXT_CHARS);
    let bounded = bound_text(&raw);
    assert_eq!(bounded.text, raw);
    assert!(!bounded.truncated);
    assert_eq!(bounded.original_chars, MAX_TEXT_CHARS);
    assert_eq!(bounded.text.chars().count(), MAX_TEXT_CHARS);
}

#[test]
fn bound_one_over_limit_truncates_with_flag() {
    let raw = "a".repeat(MAX_TEXT_CHARS + 1);
    let bounded = bound_text(&raw);
    assert!(bounded.truncated);
    assert_eq!(bounded.original_chars, MAX_TEXT_CHARS + 1);
    assert_eq!(bounded.text.chars().count(), MAX_TEXT_CHARS);
    assert_eq!(bounded.text, "a".repeat(MAX_TEXT_CHARS));
}

#[test]
fn bound_truncates_on_char_boundary() {
    let raw = "ş".repeat(MAX_TEXT_CHARS + 5);
    let bounded = bound_text(&raw);
    assert!(bounded.truncated);
    assert_eq!(bounded.original_chars, MAX_TEXT_CHARS + 5);
    assert_eq!(bounded.text.chars().count(), MAX_TEXT_CHARS);
    assert_eq!(bounded.text, "ş".repeat(MAX_TEXT_CHARS));
    assert!(std::str::from_utf8(bounded.text.as_bytes()).is_ok());

    let emoji_raw = "🦀".repeat(MAX_TEXT_CHARS + 3);
    let emoji = bound_text(&emoji_raw);
    assert!(emoji.truncated);
    assert_eq!(emoji.original_chars, MAX_TEXT_CHARS + 3);
    assert_eq!(emoji.text.chars().count(), MAX_TEXT_CHARS);
    assert_eq!(emoji.text, "🦀".repeat(MAX_TEXT_CHARS));
    assert!(std::str::from_utf8(emoji.text.as_bytes()).is_ok());

    // Mixed widths straddling the limit.
    let mut mixed = "a".repeat(MAX_TEXT_CHARS - 1);
    mixed.push('ş');
    mixed.push('🦀');
    mixed.push('z');
    let bounded_mixed = bound_text(&mixed);
    assert!(bounded_mixed.truncated);
    assert_eq!(bounded_mixed.text.chars().count(), MAX_TEXT_CHARS);
    assert!(bounded_mixed.text.ends_with('ş'));
}

#[test]
fn additional_text_none_when_blank() {
    let docs = prepare_documents("jd", "").expect("prepare empty additional");
    assert_eq!(docs.additional_text(), None);
    let docs = prepare_documents("jd", "  \n ").expect("prepare blank additional");
    assert_eq!(docs.additional_text(), None);
    let docs = prepare_documents("jd", " note ").expect("prepare with additional");
    assert_eq!(docs.additional_text(), Some("note"));
}
