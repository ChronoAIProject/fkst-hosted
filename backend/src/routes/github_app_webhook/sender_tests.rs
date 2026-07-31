//! Unit tests for verified webhook sender identity.

use super::*;
use crate::audit::event::{ActorKind, PrincipalKind};
use crate::audit::AuditIdentitySlot;

/// Install a slot, record from `body`, and return whatever landed.
fn record(body: &str) -> crate::audit::AuditIdentity {
    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());
    record_verified_sender(&extensions, body.as_bytes());
    slot.get().expect("an identity is always recorded")
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
