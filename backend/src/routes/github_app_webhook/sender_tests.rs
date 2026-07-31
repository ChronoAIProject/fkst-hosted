//! Unit tests for verified webhook sender identity.

use super::*;
use crate::audit::event::{ActorKind, PrincipalKind};
use crate::audit::request::AuditRequestContext;
use crate::audit::AuditIdentitySlot;

/// Install a slot, record from `body`, and return whatever landed.
fn record(body: &str) -> crate::audit::AuditIdentity {
    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());
    record_verified_delivery(&extensions, &HeaderMap::new(), body.as_bytes());
    slot.get().expect("an identity is always recorded")
}

/// Install a full request context, record from `headers` + `body`, and return
/// the correlation the middleware would freeze.
fn correlate(headers: &[(&str, &str)], body: &str) -> crate::audit::event::Correlation {
    let mut extensions = Extensions::new();
    AuditRequestContext::new().install(&mut extensions);
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            axum::http::HeaderValue::from_str(value).expect("header value"),
        );
    }
    record_verified_delivery(&extensions, &map, body.as_bytes());
    AuditRequestContext::from_extensions(&extensions)
        .expect("the context stays installed")
        .freeze()
        .correlation
}

#[test]
fn a_named_sender_keeps_its_immutable_id_and_installation() {
    let identity = record(
        r#"{"action":"opened","sender":{"login":"octocat","id":583231},
            "installation":{"id":146704012}}"#,
    );
    assert_eq!(identity.actor.kind, ActorKind::GithubWebhookSender);
    assert_eq!(identity.actor.id, Some(583231));
    assert_eq!(identity.actor.login.as_deref(), Some("octocat"));
    assert_eq!(identity.principal.kind, PrincipalKind::WebhookHmac);
    assert_eq!(identity.principal.id.as_deref(), Some("146704012"));
}

#[test]
fn every_supported_shape_yields_the_same_identity() {
    // installation, installation_repositories, and issues all carry the two
    // top-level objects, which is exactly why one envelope covers them.
    for body in [
        r#"{"action":"created","installation":{"id":7,"account":{"login":"acme"}},
            "sender":{"login":"alice","id":101}}"#,
        r#"{"action":"added","installation":{"id":7,"account":{"login":"acme"}},
            "repositories_added":[],"sender":{"login":"alice","id":101}}"#,
        r#"{"action":"opened","issue":{"number":9},
            "repository":{"owner":{"login":"acme"},"name":"site"},
            "installation":{"id":7},"sender":{"login":"alice","id":101}}"#,
    ] {
        let identity = record(body);
        assert_eq!(identity.actor.id, Some(101), "{body}");
        assert_eq!(identity.principal.id.as_deref(), Some("7"), "{body}");
    }
}

#[test]
fn an_unsupported_but_validly_signed_event_still_carries_its_sender() {
    let identity = record(r#"{"zen":"Non-blocking is better","sender":{"login":"bob","id":102}}"#);
    assert_eq!(identity.actor.id, Some(102));
    assert_eq!(
        identity.principal.id, None,
        "a delivery with no installation object simply has no principal id"
    );
}

#[test]
fn a_missing_sender_degrades_to_an_unattributable_delivery() {
    let identity = record(r#"{"action":"deleted","installation":{"id":7}}"#);
    assert_eq!(identity.actor.kind, ActorKind::GithubWebhookSender);
    assert_eq!(identity.actor.id, None);
    assert_eq!(identity.actor.login, None);
    assert_eq!(
        identity.actor_id(),
        None,
        "an unattributable delivery must never look like a person"
    );
}

#[test]
fn a_sender_without_an_id_keeps_only_the_login_snapshot() {
    let identity = record(r#"{"sender":{"login":"legacy-fixture"}}"#);
    assert_eq!(identity.actor.login.as_deref(), Some("legacy-fixture"));
    assert_eq!(identity.actor.id, None);
    assert_eq!(
        identity.actor_id(),
        None,
        "a login alone proves no immutable ownership"
    );
}

#[test]
fn a_blank_sender_login_is_treated_as_absent() {
    let identity = record(r#"{"sender":{"login":"   "}}"#);
    assert_eq!(identity.actor.login, None);
}

#[test]
fn an_unparseable_body_yields_an_anonymous_but_authentic_sender() {
    let identity = record("not json at all");
    assert_eq!(identity.actor.kind, ActorKind::GithubWebhookSender);
    assert_eq!(identity.actor.id, None);
    assert_eq!(identity.principal.kind, PrincipalKind::WebhookHmac);
}

/// The two correlation handles a delivery carries: the header GitHub's *Recent
/// Deliveries* page is searchable by, and the installation from the signed body.
#[test]
fn a_verified_delivery_records_its_delivery_and_installation_ids() {
    let correlation = correlate(
        &[("x-github-delivery", "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")],
        r#"{"action":"opened","sender":{"login":"octocat","id":1},"installation":{"id":146704012}}"#,
    );
    assert_eq!(
        correlation.webhook_delivery_id.as_deref(),
        Some("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")
    );
    assert_eq!(correlation.installation_id, Some(146704012));
}

/// The delivery header sits OUTSIDE the signed body, so an over-long or
/// separator-bearing value is dropped rather than sanitized — a missing
/// correlation handle beats a forged one.
#[test]
fn an_unacceptable_delivery_header_is_dropped_rather_than_recorded() {
    let too_long = "x".repeat(crate::audit::validate::limits::WEBHOOK_DELIVERY_ID + 1);
    for value in ["has spaces and; separators", &too_long, ""] {
        let correlation = correlate(&[("x-github-delivery", value)], r#"{"sender":{"id":1}}"#);
        assert_eq!(
            correlation.webhook_delivery_id, None,
            "value {value:?} must not be recorded"
        );
    }
}

#[test]
fn a_delivery_without_the_header_simply_has_no_delivery_id() {
    let correlation = correlate(&[], r#"{"sender":{"id":1},"installation":{"id":7}}"#);
    assert_eq!(correlation.webhook_delivery_id, None);
    assert_eq!(correlation.installation_id, Some(7));
}

#[test]
fn recording_never_retains_the_raw_payload() {
    let identity = record(
        r#"{"sender":{"login":"octocat","id":1},
            "issue":{"title":"secret internal title","body":"do not retain me"}}"#,
    );
    let rendered = format!("{identity:?}");
    assert!(!rendered.contains("secret internal title"), "{rendered}");
    assert!(!rendered.contains("do not retain me"), "{rendered}");
}
