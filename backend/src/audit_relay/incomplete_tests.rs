//! Incomplete synthesis: no fabricated status, actor, or arguments, and the same
//! event id as the start it closes.

use super::*;
use crate::audit::event::AuditOutcome;
use crate::audit::validate::validate;
use crate::audit_relay::test_support::{now, start};

const EVENT: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn the_synthesized_terminal_states_only_what_is_known() {
    let (terminal, terminal_at) = synthesize(&start(EVENT)).expect("synthesizes");
    assert_eq!(terminal.event_id, EVENT, "the SAME event id as the start");
    assert_eq!(terminal.status_code, None, "no status was ever produced");
    assert_eq!(terminal.outcome, AuditOutcome::Incomplete.as_str());
    assert_eq!(terminal.error_code.as_deref(), Some(INCOMPLETE_ERROR_CODE));
    assert_eq!(terminal.actor_id, None, "no actor was ever verified");
    assert_eq!(terminal.actor.kind, "anonymous");
    assert!(terminal.arguments.is_empty());
    assert_eq!(terminal.arguments_parse_status, "unavailable");
    assert_eq!(terminal.session_id, None);
    // The terminal instant is the record's own deadline, not "now".
    assert_eq!(
        terminal_at,
        now() + k8s_openapi::chrono::Duration::seconds(60)
    );
    assert_eq!(terminal.completed_at, start(EVENT).completion_deadline_at);
    assert_eq!(terminal.duration_ms, 60_000);
}

#[test]
fn the_synthesized_terminal_passes_the_audit_event_contract() {
    let (terminal, _) = synthesize(&start(EVENT)).expect("synthesizes");
    let domain = terminal.to_domain().expect("parses back");
    validate(&domain).expect("a synthesized incomplete is a legal audit record");
}

#[test]
fn the_route_identity_is_copied_from_the_start_never_invented() {
    let mut registered = start(EVENT);
    registered.method = "POST".to_string();
    registered.route_template = "/api/v1/users/me/environment-profiles/{name}".to_string();
    registered.operation_id = "environments_put".to_string();
    let (terminal, _) = synthesize(&registered).expect("synthesizes");
    assert_eq!(terminal.method, "POST");
    assert_eq!(
        terminal.route_template,
        "/api/v1/users/me/environment-profiles/{name}"
    );
    assert_eq!(terminal.operation_id, "environments_put");
    assert_eq!(terminal.service_version, registered.service_version);
    assert_eq!(
        terminal.deployment_environment,
        registered.deployment_environment
    );
}
