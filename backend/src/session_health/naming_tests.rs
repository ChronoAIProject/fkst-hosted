//! Tests for report filenames: the generator/parser round trip, the URL-safety
//! guarantee, and the traversal guard every downstream consumer leans on.

use k8s_openapi::chrono::{DateTime, Utc};

use super::*;

const SESSION: &str = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab";

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("fixture timestamp")
        .with_timezone(&Utc)
}

#[test]
fn filename_has_the_contract_shape() {
    let name = report_filename(Some("chronoai-fkst"), SESSION, at("2026-07-30T14:15:00Z"));
    assert_eq!(
        name,
        format!("chronoai-fkst-{SESSION}-health-agent-status-report-20260730-141500.md")
    );
}

#[test]
fn filename_never_contains_a_colon() {
    let name = report_filename(Some("chronoai-fkst"), SESSION, at("2026-07-30T14:15:00Z"));
    assert!(
        !name.contains(':'),
        "the name becomes an object key and a URL segment: {name}"
    );
}

#[test]
fn the_timestamp_is_rendered_in_utc() {
    // Same instant, expressed with an offset: the stamp must be the UTC rendering.
    let name = report_filename(None, SESSION, at("2026-07-30T16:15:00+02:00"));
    assert!(name.ends_with("-20260730-141500.md"), "{name}");
}

#[test]
fn an_absent_namespace_omits_the_segment_and_its_hyphen() {
    let name = report_filename(None, SESSION, at("2026-07-30T14:15:00Z"));
    assert_eq!(
        name,
        format!("{SESSION}-health-agent-status-report-20260730-141500.md")
    );
    assert!(!name.starts_with('-'), "no orphaned joining hyphen");
}

#[test]
fn a_blank_namespace_is_treated_as_absent() {
    let name = report_filename(Some("   "), SESSION, at("2026-07-30T14:15:00Z"));
    assert_eq!(
        name,
        format!("{SESSION}-health-agent-status-report-20260730-141500.md")
    );
}

#[test]
fn generated_names_round_trip_with_a_namespace() {
    let generated_at = at("2026-07-30T14:15:00Z");
    let name = report_filename(Some("chronoai-fkst"), SESSION, generated_at);
    let parsed = parse_report_filename(&name).expect("parses");

    assert_eq!(parsed.namespace.as_deref(), Some("chronoai-fkst"));
    assert_eq!(parsed.session_id, SESSION);
    assert_eq!(parsed.stamp, "20260730-141500");
    assert_eq!(parsed.id, name.trim_end_matches(".md"));
}

#[test]
fn generated_names_round_trip_without_a_namespace() {
    let name = report_filename(None, SESSION, at("2026-07-30T14:15:00Z"));
    let parsed = parse_report_filename(&name).expect("parses");

    assert_eq!(parsed.namespace, None);
    assert_eq!(parsed.session_id, SESSION);
}

#[test]
fn a_multi_hyphen_namespace_round_trips() {
    // The whole point of anchoring on the UUID: the namespace's own hyphens must not
    // confuse the split.
    let name = report_filename(
        Some("chronoai-fkst-cloud-test"),
        SESSION,
        at("2026-07-30T14:15:00Z"),
    );
    let parsed = parse_report_filename(&name).expect("parses");
    assert_eq!(
        parsed.namespace.as_deref(),
        Some("chronoai-fkst-cloud-test")
    );
    assert_eq!(parsed.session_id, SESSION);
}

#[test]
fn a_namespace_that_itself_ends_in_a_uuid_still_splits_on_the_real_session_id() {
    let odd = format!("ns-{SESSION}");
    let name = report_filename(Some(&odd), SESSION, at("2026-07-30T14:15:00Z"));
    let parsed = parse_report_filename(&name).expect("parses");
    assert_eq!(
        parsed.session_id, SESSION,
        "the LAST uuid is the session id"
    );
    assert_eq!(parsed.namespace.as_deref(), Some(odd.as_str()));
}

#[test]
fn the_id_is_the_stem_and_is_url_safe() {
    let name = report_filename(Some("chronoai-fkst"), SESSION, at("2026-07-30T14:15:00Z"));
    let parsed = parse_report_filename(&name).expect("parses");
    assert!(!parsed.id.contains(':'));
    assert!(!parsed.id.contains('/'));
    assert!(!parsed.id.ends_with(".md"));
}

#[test]
fn a_non_uuid_session_id_degrades_to_no_namespace() {
    // Documented limitation: with no UUID anchor the split is unrecoverable, so the
    // parser reports what is certain rather than guessing. Production session ids are
    // always UUIDs, so this case does not arise there.
    let name = report_filename(Some("ns"), "sess-1", at("2026-07-30T14:15:00Z"));
    let parsed = parse_report_filename(&name).expect("parses");
    assert_eq!(parsed.namespace, None);
    assert_eq!(parsed.session_id, "ns-sess-1");
    assert_eq!(parsed.stamp, "20260730-141500");
}

#[test]
fn non_report_filenames_are_rejected() {
    for name in [
        "",
        "notes.md",
        "readme.txt",
        &format!("{SESSION}-health-agent-status-report-20260730-141500"), // no extension
        &format!("{SESSION}-health-agent-status-report-20260730-141500.markdown"),
        &format!("{SESSION}-health-agent-status-report-2026073-141500.md"), // short date
        &format!("{SESSION}-health-agent-status-report-20260730-14150.md"), // short time
        &format!("{SESSION}-health-agent-status-report-20260730T141500.md"), // no hyphen
        &format!("{SESSION}-health-agent-status-report-abcdefgh-141500.md"), // non-digits
        "-health-agent-status-report-20260730-141500.md",                   // empty prefix
        &format!("{SESSION}-some-other-artifact-20260730-141500.md"),       // no marker
    ] {
        assert!(
            parse_report_filename(name).is_none(),
            "must reject {name:?}"
        );
    }
}

#[test]
fn traversal_shaped_names_are_rejected() {
    // The guard every consumer that builds a path or an object key depends on.
    for name in [
        "../etc-health-agent-status-report-20260730-141500.md",
        "a/b-health-agent-status-report-20260730-141500.md",
        "a\\b-health-agent-status-report-20260730-141500.md",
        ".md",
        "..md",
        &format!(".hidden-{SESSION}-health-agent-status-report-20260730-141500.md"),
        &format!("{SESSION}-health-agent-status-report-20260730-141500\n.md"),
    ] {
        assert!(
            parse_report_filename(name).is_none(),
            "must reject traversal-shaped {name:?}"
        );
    }
}

#[test]
fn a_repeated_marker_resolves_on_the_last_occurrence() {
    let odd_namespace = "ns-health-agent-status-report-x";
    let name = report_filename(Some(odd_namespace), SESSION, at("2026-07-30T14:15:00Z"));
    let parsed = parse_report_filename(&name).expect("parses");
    assert_eq!(parsed.session_id, SESSION);
    assert_eq!(parsed.stamp, "20260730-141500");
}

#[test]
fn stamps_sort_chronologically_as_plain_strings() {
    // The collector and the index both rely on lexical order being time order.
    let earlier = report_filename(None, SESSION, at("2026-07-30T09:05:00Z"));
    let later = report_filename(None, SESSION, at("2026-07-30T14:15:00Z"));
    let next_year = report_filename(None, SESSION, at("2027-01-01T00:00:00Z"));
    assert!(earlier < later);
    assert!(later < next_year);
}
