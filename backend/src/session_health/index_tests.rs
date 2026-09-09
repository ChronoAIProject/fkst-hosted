//! Tests for the health index: key shapes, the newest-first read-modify-write, the
//! cap, idempotency, and the leniency that keeps a corrupt index from propagating.

use super::super::{parse_report, parse_report_filename};
use super::*;

const SESSION: &str = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab";

fn report_text(generated_at: &str, status: &str, headline: &str) -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION}\"\n\
         producer = \"fkst-health@0.1.0\"\n\
         generated_at = \"{generated_at}\"\n\
         expected_interval_secs = 600\n\
         status = \"{status}\"\n\
         headline = \"{headline}\"\n\
         +++\nbody\n"
    )
}

fn entry(stamp: &str, generated_at: &str, status: &str) -> HealthIndexEntry {
    let file_name = format!("chronoai-fkst-{SESSION}-health-agent-status-report-{stamp}.md");
    let name = parse_report_filename(&file_name).expect("contract filename");
    let report = parse_report(&report_text(generated_at, status, "headline")).expect("parses");
    index_entry(SESSION, &name, &report)
}

#[test]
fn keys_are_a_sibling_of_the_log_prefix_not_nested_inside_it() {
    assert_eq!(
        health_index_key(SESSION),
        format!("health/{SESSION}/index.json")
    );
    assert_eq!(
        health_report_key(SESSION, "a-report.md"),
        format!("health/{SESSION}/a-report.md")
    );
    assert!(!health_index_key(SESSION).starts_with("logs/"));
}

#[test]
fn an_entry_denormalizes_everything_a_badge_and_the_watchdog_need() {
    let built = entry("20260730-141500", "2026-07-30T14:15:00Z", "stalled");

    assert_eq!(
        built.id,
        format!("chronoai-fkst-{SESSION}-health-agent-status-report-20260730-141500")
    );
    assert_eq!(built.key, format!("health/{SESSION}/{}.md", built.id));
    assert_eq!(built.generated_at, "2026-07-30T14:15:00Z");
    assert_eq!(built.expected_interval_secs, 600);
    assert_eq!(built.status, "stalled");
    assert_eq!(built.headline, "headline");
    assert_eq!(built.producer, "fkst-health@0.1.0");
}

#[test]
fn generated_at_is_normalized_so_lexical_order_is_time_order() {
    // The producer may write any RFC3339 rendering; the sort is a string compare.
    let offset = entry("20260730-141500", "2026-07-30T16:15:00+02:00", "working");
    assert_eq!(offset.generated_at, "2026-07-30T14:15:00Z");
    let fractional = entry("20260730-141500", "2026-07-30T14:15:00.123456Z", "working");
    assert_eq!(fractional.generated_at, "2026-07-30T14:15:00Z");
}

#[test]
fn an_unrecognized_status_is_carried_raw_rather_than_flattened() {
    let built = entry("20260730-141500", "2026-07-30T14:15:00Z", "thriving");
    assert_eq!(
        built.status, "thriving",
        "the reader maps the taxonomy; the index relays verbatim"
    );
}

#[test]
fn the_index_is_newest_first() {
    let older = entry("20260730-140500", "2026-07-30T14:05:00Z", "working");
    let newer = entry("20260730-141500", "2026-07-30T14:15:00Z", "stalled");

    // Inserted oldest-first to prove the sort, not the insertion order.
    let json = upsert_report(None, SESSION, older.clone());
    let json = upsert_report(Some(json.as_bytes()), SESSION, newer.clone());

    let reports = parse_index(json.as_bytes());
    assert_eq!(reports, vec![newer, older]);
}

#[test]
fn the_envelope_carries_the_schema_and_the_session() {
    let json = upsert_report(
        None,
        SESSION,
        entry("20260730-141500", "2026-07-30T14:15:00Z", "working"),
    );
    let index: HealthIndex = serde_json::from_str(&json).expect("valid json");
    assert_eq!(index.schema, HEALTH_INDEX_SCHEMA);
    assert_eq!(index.session_id, SESSION);
    assert_eq!(index.reports.len(), 1);
    assert!(json.ends_with('\n'), "trailing newline, like the run index");
}

#[test]
fn republishing_the_same_id_is_idempotent() {
    let built = entry("20260730-141500", "2026-07-30T14:15:00Z", "working");
    let once = upsert_report(None, SESSION, built.clone());
    let twice = upsert_report(Some(once.as_bytes()), SESSION, built.clone());

    assert_eq!(parse_index(twice.as_bytes()).len(), 1, "no duplicate entry");
    assert_eq!(once, twice, "and the object is byte-identical");
}

#[test]
fn a_rewritten_report_replaces_its_entry_rather_than_keeping_the_stale_verdict() {
    let first = entry("20260730-141500", "2026-07-30T14:15:00Z", "working");
    let corrected = HealthIndexEntry {
        status: "stalled".to_string(),
        headline: "corrected".to_string(),
        ..first.clone()
    };

    let json = upsert_report(None, SESSION, first);
    let json = upsert_report(Some(json.as_bytes()), SESSION, corrected);

    let reports = parse_index(json.as_bytes());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "stalled");
    assert_eq!(reports[0].headline, "corrected");
}

#[test]
fn the_index_is_capped_and_drops_the_oldest() {
    let all: Vec<HealthIndexEntry> = (0..(MAX_INDEX_ENTRIES + 5))
        .map(|index| {
            let (hour, minute) = (index / 60, index % 60);
            entry(
                &format!("20260701-{hour:02}{minute:02}00"),
                &format!("2026-07-01T{hour:02}:{minute:02}:00Z"),
                "working",
            )
        })
        .collect();

    let mut json: Option<String> = None;
    for built in &all {
        let next = upsert_report(json.as_deref().map(str::as_bytes), SESSION, built.clone());
        json = Some(next);
    }
    let json = json.expect("an index was built");

    let reports = parse_index(json.as_bytes());
    assert_eq!(reports.len(), MAX_INDEX_ENTRIES);
    assert_eq!(
        reports[0].id,
        all.last().expect("newest").id,
        "the newest survives"
    );
    assert!(
        !reports.iter().any(|report| report.id == all[0].id),
        "the oldest was dropped"
    );
}

#[test]
fn a_corrupt_or_absent_index_is_treated_as_empty_rather_than_propagating() {
    assert!(parse_index(b"").is_empty());
    assert!(parse_index(b"not json at all").is_empty());
    assert!(
        parse_index(b"[]").is_empty(),
        "a bare array is not the envelope"
    );
    assert!(parse_index(b"{\"unexpected\": true}").is_empty());

    // And a corrupt existing object does not stop a new entry from landing.
    let json = upsert_report(
        Some(b"{{{ truncated"),
        SESSION,
        entry("20260730-141500", "2026-07-30T14:15:00Z", "working"),
    );
    assert_eq!(parse_index(json.as_bytes()).len(), 1);
}

#[test]
fn an_index_missing_its_reports_array_still_parses_as_empty() {
    let json = format!("{{\"schema\":1,\"session_id\":\"{SESSION}\"}}");
    assert!(parse_index(json.as_bytes()).is_empty());
}
