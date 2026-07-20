//! Tests for the collector's tree writer, the redact-before-disk choke point, and
//! the flush → tar.gz → sink upload path (driven with a fake sink + a deterministic
//! size-based flush, so no real clock/network is involved). Split into a sibling
//! file so `collector.rs` stays under the 500-line module cap.

use super::*;

use std::fs;
use std::io::Read;
use std::sync::mpsc::sync_channel;

use flate2::read::GzDecoder;

use crate::session_pod::log_stream::runs;
use crate::session_pod::log_stream::sink::FakeSink;

/// The run id every fake uploader in these tests writes under (matches the
/// `collector_config` instance id).
const TEST_RUN_ID: &str = "inst-test";

/// A minimal collector config rooted at `dir`, with the flush thresholds the caller
/// wants. `creds_dir` holds a `github-token` so the redactor seeds a known secret.
fn collector_config(
    dir: &std::path::Path,
    session_id: &str,
    flush_bytes: usize,
) -> CollectorConfig {
    let creds_dir = dir.join("creds");
    fs::create_dir_all(&creds_dir).expect("creds dir");
    fs::write(creds_dir.join("github-token"), "ghs_supersecretvalue").expect("token");
    CollectorConfig {
        instance_id: "inst-test".to_string(),
        session_id: session_id.to_string(),
        trigger_issue: 7,
        repo: "acme/site".to_string(),
        engine_ref: "main".to_string(),
        config_hash: "cfg-1".to_string(),
        pod_uid: "pod-uid".to_string(),
        start_time: Utc::now(),
        runtime_root: dir.join("runtime"),
        codex_home: dir.join("codex"),
        creds_dir,
        tree_dir: dir.join("tree"),
        flush_secs: 3600,
        flush_bytes,
        channel_capacity: 64,
    }
}

/// An uploader wrapping a fake sink + a dedicated current-thread runtime (the
/// collector runs on a non-tokio thread, so it owns its own runtime). Uses
/// [`TEST_RUN_ID`] as the run id, so the per-run object + index entry are
/// deterministic.
fn fake_uploader(fake: FakeSink, session_id: &str) -> Uploader {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    Uploader::new(
        Box::new(fake),
        runtime,
        session_id,
        TEST_RUN_ID.to_string(),
        Utc::now(),
    )
}

/// Decode a `tar.gz` into `(path, contents)` pairs (leading `./` trimmed).
fn extract(gz: &[u8]) -> Vec<(String, String)> {
    let mut archive = tar::Archive::new(GzDecoder::new(gz));
    let mut out = Vec::new();
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .expect("path")
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        let mut contents = String::new();
        entry.read_to_string(&mut contents).expect("read");
        out.push((path, contents));
    }
    out
}

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

#[test]
fn collector_uploads_a_redacted_bundle_on_the_size_cadence() {
    let dir = tempfile::tempdir().expect("dir");
    // flush_bytes=1 so a single record trips the size-based flush immediately —
    // deterministic, no dependence on the wall clock.
    let config = collector_config(dir.path(), "sess-1", 1);
    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-1");

    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    tx.send((
        LogClass::Supervise,
        "cloning with token ghs_supersecretvalue now".to_string(),
    ))
    .expect("send");
    drop(tx); // disconnect so the loop drains, does a final flush, and exits

    collect(config, rx, Some(uploader));

    let calls = fake.calls();
    assert!(
        !calls.is_empty(),
        "the collector uploaded at least one bundle"
    );
    // The bundle is PUT to BOTH the authoritative latest object (unchanged legacy
    // path) AND this run's per-incarnation object.
    assert!(
        calls
            .iter()
            .any(|(key, _)| key == "logs/sess-1/latest.tar.gz"),
        "latest.tar.gz uploaded"
    );
    assert!(
        calls
            .iter()
            .any(|(key, _)| key == "logs/sess-1/runs/inst-test.tar.gz"),
        "per-run object uploaded"
    );
    // Decode the last LATEST bundle: it is a valid gzip, the secret was redacted
    // BEFORE it reached the archive, and the masked label survived.
    let latest = calls
        .iter()
        .rev()
        .find(|(key, _)| key == "logs/sess-1/latest.tar.gz")
        .map(|(_, gz)| gz.clone())
        .expect("a latest bundle");
    let entries = extract(&latest);
    let supervise = entries
        .iter()
        .find(|(p, _)| p.ends_with("fkst-substrate/framework/supervise.log"))
        .map(|(_, c)| c.as_str())
        .expect("supervise.log in the bundle");
    assert!(
        !supervise.contains("ghs_supersecretvalue"),
        "the raw secret must never reach the archive: {supervise}"
    );
    assert!(
        supervise.contains("«REDACTED:github-token»"),
        "the masked label must survive into the archive: {supervise}"
    );
    // The self-describing baseline rode along too.
    assert!(entries.iter().any(|(p, _)| p.ends_with("meta.json")));
    assert!(entries.iter().any(|(p, _)| p.ends_with("README.md")));

    // The run was registered in the session's run index on the first upload, and
    // stamped with an end time on the final (shutdown) flush.
    let index = fake
        .stored("logs/sess-1/runs.json")
        .expect("runs.json written to the index");
    let runs = runs::parse_runs(&index);
    assert_eq!(runs.len(), 1, "exactly one run in the index");
    assert_eq!(runs[0].run_id, "inst-test");
    assert!(!runs[0].started_at.is_empty(), "run carries a start time");
    assert!(
        runs[0].ended_at.is_some(),
        "the run was finalized with an end time at shutdown"
    );
}

#[test]
fn collector_uploads_the_self_describing_baseline_on_the_final_flush() {
    let dir = tempfile::tempdir().expect("dir");
    // A big flush_bytes + no records: only the final (shutdown) flush fires, and it
    // uploads the baseline (meta.json + README) exactly once.
    let config = collector_config(dir.path(), "sess-2", 1_000_000);
    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-2");

    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    drop(tx); // no records; disconnect immediately

    collect(config, rx, Some(uploader));

    let calls = fake.calls();
    // Exactly one LATEST upload of the baseline (unchanged legacy cadence), plus the
    // additive per-run object.
    let latest_calls: Vec<_> = calls
        .iter()
        .filter(|(key, _)| key == "logs/sess-2/latest.tar.gz")
        .collect();
    assert_eq!(
        latest_calls.len(),
        1,
        "exactly one (final) latest upload of the baseline"
    );
    assert!(
        calls
            .iter()
            .any(|(key, _)| key == "logs/sess-2/runs/inst-test.tar.gz"),
        "per-run baseline object uploaded"
    );
    let entries = extract(&latest_calls[0].1);
    let meta = entries
        .iter()
        .find(|(p, _)| p.ends_with("meta.json"))
        .map(|(_, c)| c.as_str())
        .expect("meta.json");
    assert!(
        meta.contains("\"repo\": \"acme/site\""),
        "meta shape: {meta}"
    );
    // The run index was written even for a no-records session (the baseline still
    // constitutes a run), and finalized with an end time.
    let index = fake
        .stored("logs/sess-2/runs.json")
        .expect("runs.json written to the index");
    let runs = runs::parse_runs(&index);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "inst-test");
    assert!(runs[0].ended_at.is_some());
}

#[test]
fn collector_swallows_a_failed_index_add_and_recovers_the_run_at_shutdown() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-recover", 1);
    // The run-INDEX ops fail TRANSIENTLY: the first matching op (the one-shot
    // add's index read) fails — modelling a transient index outage — then clears.
    // So the bundle PUTs still succeed, the collector never panics, and shutdown's
    // finalize UPSERTS the run the failed add dropped (FIX 1's recovery path).
    let fake = FakeSink {
        fail_key_contains: Some("runs.json".to_string()),
        fail_key_remaining: std::sync::Arc::new(std::sync::Mutex::new(Some(1))),
        ..Default::default()
    };
    let uploader = fake_uploader(fake.clone(), "sess-recover");

    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    tx.send((LogClass::Supervise, "a line".to_string()))
        .expect("send");
    drop(tx); // disconnect so the loop drains, does a final flush, and exits

    // (a) Must not panic, and BOTH bundle objects were still put despite the
    // index-add failure.
    collect(config, rx, Some(uploader));
    let calls = fake.calls();
    assert!(
        calls
            .iter()
            .any(|(key, _)| key == "logs/sess-recover/latest.tar.gz"),
        "latest.tar.gz uploaded despite the index failure"
    );
    assert!(
        calls
            .iter()
            .any(|(key, _)| key == "logs/sess-recover/runs/inst-test.tar.gz"),
        "per-run object uploaded despite the index failure"
    );

    // (b) The failed add dropped the run, but shutdown's finalize recovered it —
    // the index ends up containing the run WITH an end time (start time intact).
    let index = fake
        .stored("logs/sess-recover/runs.json")
        .expect("the dropped run was recovered into the index at shutdown");
    let runs = runs::parse_runs(&index);
    assert_eq!(runs.len(), 1, "the dropped run was recovered exactly once");
    assert_eq!(runs[0].run_id, "inst-test");
    assert!(
        !runs[0].started_at.is_empty(),
        "the recovered run keeps its start time"
    );
    assert!(
        runs[0].ended_at.is_some(),
        "the recovered run carries an end time"
    );
}

#[test]
fn collector_without_an_uploader_still_captures_to_disk_and_never_crashes() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-3", 1);
    let tree_dir = config.tree_dir.clone();

    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    tx.send((LogClass::HostedDriver, "driver up".to_string()))
        .expect("send");
    drop(tx);

    // No uploader (the fail-closed path): capture proceeds, nothing is uploaded.
    collect(config, rx, None);

    let driver_log = fs::read_to_string(tree_dir.join("fkst-hosted/driver.log")).expect("driver");
    assert!(driver_log.contains("driver up"));
}
