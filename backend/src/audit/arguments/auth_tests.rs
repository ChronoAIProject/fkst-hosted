//! Unit tests for the authentication/OAuth safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

/// The hostile values these routes actually receive.
const CANARIES: &[&str] = &[
    "canary-oauth-code",
    "canary-oauth-state",
    "canary-oauth-error",
    "canary-access-token",
    "canary-refresh-token",
];

#[test]
fn every_flow_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafeGithubLogin>();
    assert_policy_matches::<SafeGithubLoginCallback>();
    assert_policy_matches::<SafeGithubRefreshToken>();
    assert_policy_matches::<SafeGithubBroaderConnect>();
    assert_policy_matches::<SafeGithubBroaderCallback>();
    assert_policy_matches::<SafeSessionLogsOauthCallback>();
}

#[test]
fn the_authorize_redirects_record_only_their_flow() {
    let login = properties(&SafeGithubLogin::new());
    assert_eq!(login.len(), 1);
    assert_eq!(string(&login, "flow").as_deref(), Some(FLOW_LOGIN));

    let broader = properties(&SafeGithubBroaderConnect::new());
    assert_eq!(broader.len(), 1);
    assert_eq!(
        string(&broader, "flow").as_deref(),
        Some(FLOW_BROADER_VISIBILITY)
    );
}

#[test]
fn the_default_constructors_agree_with_the_explicit_ones() {
    assert_eq!(
        properties(&SafeGithubLogin::default()),
        properties(&SafeGithubLogin::new())
    );
    assert_eq!(
        properties(&SafeGithubBroaderConnect::default()),
        properties(&SafeGithubBroaderConnect::new())
    );
}

#[test]
fn every_callback_outcome_renders_its_closed_wire_value() {
    for (result, expected) in [
        (OauthResult::Success, "success"),
        (OauthResult::Denied, "denied"),
        (OauthResult::Invalid, "invalid"),
        (OauthResult::UpstreamError, "upstream_error"),
    ] {
        let values = properties(&SafeGithubLoginCallback::new(result));
        assert_eq!(string(&values, "result").as_deref(), Some(expected));
        assert_eq!(string(&values, "flow").as_deref(), Some(FLOW_LOGIN));
        assert_eq!(values.len(), 2);
    }
}

#[test]
fn the_refresh_and_broader_callbacks_name_their_own_flows() {
    let refresh = properties(&SafeGithubRefreshToken::new(OauthResult::Success));
    assert_eq!(string(&refresh, "flow").as_deref(), Some(FLOW_REFRESH));
    let broader = properties(&SafeGithubBroaderCallback::new(OauthResult::Denied));
    assert_eq!(
        string(&broader, "flow").as_deref(),
        Some(FLOW_BROADER_VISIBILITY)
    );
    assert_eq!(string(&broader, "result").as_deref(), Some("denied"));
}

/// The rule that matters most on this surface: an unverified state names no
/// session. Anyone can craft a state; extracting a session id from one would let
/// them attach their failed callback to a session they never had access to.
#[test]
fn an_unverified_state_contributes_no_session_id() {
    let values = properties(
        &SessionLogsCallbackInput {
            verified_session_id: None,
            result: OauthResult::Invalid,
        }
        .to_safe_audit_arguments(),
    );
    assert!(!values.contains_key("session_id"));
    assert_eq!(string(&values, "flow").as_deref(), Some(FLOW_SESSION_LOGS));
    assert_eq!(string(&values, "result").as_deref(), Some("invalid"));
}

#[test]
fn a_verified_state_contributes_its_session_id() {
    let safe = SessionLogsCallbackInput {
        verified_session_id: Some("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e"),
        result: OauthResult::Success,
    }
    .to_safe_audit_arguments();
    assert_within_allowlist(&safe);
    assert_eq!(
        string(&properties(&safe), "session_id").as_deref(),
        Some("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")
    );
}

/// Even a VERIFIED state whose payload is not a session id is dropped: the
/// signature proves the value came from us, not that it is a session id.
#[test]
fn a_verified_but_malformed_session_id_is_still_dropped() {
    let safe = SessionLogsCallbackInput {
        verified_session_id: Some("../../etc/passwd"),
        result: OauthResult::Success,
    }
    .to_safe_audit_arguments();
    assert!(!properties(&safe).contains_key("session_id"));
}

#[test]
fn no_oauth_material_can_reach_any_flow_record() {
    assert_no_canary(&SafeGithubLogin::new(), CANARIES);
    assert_no_canary(
        &SafeGithubLoginCallback::new(OauthResult::UpstreamError),
        CANARIES,
    );
    assert_no_canary(&SafeGithubRefreshToken::new(OauthResult::Invalid), CANARIES);
    assert_no_canary(
        &SessionLogsCallbackInput {
            verified_session_id: Some("canary-oauth-state"),
            result: OauthResult::Denied,
        }
        .to_safe_audit_arguments(),
        &["canary-oauth-code", "canary-access-token"],
    );
}
