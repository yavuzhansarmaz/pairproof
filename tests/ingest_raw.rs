use std::fs;
use std::path::PathBuf;

use pairproof::core::{ArtifactLayout, Language};
use pairproof::ingest::{
    delete_leftover_raw_dir, find_leftover_raw_dirs, prepare_documents, store_raw_documents,
    IngestError, ADDITIONAL_FILENAME, JD_FILENAME,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pairproof-ingest-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn store_writes_jd_and_additional_with_exact_content() {
    let dir = TestDir::new("store-both");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let docs = prepare_documents("  Senior Rust role  ", "  Team notes  ").expect("prepare");
    store_raw_documents(&layout, &docs).expect("store");
    assert_eq!(
        fs::read_to_string(layout.raw_dir().join(JD_FILENAME)).expect("read jd"),
        "Senior Rust role"
    );
    assert_eq!(
        fs::read_to_string(layout.raw_dir().join(ADDITIONAL_FILENAME)).expect("read additional"),
        "Team notes"
    );
    assert_eq!(JD_FILENAME, "jd.txt");
    assert_eq!(ADDITIONAL_FILENAME, "additional.txt");
}

#[test]
fn store_skips_additional_file_when_blank() {
    let dir = TestDir::new("store-blank-additional");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let docs = prepare_documents("Only JD", "   ").expect("prepare");
    store_raw_documents(&layout, &docs).expect("store");
    assert_eq!(
        fs::read_to_string(layout.raw_dir().join(JD_FILENAME)).expect("read jd"),
        "Only JD"
    );
    assert!(!layout.raw_dir().join(ADDITIONAL_FILENAME).exists());
}

#[test]
fn store_replaces_previous_content() {
    let dir = TestDir::new("store-replaces");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let first = prepare_documents("first jd longer content", "first notes").expect("prepare 1");
    store_raw_documents(&layout, &first).expect("store 1");
    let second = prepare_documents("x", "y").expect("prepare 2");
    store_raw_documents(&layout, &second).expect("store 2");
    assert_eq!(
        fs::read_to_string(layout.raw_dir().join(JD_FILENAME)).expect("read jd"),
        "x"
    );
    assert_eq!(
        fs::read_to_string(layout.raw_dir().join(ADDITIONAL_FILENAME)).expect("read additional"),
        "y"
    );
}

#[cfg(unix)]
#[test]
fn store_rejects_symlinked_raw_dir() {
    let dir = TestDir::new("store-symlink-raw");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let target = dir.path("elsewhere");
    fs::create_dir_all(&target).expect("create target");
    fs::remove_dir(layout.raw_dir()).expect("remove raw dir");
    std::os::unix::fs::symlink(&target, layout.raw_dir()).expect("symlink raw");
    let docs = prepare_documents("JD text", "notes").expect("prepare");
    let err = store_raw_documents(&layout, &docs).expect_err("symlinked raw must fail");
    assert!(
        matches!(err, IngestError::RawDirIsSymlink(ref p) if p == layout.raw_dir()),
        "got {err:?}"
    );
    assert!(!target.join(JD_FILENAME).exists());
    assert!(!target.join(ADDITIONAL_FILENAME).exists());
}

#[test]
fn stored_raw_documents_removed_by_delete_raw_dir() {
    let dir = TestDir::new("stored-removed");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let docs = prepare_documents("JD to delete", "notes to delete").expect("prepare");
    store_raw_documents(&layout, &docs).expect("store");
    assert!(layout.raw_dir().join(JD_FILENAME).exists());
    layout.delete_raw_dir().expect("delete raw");
    assert!(!layout.raw_dir().exists());
    assert!(layout.session_dir().is_dir());
}

#[test]
fn find_leftover_lists_only_sessions_with_raw_documents() {
    let dir = TestDir::new("find-leftover");
    let root = dir.path("sessions");
    // s-b with JD, s-a with additional only: both leftover, sorted.
    let raw_b = root.join("s-b").join("raw");
    fs::create_dir_all(&raw_b).expect("create raw b");
    fs::write(raw_b.join(JD_FILENAME), "jd").expect("write jd b");
    let raw_a = root.join("s-a").join("raw");
    fs::create_dir_all(&raw_a).expect("create raw a");
    fs::write(raw_a.join(ADDITIONAL_FILENAME), "notes").expect("write additional a");
    // Empty raw dir ignored.
    let raw_empty = root.join("s-empty").join("raw");
    fs::create_dir_all(&raw_empty).expect("create empty raw");
    // Raw with unrelated file ignored.
    let raw_other = root.join("s-other").join("raw");
    fs::create_dir_all(&raw_other).expect("create other raw");
    fs::write(raw_other.join("other.txt"), "x").expect("write other");
    // Session without raw ignored.
    fs::create_dir_all(root.join("s-noraw")).expect("create session without raw");
    // Plain file directly under root ignored.
    fs::write(root.join("stray.txt"), "x").expect("write stray");

    let found = find_leftover_raw_dirs(&root).expect("find leftovers");
    assert_eq!(found, vec![raw_a, raw_b]);
    // Read-only: nothing removed.
    assert!(found.iter().all(|p| p.is_dir()));
}

#[test]
fn find_leftover_missing_root_is_empty() {
    let dir = TestDir::new("find-missing");
    let missing = dir.path("does-not-exist");
    let found = find_leftover_raw_dirs(&missing).expect("missing root ok");
    assert!(found.is_empty());
}

#[cfg(unix)]
#[test]
fn find_leftover_skips_symlinked_session() {
    let dir = TestDir::new("find-symlink-session");
    let root = dir.path("sessions");
    fs::create_dir_all(&root).expect("create root");
    let outside = dir.path("outside-session");
    let outside_raw = outside.join("raw");
    fs::create_dir_all(&outside_raw).expect("create outside raw");
    fs::write(outside_raw.join(JD_FILENAME), "jd").expect("write outside jd");
    std::os::unix::fs::symlink(&outside, root.join("s-link")).expect("symlink session");
    // A real session alongside the symlink is still found.
    let real_raw = root.join("s-real").join("raw");
    fs::create_dir_all(&real_raw).expect("create real raw");
    fs::write(real_raw.join(JD_FILENAME), "jd").expect("write real jd");

    let found = find_leftover_raw_dirs(&root).expect("find leftovers");
    assert_eq!(found, vec![real_raw]);
}

#[test]
fn delete_leftover_removes_raw_inside_root() {
    let dir = TestDir::new("delete-inside");
    let root = dir.path("sessions");
    let raw = root.join("s1").join("raw");
    fs::create_dir_all(&raw).expect("create raw");
    fs::write(raw.join(JD_FILENAME), "jd").expect("write jd");
    fs::write(root.join("s1").join("report.md"), "keep").expect("write report");
    delete_leftover_raw_dir(&root, &raw).expect("delete leftover");
    assert!(!raw.exists());
    assert_eq!(
        fs::read_to_string(root.join("s1").join("report.md")).expect("report intact"),
        "keep"
    );
}

#[test]
fn delete_leftover_rejects_outside_root() {
    let dir = TestDir::new("delete-outside");
    let root = dir.path("sessions");
    fs::create_dir_all(&root).expect("create root");

    // Sibling of root.
    let sibling_raw = dir.path("sibling").join("raw");
    fs::create_dir_all(&sibling_raw).expect("create sibling raw");
    fs::write(sibling_raw.join(JD_FILENAME), "jd").expect("write sibling jd");
    let err = delete_leftover_raw_dir(&root, &sibling_raw).expect_err("sibling must fail");
    assert!(
        matches!(err, IngestError::OutsideRoot(ref p) if p == &sibling_raw),
        "got {err:?}"
    );
    assert!(sibling_raw.join(JD_FILENAME).exists());

    // Root itself: rejected (not named raw) and nothing deleted.
    let err = delete_leftover_raw_dir(&root, &root).expect_err("root itself must fail");
    assert!(matches!(err, IngestError::NotARawDir(_)), "got {err:?}");
    assert!(root.is_dir());

    // The root itself named raw elsewhere is covered by name check order:
    // root itself has file_name "sessions", so use root/raw whose parent
    // is the root (session component missing one level) as outside-shaped,
    // plus the literal root passed as raw_dir.
    let root_raw = root.join("raw");
    fs::create_dir_all(&root_raw).expect("create root raw");
    fs::write(root_raw.join(JD_FILENAME), "jd").expect("write root jd");
    let err = delete_leftover_raw_dir(&root, &root_raw).expect_err("root/raw must fail");
    assert!(
        matches!(err, IngestError::OutsideRoot(_)),
        "root/raw got {err:?}"
    );
    assert!(root_raw.join(JD_FILENAME).exists());

    // Nested too deep: root/<session>/sub/raw.
    let deep_raw = root.join("s1").join("sub").join("raw");
    fs::create_dir_all(&deep_raw).expect("create deep raw");
    fs::write(deep_raw.join(JD_FILENAME), "jd").expect("write deep jd");
    let err = delete_leftover_raw_dir(&root, &deep_raw).expect_err("deep path must fail");
    assert!(
        matches!(err, IngestError::OutsideRoot(ref p) if p == &deep_raw),
        "got {err:?}"
    );
    assert!(deep_raw.join(JD_FILENAME).exists());
}

#[test]
fn delete_leftover_rejects_non_raw_name() {
    let dir = TestDir::new("delete-non-raw");
    let root = dir.path("sessions");
    let not_raw = root.join("s1").join("solutions");
    fs::create_dir_all(&not_raw).expect("create solutions");
    fs::write(not_raw.join("01.py"), "keep").expect("write solution");
    let err = delete_leftover_raw_dir(&root, &not_raw).expect_err("non-raw name must fail");
    assert!(
        matches!(err, IngestError::NotARawDir(ref p) if p == &not_raw),
        "got {err:?}"
    );
    assert!(not_raw.join("01.py").exists());

    // The sessions root itself is not a raw dir.
    let err = delete_leftover_raw_dir(&root, &root).expect_err("root itself must fail");
    assert!(matches!(err, IngestError::NotARawDir(_)), "got {err:?}");
    assert!(root.is_dir());
}

#[test]
fn delete_leftover_missing_raw_is_ok() {
    let dir = TestDir::new("delete-missing");
    let root = dir.path("sessions");
    fs::create_dir_all(root.join("s1")).expect("create session");
    let missing_raw = root.join("s1").join("raw");
    delete_leftover_raw_dir(&root, &missing_raw).expect("missing raw ok");
    // Also ok when the whole session dir is missing.
    let absent = root.join("ghost").join("raw");
    delete_leftover_raw_dir(&root, &absent).expect("absent session raw ok");
}

#[cfg(unix)]
#[test]
fn store_rejects_symlinked_jd_file() {
    let dir = TestDir::new("store-symlink-jd-file");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let victim = dir.path("victim.txt");
    fs::write(&victim, "ORIGINAL").expect("write victim");
    let jd_path = layout.raw_dir().join(JD_FILENAME);
    std::os::unix::fs::symlink(&victim, &jd_path).expect("symlink jd");
    let docs = prepare_documents("JD text", "notes").expect("prepare");
    let err = store_raw_documents(&layout, &docs).expect_err("symlinked jd must fail");
    assert!(
        matches!(err, IngestError::RawFileIsSymlink(ref p) if p == &jd_path),
        "got {err:?}"
    );
    assert_eq!(
        fs::read_to_string(&victim).expect("read victim"),
        "ORIGINAL"
    );
    assert!(fs::symlink_metadata(layout.raw_dir().join(ADDITIONAL_FILENAME)).is_err());
    assert!(!dir.path(ADDITIONAL_FILENAME).exists());
}

#[cfg(unix)]
#[test]
fn store_rejects_symlinked_additional_file() {
    let dir = TestDir::new("store-symlink-additional-file");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let victim = dir.path("victim.txt");
    fs::write(&victim, "ORIGINAL").expect("write victim");
    let additional_path = layout.raw_dir().join(ADDITIONAL_FILENAME);
    std::os::unix::fs::symlink(&victim, &additional_path).expect("symlink additional");
    let docs = prepare_documents("JD text", "notes").expect("prepare");
    let err = store_raw_documents(&layout, &docs).expect_err("symlinked additional must fail");
    assert!(
        matches!(err, IngestError::RawFileIsSymlink(ref p) if p == &additional_path),
        "got {err:?}"
    );
    assert_eq!(
        fs::read_to_string(&victim).expect("read victim"),
        "ORIGINAL"
    );
    assert!(fs::symlink_metadata(layout.raw_dir().join(JD_FILENAME)).is_err());
}

#[cfg(unix)]
#[test]
fn delete_leftover_rejects_symlinked_session() {
    let dir = TestDir::new("delete-symlink-session");
    let root = dir.path("sessions");
    fs::create_dir_all(&root).expect("create root");
    let outside = dir.path("outside-session");
    let outside_raw = outside.join("raw");
    fs::create_dir_all(&outside_raw).expect("create outside raw");
    fs::write(outside_raw.join(JD_FILENAME), "keep me").expect("write outside jd");
    std::os::unix::fs::symlink(&outside, root.join("s-link")).expect("symlink session");
    let raw = root.join("s-link").join("raw");
    let err = delete_leftover_raw_dir(&root, &raw).expect_err("symlinked session must fail");
    assert!(
        matches!(err, IngestError::OutsideRoot(ref p) if p == &raw),
        "got {err:?}"
    );
    assert_eq!(
        fs::read_to_string(outside_raw.join(JD_FILENAME)).expect("outside intact"),
        "keep me"
    );
}

#[test]
fn find_leftover_file_root_is_error() {
    let dir = TestDir::new("find-file-root");
    let root = dir.path("sessions");
    fs::write(&root, "not a directory").expect("write file root");
    let err = find_leftover_raw_dirs(&root).expect_err("file root must fail");
    assert!(matches!(err, IngestError::Io(_)), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn delete_leftover_symlinked_raw_removes_link_only() {
    let dir = TestDir::new("delete-symlink-raw");
    let root = dir.path("sessions");
    let session = root.join("s1");
    fs::create_dir_all(&session).expect("create session");
    let target = dir.path("target-raw");
    fs::create_dir_all(&target).expect("create target");
    fs::write(target.join(JD_FILENAME), "keep me").expect("write target jd");
    let raw = session.join("raw");
    std::os::unix::fs::symlink(&target, &raw).expect("symlink raw");
    delete_leftover_raw_dir(&root, &raw).expect("delete symlinked raw");
    assert!(fs::symlink_metadata(&raw).is_err(), "link removed");
    assert_eq!(
        fs::read_to_string(target.join(JD_FILENAME)).expect("target intact"),
        "keep me"
    );
}
