//! Handler-level tests: the verify-then-trust ordering, and event routing.
//!
//! Signature verification itself is tested beside the verifier (`verify.rs`),
//! sender parsing beside `sender.rs`, and every installation-event behaviour
//! beside `installation.rs`. What is asserted here is what only the handler can
//! prove: that a delivery is authenticated BEFORE its body is believed.

use axum::body::Bytes;
use axum::http::{Extensions, HeaderMap};
use secrecy::SecretString;

use super::test_support::state_with_reconciler;
use super::verify::{sign, SIGNATURE_HEADER};
use super::*;
use crate::audit::AuditIdentitySlot;

/// The deployment's configured webhook secret for these tests.
const SECRET_TEXT: &str = "whsec_handler_test";
const SECRET: &[u8] = SECRET_TEXT.as_bytes();

/// A body whose `sender` fields are exactly what an attacker would forge.
const MALICIOUS_BODY: &str =
    r#"{"action":"opened","sender":{"id":1,"login":"attacker"},"installation":{"id":99}}"#;

/// Drive the real handler with `signature` over `body`, returning the response
/// status and whatever identity the request's audit slot ended up holding.
async fn call(
    signature: Option<&str>,
    body: &str,
) -> (StatusCode, Option<crate::audit::AuditIdentity>) {
    let (mut state, _rx) = state_with_reconciler();
    state.github_app_webhook_secret = Some(SecretString::from(SECRET_TEXT));

    let mut headers = HeaderMap::new();
    headers.insert("x-github-event", "issues".parse().expect("header"));
    if let Some(signature) = signature {
        headers.insert(SIGNATURE_HEADER, signature.parse().expect("header"));
    }

    // The slot the outer audit middleware installs; the handler publishes into it
    // only once the delivery is proven authentic.
    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());

    let response = webhook(
        axum::extract::State(state),
        extensions,
        headers,
        Bytes::from(body.to_string()),
    )
    .await;
    (response.status(), slot.get())
}

#[tokio::test]
async fn an_invalid_signature_is_401_and_never_records_the_claimed_sender() {
    // The ordering IS the security property: parse-then-verify would attribute an
    // audit record to whoever the unverified body names. This regression-guards
    // that a future refactor cannot hoist the sender parse above the HMAC check.
    for signature in [None, Some("sha256=deadbeef"), Some("garbage")] {
        let (status, identity) = call(signature, MALICIOUS_BODY).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{signature:?}");
        assert!(
            identity.is_none(),
            "a rejected delivery must record no identity at all ({signature:?})"
        );
    }

    // Same body, signed with the WRONG secret: still 401, still nothing recorded.
    let forged = sign(b"whsec_not_ours", MALICIOUS_BODY.as_bytes());
    let (status, identity) = call(Some(&forged), MALICIOUS_BODY).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(identity.is_none());
}

#[tokio::test]
async fn a_valid_signature_records_the_verified_sender() {
    // The positive half of the same ordering: once the HMAC proves the bytes are
    // GitHub's, the very same fields become the delivery's audit identity.
    let signature = sign(SECRET, MALICIOUS_BODY.as_bytes());
    let (status, identity) = call(Some(&signature), MALICIOUS_BODY).await;
    assert!(status.is_success(), "{status}");
    let identity = identity.expect("a verified delivery records its sender");
    assert_eq!(identity.actor.id, Some(1));
    assert_eq!(identity.principal.id.as_deref(), Some("99"));
}

#[tokio::test]
async fn an_unconfigured_webhook_secret_is_503_and_records_nothing() {
    let (state, _rx) = state_with_reconciler();
    assert!(state.github_app_webhook_secret.is_none());
    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());
    let response = webhook(
        axum::extract::State(state),
        extensions,
        HeaderMap::new(),
        Bytes::from(MALICIOUS_BODY.to_string()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(slot.get().is_none());
}

#[tokio::test]
async fn issue_comment_is_inert_and_never_enqueues() {
    // `/stop` + `/status` were removed with Model A: any `issue_comment` is a
    // 2xx no-op that touches neither the cluster nor the reconcile queue.
    let (state, mut rx) = state_with_reconciler();
    let handled = dispatch_event(&state, "issue_comment", b"{}")
        .await
        .expect("ok");
    assert_eq!(handled.as_str(), "ignored");
    assert!(rx.try_recv().is_err(), "issue_comment must not enqueue");
}

#[tokio::test]
async fn an_unknown_event_is_ignored() {
    let (state, mut rx) = state_with_reconciler();
    let handled = dispatch_event(&state, "ping", b"{}").await.expect("ok");
    assert_eq!(handled.as_str(), "ignored");
    assert!(rx.try_recv().is_err());
}
