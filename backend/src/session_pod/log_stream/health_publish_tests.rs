//! Tests for report publication: the object-then-index order, idempotency, the
//! retry-on-transport-failure path, and every way a publish must degrade quietly.

use axum::body::Bytes;

use crate::session_health::{health_index_key, parse_index};

use super::super::sink::FakeSink;
use super::super::uploader::Uploader;
use super::*;

const SESSION: &str = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab";

fn report_name(stamp: &str) -> String {
    format!("chronoai-fkst-{SESSION}-health-agent-status-report-{stamp}.md")
}

fn report_body(generated_at: &str, status: &str) -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION}\"\n\
         producer = \"fkst-health@0.1.0\"\n\
         generated_at = \"{generated_at}\"\n\
         expected_interval_secs = 600\n\
         status = \"{status}\"\n\
         headline = \"a headline\"\n\
         +++\n## Narrative\n\nsome prose\n"
    )
}

fn copied(stamp: &str, generated_at: &str, status: &str) -> CopiedFile {
    CopiedFile {
        file_name: report_name(stamp),
        redacted: report_body(generated_at, status),
    }
}

fn uploader(fake: FakeSink) -> Uploader {
    Uploader::new(
        Box::new(fake),
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime"),
        SESSION,
        "inst-test".to_string(),
        k8s_openapi::chrono::Utc::now(),
    )
}

fn drain(queue: &mut HealthPublishQueue, fake: &FakeSink) {
    queue.publish_pending(Some(&uploader(fake.clone())), SESSION, &Redactor::new(&[]));
}

#[test]
fn a_report_is_published_as_its_own_object_with_the_redacted_bytes() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    let file = copied("20260730-141500", "2026-07-30T14:15:00Z", "stalled");
    queue.enqueue(vec![file.clone()], &Redactor::new(&[]));
    drain(&mut queue, &fake);

    let key = format!("health/{SESSION}/{}", file.file_name);
    assert_eq!(
        fake.stored(&key),
        Some(Bytes::from(file.redacted.clone())),
        "the bytes published are the same redacted bytes that went into the bundle"
    );
    assert_eq!(
        fake.content_type(&key).as_deref(),
        Some("text/markdown; charset=utf-8")
    );
}

#[test]
fn the_index_gains_a_matching_denormalized_entry() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![copied("20260730-141500", "2026-07-30T14:15:00Z", "stalled")],
        &Redactor::new(&[]),
    );
    drain(&mut queue, &fake);

    let index = fake
        .stored(&health_index_key(SESSION))
        .expect("index written");
    let reports = parse_index(&index);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "stalled");
    assert_eq!(reports[0].headline, "a headline");
    assert_eq!(reports[0].generated_at, "2026-07-30T14:15:00Z");
    assert_eq!(reports[0].expected_interval_secs, 600);
    assert_eq!(reports[0].producer, "fkst-health@0.1.0");
    assert_eq!(
        fake.content_type(&health_index_key(SESSION)).as_deref(),
        Some("application/json")
    );
}

#[test]
fn the_object_is_written_before_the_index_that_advertises_it() {
    // Reversed, the read API would validate an id against the index and then 404 on
    // the object it just advertised.
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    let file = copied("20260730-141500", "2026-07-30T14:15:00Z", "working");
    queue.enqueue(vec![file.clone()], &Redactor::new(&[]));
    drain(&mut queue, &fake);

    let keys: Vec<String> = fake.calls().into_iter().map(|(key, _)| key).collect();
    let object_at = keys
        .iter()
        .position(|key| key.ends_with(&file.file_name))
        .expect("object put");
    let index_at = keys
        .iter()
        .position(|key| key == &health_index_key(SESSION))
        .expect("index put");
    assert!(object_at < index_at, "{keys:?}");
}

#[test]
fn entries_are_newest_first_and_republishing_is_idempotent() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    let older = copied("20260730-140500", "2026-07-30T14:05:00Z", "working");
    let newer = copied("20260730-141500", "2026-07-30T14:15:00Z", "stalled");

    queue.enqueue(vec![older.clone(), newer.clone()], &Redactor::new(&[]));
    drain(&mut queue, &fake);
    // The same reports again — a re-copy after an mtime change, say.
    queue.enqueue(vec![older, newer], &Redactor::new(&[]));
    drain(&mut queue, &fake);

    let reports = parse_index(&fake.stored(&health_index_key(SESSION)).expect("index"));
    assert_eq!(reports.len(), 2, "no duplicate entries");
    assert_eq!(reports[0].generated_at, "2026-07-30T14:15:00Z");
    assert_eq!(reports[1].generated_at, "2026-07-30T14:05:00Z");
}

#[test]
fn an_unparseable_report_is_not_published_and_is_not_retried() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![CopiedFile {
            file_name: report_name("20260730-141500"),
            redacted: "## no front matter at all\n".to_string(),
        }],
        &Redactor::new(&[]),
    );
    drain(&mut queue, &fake);

    assert!(fake.calls().is_empty(), "nothing was published");
    assert!(
        fake.stored(&health_index_key(SESSION)).is_none(),
        "the index was not corrupted"
    );
    assert_eq!(queue.pending(), 0, "it will never parse; do not retry it");
}

#[test]
fn a_markdown_file_that_is_not_a_contract_report_name_is_bundled_but_not_published() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![CopiedFile {
            file_name: "scratch-notes.md".to_string(),
            redacted: report_body("2026-07-30T14:15:00Z", "working"),
        }],
        &Redactor::new(&[]),
    );
    drain(&mut queue, &fake);

    assert!(fake.calls().is_empty());
    assert_eq!(queue.pending(), 0);
}

#[test]
fn a_transport_failure_is_swallowed_and_retried_on_the_next_cycle() {
    // Fail the first attempt only, then let it through — the outage-then-recovery
    // shape the run index already models.
    let fake = FakeSink {
        fail_key_contains: Some("health/".to_string()),
        fail_key_remaining: std::sync::Arc::new(std::sync::Mutex::new(Some(1))),
        ..FakeSink::default()
    };
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![copied("20260730-141500", "2026-07-30T14:15:00Z", "working")],
        &Redactor::new(&[]),
    );

    drain(&mut queue, &fake);
    assert_eq!(queue.pending(), 1, "held for retry, not dropped");

    drain(&mut queue, &fake);
    assert_eq!(queue.pending(), 0, "published on the retry");
    let reports = parse_index(&fake.stored(&health_index_key(SESSION)).expect("index"));
    assert_eq!(reports.len(), 1);
}

#[test]
fn with_no_storage_configured_nothing_is_published_and_nothing_accumulates() {
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![copied("20260730-141500", "2026-07-30T14:15:00Z", "working")],
        &Redactor::new(&[]),
    );
    queue.publish_pending(None, SESSION, &Redactor::new(&[]));
    assert_eq!(
        queue.pending(),
        0,
        "no destination exists, so the queue must not grow for the pod's whole life"
    );
}

#[test]
fn a_corrupt_existing_index_does_not_stop_the_new_entry_landing() {
    let fake = FakeSink::default();
    fake.store.lock().expect("lock").insert(
        health_index_key(SESSION),
        Bytes::from_static(b"{ truncated"),
    );

    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![copied("20260730-141500", "2026-07-30T14:15:00Z", "working")],
        &Redactor::new(&[]),
    );
    drain(&mut queue, &fake);

    let reports = parse_index(&fake.stored(&health_index_key(SESSION)).expect("index"));
    assert_eq!(reports.len(), 1);
}

#[test]
fn the_queue_is_capped_and_drops_the_oldest_unpublished_reports() {
    let mut queue = HealthPublishQueue::new();
    let batch: Vec<CopiedFile> = (0..(MAX_PENDING + 3))
        .map(|index| {
            let (hour, minute) = (index / 60, index % 60);
            copied(
                &format!("20260701-{hour:02}{minute:02}00"),
                &format!("2026-07-01T{hour:02}:{minute:02}:00Z"),
                "working",
            )
        })
        .collect();
    queue.enqueue(batch, &Redactor::new(&[]));
    assert_eq!(queue.pending(), MAX_PENDING);
}

#[test]
fn publishing_does_not_touch_the_log_objects() {
    let fake = FakeSink::default();
    let mut queue = HealthPublishQueue::new();
    queue.enqueue(
        vec![copied("20260730-141500", "2026-07-30T14:15:00Z", "working")],
        &Redactor::new(&[]),
    );
    drain(&mut queue, &fake);

    assert!(
        fake.calls()
            .iter()
            .all(|(key, _)| key.starts_with("health/")),
        "health publication must never write under logs/: {:?}",
        fake.calls().iter().map(|(key, _)| key).collect::<Vec<_>>()
    );
}
