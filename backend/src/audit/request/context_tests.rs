//! Unit tests for the per-request audit context's write-once slots.

use super::*;
use crate::audit::event::{ActorKind, PrincipalKind};

fn context() -> AuditRequestContext {
    AuditRequestContext::new()
}

#[test]
fn an_untouched_context_freezes_to_an_anonymous_record() {
    let frozen = context().freeze();
    assert_eq!(frozen.identity.actor.kind, ActorKind::Anonymous);
    assert_eq!(frozen.identity.principal.kind, PrincipalKind::Anonymous);
    assert!(frozen.arguments.is_empty());
    assert_eq!(
        frozen.arguments_parse_status,
        ArgumentsParseStatus::NotApplicable
    );
    assert_eq!(frozen.error_code, None);
    assert_eq!(frozen.correlation, Correlation::default());
    assert_eq!(frozen.conflicts, 0);
}

#[test]
fn every_slot_round_trips_into_the_frozen_snapshot() {
    let context = context();
    context.record_identity(AuditIdentity::github_bearer(42, "alice"));
    let mut values = serde_json::Map::new();
    values.insert("issue_number".to_string(), serde_json::json!(7));
    context.record_arguments(values.clone(), ArgumentsParseStatus::Parsed);
    context.record_error_code("not_found");
    context.record_session_id("sess-1");
    context.record_repo_full_name("acme/site");
    context.record_installation_id(99);
    context.record_trigger_issue(7);
    context.record_webhook_delivery_id("delivery-1");

    let frozen = context.freeze();
    assert_eq!(frozen.identity.actor.id, Some(42));
    assert_eq!(frozen.arguments, values);
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Parsed);
    assert_eq!(frozen.error_code.as_deref(), Some("not_found"));
    assert_eq!(frozen.correlation.session_id.as_deref(), Some("sess-1"));
    assert_eq!(
        frozen.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert_eq!(frozen.correlation.installation_id, Some(99));
    assert_eq!(frozen.correlation.trigger_issue, Some(7));
    assert_eq!(
        frozen.correlation.webhook_delivery_id.as_deref(),
        Some("delivery-1")
    );
    assert_eq!(frozen.conflicts, 0);
}

#[test]
fn writing_the_same_value_twice_is_not_a_conflict() {
    let context = context();
    context.record_session_id("sess-1");
    context.record_session_id("sess-1");
    context.record_identity(AuditIdentity::github_bearer(42, "alice"));
    context.record_identity(AuditIdentity::github_bearer(42, "alice"));
    assert_eq!(context.conflicts(), 0);
    assert_eq!(
        context.freeze().correlation.session_id.as_deref(),
        Some("sess-1")
    );
}

/// Fail-closed resolution: the FIRST verified value survives, the conflict is
/// counted, and the two values are never merged or concatenated.
#[test]
fn a_conflicting_write_keeps_the_first_value_and_counts_the_conflict() {
    let context = context();
    context.record_session_id("sess-first");
    context.record_session_id("sess-second");
    context.record_error_code("forbidden");
    context.record_error_code("not_found");

    let frozen = context.freeze();
    assert_eq!(
        frozen.correlation.session_id.as_deref(),
        Some("sess-first"),
        "the first write must win"
    );
    assert_eq!(frozen.error_code.as_deref(), Some("forbidden"));
    assert_eq!(frozen.conflicts, 2);
}

#[test]
fn a_conflicting_identity_keeps_the_first_verified_actor() {
    let context = context();
    context.record_identity(AuditIdentity::github_bearer(42, "alice"));
    context.record_identity(AuditIdentity::github_oauth(7, "mallory"));
    let frozen = context.freeze();
    assert_eq!(frozen.identity.actor.id, Some(42));
    assert_eq!(frozen.identity.actor.login.as_deref(), Some("alice"));
    assert_eq!(frozen.conflicts, 1);
}

#[test]
fn clones_share_one_set_of_slots() {
    let context = context();
    let clone = context.clone();
    clone.record_session_id("sess-1");
    assert_eq!(
        context.freeze().correlation.session_id.as_deref(),
        Some("sess-1")
    );
}

#[test]
fn install_makes_both_the_context_and_the_identity_slot_reachable() {
    let context = context();
    let mut extensions = axum::http::Extensions::new();
    context.install(&mut extensions);

    // The pre-existing identity helper writes through the shared cell.
    crate::audit::identity::record_identity(&extensions, AuditIdentity::github_bearer(42, "alice"));
    // And so does the context lookup used by the argument contract.
    with_context(&extensions, |ctx| ctx.record_session_id("sess-1"));

    let frozen = context.freeze();
    assert_eq!(frozen.identity.actor.id, Some(42));
    assert_eq!(frozen.correlation.session_id.as_deref(), Some("sess-1"));
    assert!(AuditRequestContext::from_extensions(&extensions).is_some());
}

#[test]
fn recording_without_an_installed_context_is_a_no_op() {
    let extensions = axum::http::Extensions::new();
    with_context(&extensions, |_| panic!("must not run without a context"));
    assert!(AuditRequestContext::from_extensions(&extensions).is_none());
}

/// A `{:?}` of a request must never dump identity or correlation values into a
/// log line nobody asked to contain them.
#[test]
fn debug_reveals_only_slot_occupancy_never_values() {
    let context = context();
    context.record_identity(AuditIdentity::github_bearer(42, "alice"));
    context.record_session_id("sess-secret");
    context.record_repo_full_name("acme/site");
    let rendered = format!("{context:?}");
    for leak in ["alice", "42", "sess-secret", "acme/site"] {
        assert!(!rendered.contains(leak), "{leak} leaked into {rendered}");
    }
    assert!(rendered.contains("session_id: true"), "{rendered}");
    assert!(rendered.contains("conflicts: 0"), "{rendered}");
}
