//! Tests for whole-file report capture: per-report boundaries, change detection, the
//! mandatory redaction choke point, and every bound a session could try to blow.

use k8s_openapi::chrono::{DateTime, Utc};

use crate::session_health::report_filename;

use super::super::bundle::tar_gz_dir;
use super::*;

const SESSION: &str = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab";

/// A collector-shaped fixture: a runtime root holding the health dir, plus the tree
/// the bundle is assembled from.
struct Fixture {
    _dir: tempfile::TempDir,
    anchors: TreeAnchors,
    tree: TreeWriter,
    redactor: Redactor,
}

impl Fixture {
    fn new() -> Self {
        Self::with_secrets(&[])
    }

    fn with_secrets(secrets: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("dir");
        let runtime_root = dir.path().join("runtime");
        let codex_home = dir.path().join("codex");
        let anchors = TreeAnchors::new(&runtime_root, &codex_home);
        std::fs::create_dir_all(&anchors.health_dir).expect("health dir");
        Self {
            tree: TreeWriter::new(dir.path().join("tree")),
            anchors,
            redactor: Redactor::new(secrets),
            _dir: dir,
        }
    }

    /// Write a report into the session's health dir under a contract-shaped name.
    fn write_report(&self, at: &str, body: &str) -> String {
        let name = report_filename(Some("chronoai-fkst"), SESSION, stamp(at));
        std::fs::write(self.anchors.health_dir.join(&name), body).expect("write report");
        name
    }

    fn copied(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.tree.root().join(COPIED_TREE_DIR).join(name)).ok()
    }
}

fn stamp(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("fixture timestamp")
        .with_timezone(&Utc)
}

#[test]
fn a_report_is_copied_into_the_bundle_under_its_own_name() {
    let mut fixture = Fixture::new();
    let name = fixture.write_report(
        "2026-07-30T14:15:00Z",
        "+++\nstatus = \"working\"\n+++\nbody\n",
    );

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].file_name, name);
    assert_eq!(
        fixture.copied(&name).as_deref(),
        Some("+++\nstatus = \"working\"\n+++\nbody\n")
    );
}

#[test]
fn the_bundle_entry_lives_under_the_fkst_health_prefix() {
    let mut fixture = Fixture::new();
    let name = fixture.write_report("2026-07-30T14:15:00Z", "report\n");
    CopiedFileTracker::new().sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    let gz = tar_gz_dir(fixture.tree.root()).expect("bundle");
    let entries = bundle_entries(&gz);
    assert!(
        entries.contains(&format!("{COPIED_TREE_DIR}/{name}")),
        "bundle entries: {entries:?}"
    );
}

#[test]
fn two_reports_produce_two_distinct_entries_never_concatenated() {
    let mut fixture = Fixture::new();
    let first = fixture.write_report("2026-07-30T14:15:00Z", "first\n");
    let second = fixture.write_report("2026-07-30T14:25:00Z", "second\n");
    assert_ne!(first, second);

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(copied.len(), 2);
    assert_eq!(fixture.copied(&first).as_deref(), Some("first\n"));
    assert_eq!(fixture.copied(&second).as_deref(), Some("second\n"));
}

#[test]
fn an_unchanged_file_is_not_recopied_on_later_sweeps() {
    let mut fixture = Fixture::new();
    fixture.write_report("2026-07-30T14:15:00Z", "stable\n");

    let mut tracker = CopiedFileTracker::new();
    for _ in 0..5 {
        tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    }
    assert_eq!(
        tracker.copies(),
        1,
        "a 500ms poll must not re-copy an unchanged report"
    );
}

#[test]
fn a_rewritten_file_is_recopied() {
    let mut fixture = Fixture::new();
    let name = fixture.write_report("2026-07-30T14:15:00Z", "first version\n");

    let mut tracker = CopiedFileTracker::new();
    tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    assert_eq!(tracker.copies(), 1);

    std::fs::write(
        fixture.anchors.health_dir.join(&name),
        "a longer second version\n",
    )
    .expect("rewrite");
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(tracker.copies(), 2);
    assert_eq!(copied.len(), 1);
    assert_eq!(
        fixture.copied(&name).as_deref(),
        Some("a longer second version\n")
    );
}

#[test]
fn a_seeded_secret_inside_a_report_never_reaches_the_bundle() {
    // A report is authored by a codex that has read the session's logs, so this is a
    // real path, not a hypothetical one.
    let secret = "ghs_016C8fS3cReTvALuE0000000000000000000000";
    let mut fixture = Fixture::with_secrets(&[("github-token", secret)]);
    let name = fixture.write_report(
        "2026-07-30T14:15:00Z",
        &format!("+++\nheadline = \"x\"\n+++\nthe token is {secret} in prose\n"),
    );

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    let on_disk = fixture.copied(&name).expect("copied");
    assert!(!on_disk.contains(secret), "secret survived into the tree");
    assert!(
        !copied[0].redacted.contains(secret),
        "secret survived into the returned bytes"
    );
    assert!(on_disk.contains("REDACTED"), "{on_disk}");

    let gz = tar_gz_dir(fixture.tree.root()).expect("bundle");
    assert!(
        !String::from_utf8_lossy(&gz).contains(secret),
        "secret survived into the bundle"
    );
}

#[test]
fn line_structure_is_preserved_for_content_holding_no_secret() {
    // The parsed body must round-trip, so redaction may not silently reshape a clean
    // document (trailing newline, CRLF, blank lines).
    let mut fixture = Fixture::new();
    let body = "+++\nkey = 1\n+++\n\n## Title\r\n\r\nline\n\n";
    let name = fixture.write_report("2026-07-30T14:15:00Z", body);

    CopiedFileTracker::new().sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    assert_eq!(fixture.copied(&name).as_deref(), Some(body));
}

#[test]
fn a_file_over_the_size_bound_is_skipped_and_the_rest_still_copy() {
    let mut fixture = Fixture::new();
    let huge = fixture.write_report(
        "2026-07-30T14:15:00Z",
        &"x".repeat(MAX_FILE_BYTES as usize + 1),
    );
    let normal = fixture.write_report("2026-07-30T14:25:00Z", "fine\n");

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(copied.len(), 1, "only the in-bounds report");
    assert_eq!(copied[0].file_name, normal);
    assert!(fixture.copied(&huge).is_none());
    assert_eq!(fixture.copied(&normal).as_deref(), Some("fine\n"));
}

#[test]
fn an_oversized_file_is_not_re_warned_on_every_poll() {
    let mut fixture = Fixture::new();
    fixture.write_report(
        "2026-07-30T14:15:00Z",
        &"x".repeat(MAX_FILE_BYTES as usize + 1),
    );

    let mut tracker = CopiedFileTracker::new();
    for _ in 0..3 {
        tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    }
    assert_eq!(tracker.copies(), 0, "never copied, and never crashed");
}

#[test]
fn only_the_newest_reports_up_to_the_cap_are_copied() {
    let mut fixture = Fixture::new();
    let mut names = Vec::new();
    for minute in 0..(MAX_FILES + 10) {
        let at = format!("2026-07-30T{:02}:{:02}:00Z", minute / 60, minute % 60);
        names.push(fixture.write_report(&at, "r\n"));
    }
    names.sort();

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(copied.len(), MAX_FILES);
    assert!(
        fixture.copied(names.last().expect("newest")).is_some(),
        "the newest report must be copied"
    );
    assert!(
        fixture.copied(&names[0]).is_none(),
        "the oldest report falls outside the cap"
    );
}

#[test]
fn the_total_byte_budget_stops_further_copies() {
    let mut fixture = Fixture::new();
    // Many short lines, not one giant line: the budget counts REDACTED bytes, and a
    // single over-long line collapses to the redactor's overflow mask.
    let chunk = format!("{}\n", "x".repeat(31)).repeat(MAX_FILE_BYTES as usize / 32);
    let files = MAX_TOTAL_BYTES / MAX_FILE_BYTES as usize;
    for index in 0..=files {
        let at = format!("2026-07-30T{:02}:{:02}:00Z", index / 60, index % 60);
        fixture.write_report(&at, &chunk);
    }

    let mut tracker = CopiedFileTracker::new();
    tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(
        tracker.copies(),
        files,
        "the copy past the budget must be refused"
    );
}

#[test]
fn a_non_markdown_file_in_the_health_dir_is_ignored() {
    let mut fixture = Fixture::new();
    std::fs::write(fixture.anchors.health_dir.join("notes.txt"), "nope").expect("write");
    std::fs::write(fixture.anchors.health_dir.join("state.json"), "{}").expect("write");
    std::fs::create_dir_all(fixture.anchors.health_dir.join("nested")).expect("dir");

    let mut tracker = CopiedFileTracker::new();
    let copied = tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    assert!(copied.is_empty());
}

#[test]
fn a_dotfile_is_refused_even_with_a_markdown_extension() {
    // The producer writes atomically via a temp name; a hidden partial file must
    // never be captured, and the name is a traversal guard besides.
    let mut fixture = Fixture::new();
    std::fs::write(fixture.anchors.health_dir.join(".partial.md"), "half").expect("write");

    let mut tracker = CopiedFileTracker::new();
    assert!(tracker
        .sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree)
        .is_empty());
    assert_eq!(tracker.copies(), 0);
}

/// A producer legitimately keeps working files beside its reports — the health
/// reporter writes its codex context as `.fkst-health-context.md` — and the poll runs
/// twice a second. Re-deciding a skipped file without remembering it floods the log
/// bundle this module exists to fill. Observed on the real cluster: 411 identical
/// warnings in four minutes.
#[test]
fn a_repeatedly_seen_unsafe_filename_is_only_judged_once() {
    let mut fixture = Fixture::new();
    std::fs::write(
        fixture.anchors.health_dir.join(".fkst-health-context.md"),
        "codex context",
    )
    .expect("write");

    let mut tracker = CopiedFileTracker::new();
    for _ in 0..10 {
        assert!(tracker
            .sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree)
            .is_empty());
    }
    assert_eq!(tracker.judged(), 1, "judged once, not once per poll");
    assert_eq!(tracker.copies(), 0);
}

/// ...but a CHANGE re-opens the question, so a file that becomes valid is picked up.
#[test]
fn a_changed_file_is_judged_again() {
    let mut fixture = Fixture::new();
    let hidden = fixture.anchors.health_dir.join(".fkst-health-context.md");
    std::fs::write(&hidden, "v1").expect("write");

    let mut tracker = CopiedFileTracker::new();
    tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    std::fs::write(&hidden, "a longer v2").expect("rewrite");
    tracker.sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);

    assert_eq!(tracker.judged(), 2);
}

#[test]
fn an_absent_health_dir_contributes_nothing_and_does_not_fail() {
    let dir = tempfile::tempdir().expect("dir");
    let anchors = TreeAnchors::new(&dir.path().join("nope"), &dir.path().join("codex"));
    let mut tree = TreeWriter::new(dir.path().join("tree"));
    let redactor = Redactor::new(&[]);

    let mut tracker = CopiedFileTracker::new();
    assert!(tracker.sweep(&anchors, &redactor, &mut tree).is_empty());
    assert_eq!(tree.pending_bytes(), 0);
}

#[test]
fn copied_bytes_mark_the_tree_dirty_so_the_bundle_is_re_uploaded() {
    // Load-bearing: the flush cycle skips re-bundling an unchanged tree, so a report
    // that did not move `pending_bytes` would never reach chrono-storage.
    let mut fixture = Fixture::new();
    fixture.write_report("2026-07-30T14:15:00Z", "0123456789\n");

    assert_eq!(fixture.tree.pending_bytes(), 0);
    CopiedFileTracker::new().sweep(&fixture.anchors, &fixture.redactor, &mut fixture.tree);
    assert_eq!(fixture.tree.pending_bytes(), 11);
}

fn bundle_entries(gz: &[u8]) -> Vec<String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(std::io::Cursor::new(gz)));
    archive
        .entries()
        .expect("entries")
        .flatten()
        .filter(|entry| !entry.header().entry_type().is_dir())
        .filter_map(|entry| entry.path().ok().map(|path| path.display().to_string()))
        .collect()
}
