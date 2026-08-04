//! Boundary tests for the source contract itself.

use super::*;
use crate::operations::filters::{ActivityFilters, RecordKind};
use crate::operations::test_support::{all, authorized_session, mine, range};

const VIEWER_ID: i64 = 101;
const VIEWER: &str = "alice";
const SESSION: &str = "sess-alice";

fn query(constraint: crate::session_access::ActivityVisibilityConstraint) -> SourceQuery {
    SourceQuery {
        constraint,
        record_kind: RecordKind::All,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: None,
        fetch_limit: 11,
    }
}

/// The authorized session travels with the constraint, never as a loose value a
/// source could forget to apply.
#[test]
fn the_authorized_session_is_read_from_the_sealed_constraint() {
    let session = authorized_session(SESSION, VIEWER_ID, VIEWER);
    let personal = query(mine(VIEWER_ID, VIEWER, Some(session)));
    assert_eq!(personal.authorized_session_id(), Some(SESSION));

    let unscoped = query(mine(VIEWER_ID, VIEWER, None));
    assert_eq!(unscoped.authorized_session_id(), None);
}

/// A global administrator carries no session token: they already see every
/// session's rows, and a token would only suggest it narrowed authorization.
#[test]
fn the_global_scope_carries_no_session_token() {
    assert_eq!(query(all(900, "root")).authorized_session_id(), None);
}

#[test]
fn source_errors_split_into_the_documented_fault_classes() {
    assert!(SourceError::Upstream { kind: "auth" }.is_upstream_fault());
    assert!(SourceError::Upstream { kind: "schema" }.is_upstream_fault());
    assert!(!SourceError::Transient { kind: "timeout" }.is_upstream_fault());
    assert!(!SourceError::NotConfigured.is_upstream_fault());
    assert_eq!(SourceError::NotConfigured.kind(), "not_configured");
    assert_eq!(SourceError::Upstream { kind: "auth" }.kind(), "auth");
}

/// The error type must be structurally incapable of carrying a URL, a
/// credential, or upstream text.
#[test]
fn a_source_error_never_carries_free_text() {
    let rendered = format!(
        "{} {}",
        SourceError::Upstream { kind: "auth" },
        SourceError::Transient { kind: "timeout" }
    );
    assert!(!rendered.contains("http"), "{rendered}");
    assert!(!rendered.contains("Bearer"), "{rendered}");
}

#[test]
fn saturation_is_measured_on_raw_rows_not_decoded_ones() {
    let page = SourcePage {
        records: Vec::new(),
        raw_rows: 11,
        row_errors: 11,
    };
    assert!(
        page.saturated(11),
        "a page of undecodable rows is still a full page; forgetting that would \
         silently truncate the timeline"
    );
    assert!(!SourcePage::default().saturated(11));
}
