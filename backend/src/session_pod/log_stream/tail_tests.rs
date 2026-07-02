//! Tests for the incremental tail offset + line-framing tracker. Split into a
//! sibling file so `tail.rs` stays under the 500-line module cap.

use super::*;

use std::io::Write;

#[test]
fn frame_splits_complete_lines_and_holds_the_partial_tail() {
    let mut t = TailTracker::new();
    let first = t.frame("alpha\nbeta\ngam");
    assert_eq!(first, vec!["alpha", "beta"]);
    // "gam" has no newline yet — held in the carry, not emitted.
    let second = t.frame("ma\ndelta\n");
    assert_eq!(second, vec!["gamma", "delta"]);
}

#[test]
fn finish_emits_the_unterminated_tail_once() {
    let mut t = TailTracker::new();
    assert!(t.frame("no-newline-yet").is_empty());
    assert_eq!(t.finish(), Some("no-newline-yet".to_string()));
    // A second finish yields nothing (carry cleared).
    assert_eq!(t.finish(), None);
}

#[test]
fn poll_reads_only_the_newly_appended_lines() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("child.log");
    std::fs::write(&path, b"line-1\nline-2\n").expect("seed");

    let mut t = TailTracker::new();
    let first = t.poll(&path);
    assert_eq!(first, vec!["line-1", "line-2"]);
    let start_offset = t.offset();
    assert_eq!(start_offset, 14);

    // No growth → no new lines, offset unchanged.
    assert!(t.poll(&path).is_empty());
    assert_eq!(t.offset(), start_offset);

    // Append more; only the new content is emitted.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open");
    f.write_all(b"line-3\n").expect("append");
    let third = t.poll(&path);
    assert_eq!(third, vec!["line-3"]);
}

#[test]
fn poll_holds_a_partial_line_across_reads() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("child.log");
    std::fs::write(&path, b"complete\npart").expect("seed");

    let mut t = TailTracker::new();
    assert_eq!(t.poll(&path), vec!["complete"]);

    // Finish the partial line in a later write.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open");
    f.write_all(b"ial\n").expect("append");
    assert_eq!(t.poll(&path), vec!["partial"]);
}

#[test]
fn poll_restarts_after_a_truncation() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("child.log");
    std::fs::write(&path, b"aaaa\nbbbb\n").expect("seed");

    let mut t = TailTracker::new();
    assert_eq!(t.poll(&path), vec!["aaaa", "bbbb"]);

    // Truncate/rotate in place to a shorter file: the tracker must re-read from 0.
    std::fs::write(&path, b"cccc\n").expect("truncate");
    assert_eq!(t.poll(&path), vec!["cccc"]);
    assert_eq!(t.offset(), 5);
}

#[test]
fn poll_on_a_missing_file_is_a_no_op() {
    let mut t = TailTracker::new();
    assert!(t.poll(Path::new("/no/such/file.log")).is_empty());
    assert_eq!(t.offset(), 0);
}
