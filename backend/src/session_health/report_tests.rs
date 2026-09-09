//! Tests for the v1 report document: the happy path, every documented failure, and
//! the leniency rules that keep one producer defect from becoming a fleet-wide outage.

use super::*;

const SESSION: &str = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab";

/// A complete, well-formed report — the shape the contract documents.
fn full_report() -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION}\"\n\
         namespace = \"chronoai-fkst\"\n\
         producer = \"fkst-health@0.1.0\"\n\
         generated_at = \"2026-07-30T14:15:00Z\"\n\
         window_start = \"2026-07-30T14:05:00Z\"\n\
         expected_interval_secs = 600\n\
         status = \"stalled\"\n\
         headline = \"No movement in 10m with 3 work items open\"\n\
         confidence = \"high\"\n\
         \n\
         [[evidence]]\n\
         key = \"deliveries_completed_delta\"\n\
         value = \"0\"\n\
         \n\
         [[work_items]]\n\
         number = 812\n\
         state = \"open\"\n\
         progress = \"none\"\n\
         +++\n\
         ## What this session is doing\n\
         \n\
         Nothing observable.\n"
    )
}

/// Front matter with only the required fields, plus whatever `extra` adds.
fn minimal(extra: &str) -> String {
    format!(
        "+++\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION}\"\n\
         producer = \"p@1\"\n\
         generated_at = \"2026-07-30T14:15:00Z\"\n\
         status = \"working\"\n\
         headline = \"fine\"\n\
         {extra}+++\nbody\n"
    )
}

#[test]
fn well_formed_report_parses_every_documented_field() {
    let report = parse_report(&full_report()).expect("parses");

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.session_id, SESSION);
    assert_eq!(report.namespace.as_deref(), Some("chronoai-fkst"));
    assert_eq!(report.producer, "fkst-health@0.1.0");
    assert_eq!(
        report.generated_at.to_rfc3339(),
        "2026-07-30T14:15:00+00:00"
    );
    assert_eq!(
        report.window_start.expect("window").to_rfc3339(),
        "2026-07-30T14:05:00+00:00"
    );
    assert_eq!(report.expected_interval_secs, 600);
    assert_eq!(report.status, HealthStatus::Stalled);
    assert_eq!(report.status_raw, "stalled");
    assert_eq!(report.headline, "No movement in 10m with 3 work items open");
    assert_eq!(report.confidence.as_deref(), Some("high"));
    assert_eq!(
        report.evidence,
        vec![EvidenceEntry {
            key: "deliveries_completed_delta".to_string(),
            value: "0".to_string(),
        }]
    );
    assert_eq!(
        report.work_items,
        vec![WorkItemProgress {
            number: 812,
            state: "open".to_string(),
            progress: "none".to_string(),
        }]
    );
    assert_eq!(
        report.body_markdown,
        "## What this session is doing\n\nNothing observable.\n"
    );
}

#[test]
fn every_taxonomy_status_maps_to_its_variant() {
    for (raw, expected) in [
        ("working", HealthStatus::Working),
        ("idle", HealthStatus::Idle),
        ("blocked", HealthStatus::Blocked),
        ("stalled", HealthStatus::Stalled),
        ("failing", HealthStatus::Failing),
        ("unknown", HealthStatus::Unknown),
    ] {
        let text = minimal("").replace("status = \"working\"", &format!("status = \"{raw}\""));
        let report = parse_report(&text).expect("parses");
        assert_eq!(report.status, expected, "status {raw}");
        assert_eq!(report.status_raw, raw);
    }
}

#[test]
fn status_matching_is_case_and_whitespace_insensitive() {
    let text = minimal("").replace("status = \"working\"", "status = \"  WORKING \"");
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.status, HealthStatus::Working);
    assert_eq!(report.status_raw, "WORKING", "the raw string is preserved");
}

#[test]
fn unknown_top_level_key_is_ignored() {
    let report = parse_report(&minimal("future_field = \"whatever\"\n")).expect("parses");
    assert_eq!(report.status, HealthStatus::Working);
}

#[test]
fn unrecognized_status_maps_to_unknown_and_preserves_the_raw_string() {
    let text = minimal("").replace("status = \"working\"", "status = \"thriving\"");
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.status, HealthStatus::Unknown);
    assert_eq!(report.status_raw, "thriving");
}

#[test]
fn empty_status_value_degrades_to_unknown_rather_than_failing() {
    let text = minimal("").replace("status = \"working\"", "status = \"\"");
    let report = parse_report(&text).expect("present-but-empty status is lenient");
    assert_eq!(report.status, HealthStatus::Unknown);
    assert_eq!(report.status_raw, "");
}

#[test]
fn missing_schema_version_is_the_skip_error() {
    let text = minimal("").replace("fkst_health_report = 1\n", "");
    assert_eq!(
        parse_report(&text),
        Err(ReportParseError::UnsupportedSchema {
            found: None,
            expected: SCHEMA_VERSION,
        })
    );
}

#[test]
fn wrong_schema_version_is_the_skip_error() {
    let text = minimal("").replace("fkst_health_report = 1", "fkst_health_report = 2");
    assert_eq!(
        parse_report(&text),
        Err(ReportParseError::UnsupportedSchema {
            found: Some(2),
            expected: SCHEMA_VERSION,
        })
    );
}

#[test]
fn non_numeric_schema_version_is_the_skip_error() {
    let text = minimal("").replace("fkst_health_report = 1", "fkst_health_report = \"one\"");
    assert_eq!(
        parse_report(&text),
        Err(ReportParseError::UnsupportedSchema {
            found: None,
            expected: SCHEMA_VERSION,
        })
    );
}

#[test]
fn stringified_schema_version_is_accepted() {
    let text = minimal("").replace("fkst_health_report = 1", "fkst_health_report = \"1\"");
    assert!(parse_report(&text).is_ok(), "a quoting slip is not fatal");
}

#[test]
fn tables_before_scalar_keys_is_a_typed_syntax_error() {
    // Invalid TOML: once `[[evidence]]` opens, `session_id` belongs to that table, so
    // the document does not carry the scalars the contract requires.
    let text = format!(
        "+++\n\
         [[evidence]]\n\
         key = \"k\"\n\
         value = \"v\"\n\
         fkst_health_report = 1\n\
         session_id = \"{SESSION}\"\n\
         +++\nbody\n"
    );
    match parse_report(&text) {
        Err(ReportParseError::UnsupportedSchema { .. })
        | Err(ReportParseError::FrontMatterSyntax(_)) => {}
        other => panic!("expected a typed error, got {other:?}"),
    }
}

#[test]
fn absent_front_matter_is_a_typed_error() {
    assert_eq!(
        parse_report("## just markdown\n"),
        Err(ReportParseError::MissingFrontMatter)
    );
    assert_eq!(parse_report(""), Err(ReportParseError::MissingFrontMatter));
    assert_eq!(
        parse_report("  +++\nfkst_health_report = 1\n+++\n"),
        Err(ReportParseError::MissingFrontMatter),
        "an indented fence does not open front matter"
    );
}

#[test]
fn unterminated_front_matter_is_a_typed_error() {
    assert_eq!(
        parse_report("+++\nfkst_health_report = 1\n"),
        Err(ReportParseError::UnterminatedFrontMatter)
    );
    assert_eq!(
        parse_report("+++"),
        Err(ReportParseError::UnterminatedFrontMatter)
    );
}

#[test]
fn non_map_front_matter_is_a_typed_error() {
    match parse_report("+++\nnot a table at all\n+++\nbody\n") {
        Err(ReportParseError::FrontMatterSyntax(_)) => {}
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

#[test]
fn each_required_field_is_reported_by_name_when_missing() {
    for (line, field) in [
        ("session_id = \"", "session_id"),
        ("producer = \"", "producer"),
        ("generated_at = \"", "generated_at"),
        ("headline = \"", "headline"),
        ("status = \"", "status"),
    ] {
        let full = minimal("");
        let stripped: String = full
            .lines()
            .filter(|candidate| !candidate.starts_with(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            parse_report(&format!("{stripped}\n")),
            Err(ReportParseError::MissingField(field)),
            "missing {field}"
        );
    }
}

#[test]
fn a_required_field_present_but_blank_is_missing() {
    let text = minimal("").replace("session_id = \"8f2c", "session_id = \"   \" #");
    match parse_report(&text) {
        Err(ReportParseError::MissingField("session_id"))
        | Err(ReportParseError::FrontMatterSyntax(_)) => {}
        other => panic!("expected a blank session_id to be rejected, got {other:?}"),
    }
    let blank = minimal("").replace("headline = \"fine\"", "headline = \"  \"");
    assert_eq!(
        parse_report(&blank),
        Err(ReportParseError::MissingField("headline"))
    );
}

#[test]
fn unparseable_timestamps_are_typed_errors() {
    let bad_generated = minimal("").replace("2026-07-30T14:15:00Z", "yesterday");
    assert_eq!(
        parse_report(&bad_generated),
        Err(ReportParseError::InvalidTimestamp {
            field: "generated_at"
        })
    );
    let bad_window = minimal("window_start = \"not-a-time\"\n");
    assert_eq!(
        parse_report(&bad_window),
        Err(ReportParseError::InvalidTimestamp {
            field: "window_start"
        })
    );
}

#[test]
fn blank_optional_window_start_is_absent_not_an_error() {
    let report = parse_report(&minimal("window_start = \"\"\n")).expect("parses");
    assert!(report.window_start.is_none());
}

#[test]
fn over_long_headline_is_truncated_never_rejected() {
    let long = "x".repeat(500);
    let text = minimal("").replace("headline = \"fine\"", &format!("headline = \"{long}\""));
    let report = parse_report(&text).expect("truncated, not rejected");
    assert_eq!(report.headline.chars().count(), HEADLINE_MAX_CHARS);
    assert!(report.headline.ends_with('…'));
}

#[test]
fn headline_truncation_counts_characters_not_bytes() {
    let long = "é".repeat(500);
    let text = minimal("").replace("headline = \"fine\"", &format!("headline = \"{long}\""));
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.headline.chars().count(), HEADLINE_MAX_CHARS);
}

#[test]
fn evidence_over_the_cap_is_truncated_to_the_cap() {
    let mut extra = String::new();
    for index in 0..(EVIDENCE_MAX_ENTRIES + 20) {
        extra.push_str(&format!(
            "\n[[evidence]]\nkey = \"k{index}\"\nvalue = \"v\"\n"
        ));
    }
    let report = parse_report(&minimal(&extra)).expect("parses");
    assert_eq!(report.evidence.len(), EVIDENCE_MAX_ENTRIES);
    assert_eq!(
        report.evidence[0].key, "k0",
        "the earliest entries are kept"
    );
}

#[test]
fn work_items_over_the_cap_are_truncated_to_the_cap() {
    let mut extra = String::new();
    for index in 0..(WORK_ITEMS_MAX_ENTRIES + 10) {
        extra.push_str(&format!(
            "\n[[work_items]]\nnumber = {index}\nstate = \"open\"\nprogress = \"none\"\n"
        ));
    }
    let report = parse_report(&minimal(&extra)).expect("parses");
    assert_eq!(report.work_items.len(), WORK_ITEMS_MAX_ENTRIES);
}

#[test]
fn over_long_evidence_key_and_value_are_truncated() {
    let key = "k".repeat(200);
    let value = "v".repeat(1000);
    let report = parse_report(&minimal(&format!(
        "\n[[evidence]]\nkey = \"{key}\"\nvalue = \"{value}\"\n"
    )))
    .expect("parses");
    assert_eq!(
        report.evidence[0].key.chars().count(),
        EVIDENCE_KEY_MAX_CHARS
    );
    assert_eq!(
        report.evidence[0].value.chars().count(),
        EVIDENCE_VALUE_MAX_CHARS
    );
}

#[test]
fn malformed_optional_structure_degrades_instead_of_failing() {
    // `evidence` is not an array at all.
    let report = parse_report(&minimal("evidence = \"nope\"\n")).expect("still parses");
    assert!(report.evidence.is_empty());

    // One junk entry among good ones: the junk is dropped, the rest survive.
    let report = parse_report(&minimal(
        "\n[[evidence]]\nkey = \"good\"\nvalue = \"1\"\n\n[[evidence]]\nvalue = \"keyless\"\n",
    ))
    .expect("still parses");
    assert_eq!(report.evidence.len(), 1);
    assert_eq!(report.evidence[0].key, "good");

    // A work item whose number cannot be read is dropped, not fatal.
    let report = parse_report(&minimal(
        "\n[[work_items]]\nnumber = \"812\"\nstate = \"open\"\nprogress = \"none\"\n\n[[work_items]]\nnumber = \"abc\"\n",
    ))
    .expect("still parses");
    assert_eq!(report.work_items.len(), 1);
    assert_eq!(report.work_items[0].number, 812);
}

#[test]
fn non_string_evidence_values_are_rendered_as_strings() {
    let report = parse_report(&minimal(
        "\n[[evidence]]\nkey = \"count\"\nvalue = 0\n\n[[evidence]]\nkey = \"flag\"\nvalue = true\n",
    ))
    .expect("parses");
    assert_eq!(report.evidence[0].value, "0");
    assert_eq!(report.evidence[1].value, "true");
}

#[test]
fn expected_interval_defaults_when_absent_or_nonsensical() {
    let report = parse_report(&minimal("")).expect("parses");
    assert_eq!(
        report.expected_interval_secs,
        DEFAULT_EXPECTED_INTERVAL_SECS
    );

    let zero = parse_report(&minimal("expected_interval_secs = 0\n")).expect("parses");
    assert_eq!(zero.expected_interval_secs, DEFAULT_EXPECTED_INTERVAL_SECS);

    let negative = parse_report(&minimal("expected_interval_secs = -5\n")).expect("parses");
    assert_eq!(
        negative.expected_interval_secs,
        DEFAULT_EXPECTED_INTERVAL_SECS
    );

    let junk = parse_report(&minimal("expected_interval_secs = \"soon\"\n")).expect("parses");
    assert_eq!(junk.expected_interval_secs, DEFAULT_EXPECTED_INTERVAL_SECS);
}

#[test]
fn a_producer_declared_interval_is_honoured() {
    let report = parse_report(&minimal("expected_interval_secs = 1800\n")).expect("parses");
    assert_eq!(
        report.expected_interval_secs, 1800,
        "the producer, not the control plane, owns the cadence"
    );
}

#[test]
fn body_round_trips_byte_for_byte_including_embedded_fences() {
    let body = "## Title\n\n+++\nnot front matter\n+++\n\n---\n\nyaml-looking, still opaque\n";
    let text = format!("{}{body}", minimal("").trim_end_matches("body\n"));
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.body_markdown, body);
}

#[test]
fn an_empty_body_is_allowed() {
    let text = format!(
        "+++\nfkst_health_report = 1\nsession_id = \"{SESSION}\"\nproducer = \"p@1\"\n\
         generated_at = \"2026-07-30T14:15:00Z\"\nstatus = \"working\"\nheadline = \"x\"\n+++"
    );
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.body_markdown, "");
}

#[test]
fn crlf_line_endings_parse() {
    let text = minimal("").replace('\n', "\r\n");
    let report = parse_report(&text).expect("parses CRLF");
    assert_eq!(report.status, HealthStatus::Working);
    assert_eq!(report.body_markdown, "body\r\n");
}

#[test]
fn a_leading_byte_order_mark_does_not_hide_the_fence() {
    let text = format!("\u{feff}{}", minimal(""));
    assert!(parse_report(&text).is_ok());
}

#[test]
fn trailing_whitespace_on_a_fence_is_tolerated() {
    let text = minimal("").replacen("+++\n", "+++  \n", 1);
    assert!(parse_report(&text).is_ok());
}

#[test]
fn parse_errors_render_a_useful_message() {
    assert_eq!(
        ReportParseError::MissingField("status").to_string(),
        "report is missing required field `status`"
    );
    assert!(ReportParseError::MissingFrontMatter
        .to_string()
        .contains("front matter"));
}

/// A CROSS-PRODUCER conformance fixture: the exact bytes the `fkst-health` package's
/// Lua renderer emits, captured by running that renderer and pasted here verbatim.
///
/// The producer and this parser ship from two different branches and two different
/// languages, so nothing but a real captured artifact proves they agree. If the
/// producer's rendering ever drifts — a YAML fence, a reordered table, a `:` in the
/// stamp — this test fails on the develop side before a session ever writes one.
#[test]
fn the_real_producers_output_parses_into_every_field() {
    let text = include_str!("fixtures/producer-report-v1.md");
    let report = parse_report(text).expect("the shipped producer's output must parse");

    assert_eq!(report.schema_version, SCHEMA_VERSION);
    assert_eq!(report.session_id, "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab");
    assert_eq!(
        report.namespace.as_deref(),
        Some("chronoai-fkst-cloud-test")
    );
    assert_eq!(report.producer, "fkst-health@0.1.0");
    assert_eq!(
        report.generated_at.to_rfc3339(),
        "2026-07-31T12:15:00+00:00"
    );
    assert_eq!(
        report.window_start.expect("window").to_rfc3339(),
        "2026-07-31T12:05:00+00:00"
    );
    assert_eq!(report.expected_interval_secs, 600);
    assert_eq!(report.status, HealthStatus::Stalled);
    assert_eq!(report.confidence.as_deref(), Some("high"));
    assert_eq!(report.evidence.len(), 2);
    assert_eq!(report.evidence[0].key, "deliveries_completed_delta");
    assert_eq!(report.work_items.len(), 1);
    assert_eq!(report.work_items[0].number, 812);
    assert!(report
        .body_markdown
        .starts_with("## What this session is doing"));
}

/// The filenames that same renderer produces, parsed by this module's parser — the
/// other half of the cross-language contract.
#[test]
fn the_real_producers_filenames_parse() {
    let namespaced = "chronoai-fkst-cloud-test-8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260731-121500.md";
    let parsed = crate::session_health::parse_report_filename(namespaced).expect("parses");
    assert_eq!(
        parsed.namespace.as_deref(),
        Some("chronoai-fkst-cloud-test")
    );
    assert_eq!(parsed.session_id, "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab");
    assert_eq!(parsed.stamp, "20260731-121500");

    let bare = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab-health-agent-status-report-20260731-121500.md";
    let parsed = crate::session_health::parse_report_filename(bare).expect("parses");
    assert_eq!(parsed.namespace, None);
    assert_eq!(parsed.session_id, "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab");
}
