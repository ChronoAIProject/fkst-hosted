//! Unit tests for the closed filter vocabulary.
//!
//! Two properties are argued here: every filter is EXACT (an absent runtime value
//! never matches), and every rejection is a named `400` rather than a silently
//! widened query.

use super::super::test_support::item;
use super::*;

fn alice_item() -> RuntimeInventoryItem {
    item("fkst-sess-a", Some("sess-a"))
}

#[test]
fn an_empty_filter_set_matches_every_authorized_row() {
    assert!(SandboxFilters::default().matches(&alice_item()));
}

#[test]
fn every_filter_matches_its_own_field_and_nothing_else() {
    let subject = alice_item();
    let cases: Vec<(SandboxFilters, bool)> = vec![
        (
            SandboxFilters {
                status: Some(RuntimeInventoryStatus::Running),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                status: Some(RuntimeInventoryStatus::Failed),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                backend: Some(RuntimeBackendKind::Kubernetes),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                backend: Some(RuntimeBackendKind::OpenSandbox),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                creator_id: Some(101),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                creator_id: Some(999),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                creator_login: Some("ALICE".to_string()),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                creator_login: Some("bob".to_string()),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                repo_full_name: Some("ACME/Site".to_string()),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                repo_full_name: Some("acme/other".to_string()),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                session_id: Some("sess-a".to_string()),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                session_id: Some("sess-b".to_string()),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                trigger_issue: Some(7),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                trigger_issue: Some(8),
                ..SandboxFilters::default()
            },
            false,
        ),
        (
            SandboxFilters {
                attribution_source: Some(AttributionSource::LaunchMetadata),
                ..SandboxFilters::default()
            },
            true,
        ),
        (
            SandboxFilters {
                attribution_source: Some(AttributionSource::UnknownLegacy),
                ..SandboxFilters::default()
            },
            false,
        ),
    ];
    for (filters, expected) in cases {
        assert_eq!(
            filters.matches(&subject),
            expected,
            "unexpected verdict for {filters:?}"
        );
    }
}

/// Combined filters are a conjunction: one mismatch is enough to drop the row.
#[test]
fn combined_filters_must_all_match() {
    let subject = alice_item();
    assert!(SandboxFilters {
        status: Some(RuntimeInventoryStatus::Running),
        creator_id: Some(101),
        repo_full_name: Some("acme/site".to_string()),
        ..SandboxFilters::default()
    }
    .matches(&subject));
    assert!(!SandboxFilters {
        status: Some(RuntimeInventoryStatus::Running),
        creator_id: Some(101),
        repo_full_name: Some("acme/other".to_string()),
        ..SandboxFilters::default()
    }
    .matches(&subject));
}

/// "Unknown" is not "equal to whatever you asked for": a runtime that never
/// carried the value is not returned by a filter on it.
#[test]
fn an_absent_runtime_value_never_matches_a_stated_filter() {
    let orphan = RuntimeInventoryItem {
        creator_id: None,
        creator_login: None,
        repo_full_name: None,
        trigger_issue: None,
        session_id: None,
        ..alice_item()
    };
    for filters in [
        SandboxFilters {
            creator_id: Some(101),
            ..SandboxFilters::default()
        },
        SandboxFilters {
            creator_login: Some("alice".to_string()),
            ..SandboxFilters::default()
        },
        SandboxFilters {
            repo_full_name: Some("acme/site".to_string()),
            ..SandboxFilters::default()
        },
        SandboxFilters {
            session_id: Some("sess-a".to_string()),
            ..SandboxFilters::default()
        },
        SandboxFilters {
            trigger_issue: Some(7),
            ..SandboxFilters::default()
        },
    ] {
        assert!(!filters.matches(&orphan), "{filters:?} matched an orphan");
    }
}

#[test]
fn every_closed_vocabulary_round_trips_its_wire_value() {
    for status in RuntimeInventoryStatus::ALL {
        assert_eq!(parse_status(status.as_str()).expect("parses"), status);
    }
    for backend in RuntimeBackendKind::ALL {
        assert_eq!(parse_backend(backend.as_str()).expect("parses"), backend);
    }
    for source in AttributionSource::ALL {
        assert_eq!(
            parse_attribution_source(source.as_str()).expect("parses"),
            source
        );
    }
}

/// A value outside the closed set is a named `400`, never a silently dropped
/// filter — dropping it would WIDEN the query the caller asked for.
#[test]
fn an_unknown_value_is_rejected_and_names_its_parameter() {
    for (rendered, expected) in [
        (
            format!("{}", parse_status("gone").expect_err("rejected")),
            "status",
        ),
        (
            format!("{}", parse_backend("nomad").expect_err("rejected")),
            "backend",
        ),
        (
            format!(
                "{}",
                parse_attribution_source("guessed").expect_err("rejected")
            ),
            "attribution_source",
        ),
        (
            format!("{}", parse_creator_id(0).expect_err("rejected")),
            "creator_id",
        ),
        (
            format!("{}", parse_creator_id(-4).expect_err("rejected")),
            "creator_id",
        ),
        (
            format!(
                "{}",
                parse_creator_login("not a login").expect_err("rejected")
            ),
            "creator_login",
        ),
        (
            format!("{}", parse_repo_full_name("acme").expect_err("rejected")),
            "repo_full_name",
        ),
        (
            format!("{}", parse_session_id("../escape").expect_err("rejected")),
            "session_id",
        ),
        (
            format!("{}", parse_trigger_issue(0).expect_err("rejected")),
            "trigger_issue",
        ),
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
}

/// The value that failed is never echoed back — an exact probe must not become an
/// oracle even through an error message.
#[test]
fn a_rejection_never_echoes_the_value_that_failed() {
    let error = parse_session_id("canary-session-value/../escape").expect_err("rejected");
    assert!(
        !format!("{error}").contains("canary-session-value"),
        "{error}"
    );
}

#[test]
fn accepted_values_are_normalized_into_their_stored_form() {
    assert_eq!(parse_creator_login(" @Alice ").expect("parses"), "Alice");
    assert_eq!(
        parse_repo_full_name(" acme/site ").expect("parses"),
        "acme/site"
    );
    assert_eq!(
        parse_status(" running ").expect("parses"),
        RuntimeInventoryStatus::Running
    );
}
