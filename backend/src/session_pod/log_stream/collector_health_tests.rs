//! Collector-level tests for the health-report path: capture into the bundle through
//! the real `collect` loop, and publication of the small objects beside it. Split from
//! `collector_tests.rs` so both stay under the 500-line module cap.

use super::*;

use std::fs;
use std::io::Read;
use std::sync::mpsc::sync_channel;

use flate2::read::GzDecoder;

use crate::session_health::{health_index_key, parse_index};
use crate::session_pod::log_stream::sink::FakeSink;

/// A collector config rooted at `dir` whose creds seed a known secret, mirroring
/// `collector_tests::collector_config`.
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

fn fake_uploader(fake: FakeSink, session_id: &str) -> Uploader {
    Uploader::new(
        Box::new(fake),
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime"),
        session_id,
        "inst-test".to_string(),
        Utc::now(),
    )
}

fn extract(gz: &[u8]) -> Vec<(String, String)> {
    let mut archive = tar::Archive::new(GzDecoder::new(std::io::Cursor::new(gz)));
    let mut out = Vec::new();
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = entry.path().expect("path").display().to_string();
        let mut content = String::new();
        entry.read_to_string(&mut content).ok();
        out.push((path, content));
    }
    out
}

/// Write a contract-shaped report into the session's health dir.
fn write_report(config: &CollectorConfig, stamp: &str, body: &str) -> String {
    let health_dir = config
        .runtime_root
        .join(crate::session_health::HEALTH_DIR_NAME);
    fs::create_dir_all(&health_dir).expect("health dir");
    let name = format!(
        "chronoai-fkst-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-{stamp}.md"
    );
    fs::write(health_dir.join(&name), body).expect("report");
    name
}

fn valid_report(generated_at: &str, status: &str) -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"8f2c1d64-0a1b-4c2d-8e3f-0123456789ab\"\n\
         producer = \"fkst-health@0.1.0\"\n\
         generated_at = \"{generated_at}\"\n\
         status = \"{status}\"\n\
         headline = \"a headline\"\n\
         +++\n## Narrative\n\nprose\n"
    )
}

/// A report written after the last periodic poll — the likeliest moment for one,
/// since the producer and the pod terminate together — must still reach the bundle
/// via the shutdown drain.
#[test]
fn the_shutdown_drain_captures_a_report_written_after_the_last_poll() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-health", 1);
    let health_dir = config
        .runtime_root
        .join(crate::session_health::HEALTH_DIR_NAME);
    fs::create_dir_all(&health_dir).expect("health dir");
    let name = "chronoai-fkst-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260730-141500.md";
    fs::write(
        health_dir.join(name),
        "+++\nfkst_health_report = 1\n+++\nnarrative\n",
    )
    .expect("report");

    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-health");
    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    drop(tx); // straight to the drain: no poll tick ever runs

    collect(config, rx, Some(uploader));

    let latest = fake
        .stored("logs/sess-health/latest.tar.gz")
        .expect("a bundle was uploaded");
    let entries = extract(&latest);
    let report = entries
        .iter()
        .find(|(path, _)| path == &format!("fkst-health/{name}"))
        .map(|(_, content)| content.as_str())
        .unwrap_or_else(|| {
            panic!(
                "report missing from the bundle: {:?}",
                entries.iter().map(|(p, _)| p).collect::<Vec<_>>()
            )
        });
    assert_eq!(report, "+++\nfkst_health_report = 1\n+++\nnarrative\n");
}

/// A secret quoted inside a report must not survive the collector, end to end.
#[test]
fn a_secret_inside_a_report_is_redacted_before_the_bundle_leaves_the_pod() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-secret", 1);
    let health_dir = config
        .runtime_root
        .join(crate::session_health::HEALTH_DIR_NAME);
    fs::create_dir_all(&health_dir).expect("health dir");
    fs::write(
        health_dir.join(
            "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260730-141500.md",
        ),
        "the codex quoted ghs_supersecretvalue in its narrative\n",
    )
    .expect("report");

    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-secret");
    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    drop(tx);

    collect(config, rx, Some(uploader));

    let latest = fake
        .stored("logs/sess-secret/latest.tar.gz")
        .expect("a bundle was uploaded");
    assert!(
        !String::from_utf8_lossy(&latest).contains("ghs_supersecretvalue"),
        "the raw secret must never reach the archive"
    );
    let entries = extract(&latest);
    let report = entries
        .iter()
        .find(|(path, _)| path.starts_with("fkst-health/"))
        .map(|(_, content)| content.as_str())
        .expect("the report is in the bundle");
    assert!(report.contains("«REDACTED:github-token»"), "{report}");
}

/// The whole in-pod path in one run: a report lands in the bundle AND as its own
/// small object AND in the index, without disturbing the log objects.
#[test]
fn a_report_reaches_the_bundle_the_object_store_and_the_index() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-e2e", 1);
    let name = write_report(
        &config,
        "20260730-141500",
        &valid_report("2026-07-30T14:15:00Z", "stalled"),
    );

    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-e2e");
    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    drop(tx);

    collect(config, rx, Some(uploader));

    // 1. In the bundle, under its own name.
    let latest = fake
        .stored("logs/sess-e2e/latest.tar.gz")
        .expect("bundle uploaded");
    assert!(extract(&latest)
        .iter()
        .any(|(path, _)| path == &format!("fkst-health/{name}")));

    // 2. As its own small object.
    assert!(fake.stored(&format!("health/sess-e2e/{name}")).is_some());

    // 3. In the index, denormalized.
    let reports = parse_index(&fake.stored(&health_index_key("sess-e2e")).expect("index"));
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "stalled");
    assert_eq!(reports[0].headline, "a headline");
}

/// Regression: the log objects must be untouched by the health path.
#[test]
fn the_log_objects_are_unchanged_by_health_publication() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-reg", 1);
    write_report(
        &config,
        "20260730-141500",
        &valid_report("2026-07-30T14:15:00Z", "working"),
    );

    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-reg");
    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    tx.send((LogClass::Supervise, "a supervise line".to_string()))
        .expect("send");
    drop(tx);

    collect(config, rx, Some(uploader));

    assert!(fake.stored("logs/sess-reg/latest.tar.gz").is_some());
    assert!(fake.stored("logs/sess-reg/runs/inst-test.tar.gz").is_some());
    assert!(fake.stored("logs/sess-reg/runs.json").is_some());
    let log_keys: Vec<String> = fake
        .calls()
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("logs/"))
        .collect();
    assert!(
        log_keys
            .iter()
            .all(|key| key == "logs/sess-reg/latest.tar.gz"
                || key == "logs/sess-reg/runs/inst-test.tar.gz"
                || key == "logs/sess-reg/runs.json"),
        "unexpected log-side key: {log_keys:?}"
    );
}

/// A session whose reports never parse still gets its logs, and the health index is
/// simply never created — the fleet-wide fail-safe posture.
#[test]
fn an_unparseable_report_leaves_the_index_absent_and_the_session_unharmed() {
    let dir = tempfile::tempdir().expect("dir");
    let config = collector_config(dir.path(), "sess-bad", 1);
    let name = write_report(&config, "20260730-141500", "## not a report\n");

    let fake = FakeSink::default();
    let uploader = fake_uploader(fake.clone(), "sess-bad");
    let (tx, rx) = sync_channel::<CollectorRecord>(64);
    drop(tx);

    collect(config, rx, Some(uploader));

    assert!(fake.stored(&health_index_key("sess-bad")).is_none());
    assert!(fake.stored(&format!("health/sess-bad/{name}")).is_none());
    // It still rode into the bundle, so nothing was silently lost.
    let latest = fake.stored("logs/sess-bad/latest.tar.gz").expect("bundle");
    assert!(extract(&latest)
        .iter()
        .any(|(path, _)| path == &format!("fkst-health/{name}")));
}
