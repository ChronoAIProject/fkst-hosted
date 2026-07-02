//! Tests for the collector's tree writer + the redact-before-disk choke point.
//! Split into a sibling file so `collector.rs` stays under the 500-line module cap.
//! (The full loop + git push are validated on-cluster; the pure pieces —
//! classification, tailing, redactor seeding, git sequence — are covered in their
//! own sibling test files.)

use super::*;

use std::fs;

#[test]
fn tree_writer_routes_each_class_to_its_own_tree_file() {
    let dir = tempfile::tempdir().expect("dir");
    let instance = dir.path().join("instances/inst-1");
    let mut tree = TreeWriter::new(instance.clone());

    tree.append(LogClass::HostedDriver, "driver line");
    tree.append(LogClass::Supervise, "supervise line");
    tree.append(LogClass::Codex, "codex line");
    tree.append(LogClass::Misc, "misc line");
    tree.flush_pending().expect("flush");

    assert_eq!(
        fs::read_to_string(instance.join("fkst-hosted/driver.log")).expect("driver"),
        "driver line\n"
    );
    assert_eq!(
        fs::read_to_string(instance.join("fkst-substrate/framework/supervise.log"))
            .expect("supervise"),
        "supervise line\n"
    );
    assert_eq!(
        fs::read_to_string(instance.join("fkst-substrate/codex/codex.log")).expect("codex"),
        "codex line\n"
    );
    assert_eq!(
        fs::read_to_string(instance.join("fkst-substrate/etc/misc.log")).expect("misc"),
        "misc line\n"
    );
}

#[test]
fn tree_writer_appends_across_flushes_and_tracks_pending_bytes() {
    let dir = tempfile::tempdir().expect("dir");
    let instance = dir.path().join("inst");
    let mut tree = TreeWriter::new(instance.clone());

    tree.append(LogClass::Supervise, "first");
    assert_eq!(tree.pending_bytes(), "first".len() + 1);
    tree.flush_pending().expect("flush 1");
    assert_eq!(tree.pending_bytes(), 0, "pending resets after a flush");

    tree.append(LogClass::Supervise, "second");
    tree.flush_pending().expect("flush 2");

    // The second flush APPENDED — the first line is still present.
    let content = fs::read_to_string(instance.join("fkst-substrate/framework/supervise.log"))
        .expect("supervise");
    assert_eq!(content, "first\nsecond\n");
}

#[test]
fn tree_writer_only_writes_classes_that_received_lines() {
    let dir = tempfile::tempdir().expect("dir");
    let instance = dir.path().join("inst");
    let mut tree = TreeWriter::new(instance.clone());
    tree.append(LogClass::Supervise, "only supervise");
    tree.flush_pending().expect("flush");

    assert!(instance
        .join("fkst-substrate/framework/supervise.log")
        .exists());
    // No misc line was appended → no misc file is created.
    assert!(!instance.join("fkst-substrate/etc/misc.log").exists());
}

#[test]
fn append_line_redacts_before_the_record_touches_disk() {
    let dir = tempfile::tempdir().expect("dir");
    let instance = dir.path().join("inst");
    let mut tree = TreeWriter::new(instance.clone());
    // Seed the redactor with a known secret, exactly as the collector does.
    let redactor = Redactor::new(&[("github-token", "ghs_supersecretvalue")]);

    append_line(
        &mut tree,
        &redactor,
        LogClass::Supervise,
        "cloning with token ghs_supersecretvalue now",
    );
    tree.flush_pending().expect("flush");

    let content = fs::read_to_string(instance.join("fkst-substrate/framework/supervise.log"))
        .expect("supervise");
    assert!(
        !content.contains("ghs_supersecretvalue"),
        "the raw secret must never reach disk: {content}"
    );
    assert!(
        content.contains("«REDACTED:github-token»"),
        "the masked label must be written instead: {content}"
    );
}
