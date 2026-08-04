//! Unit tests for the credential-free request identity.

use super::*;

/// A canary that would be a catastrophic leak if any identity type retained it.
const TOKEN_CANARY: &str = "gho_canary_never_store_me";

#[test]
fn anonymous_carries_no_identity_at_all() {
    let identity = AuditIdentity::anonymous();
    assert_eq!(identity.actor.kind, ActorKind::Anonymous);
    assert_eq!(identity.actor.id, None);
    assert_eq!(identity.actor.login, None);
    assert_eq!(identity.actor.authentication, AuthenticationMethod::None);
    assert_eq!(identity.principal.kind, PrincipalKind::Anonymous);
    assert_eq!(identity.principal.id, None);
    assert_eq!(identity.actor_id(), None);
}

#[test]
fn bearer_and_oauth_humans_differ_only_in_credential() {
    let bearer = AuditIdentity::github_bearer(42, "alice");
    let oauth = AuditIdentity::github_oauth(42, "alice");
    assert_eq!(bearer.actor.id, oauth.actor.id);
    assert_eq!(bearer.actor.login, oauth.actor.login);
    assert_eq!(bearer.actor.authentication, AuthenticationMethod::Bearer);
    assert_eq!(oauth.actor.authentication, AuthenticationMethod::Oauth);
    assert_eq!(bearer.principal.kind, PrincipalKind::GithubUserToken);
    assert_eq!(oauth.principal.kind, PrincipalKind::OauthSession);
    assert_eq!(bearer.actor_id(), Some(42));
    assert_eq!(oauth.actor_id(), Some(42));
}

#[test]
fn webhook_sender_keeps_the_id_when_present_and_none_when_absent() {
    let named = AuditIdentity::webhook_sender(Some(7), Some("octocat".to_string()), Some(99));
    assert_eq!(named.actor.kind, ActorKind::GithubWebhookSender);
    assert_eq!(named.actor.id, Some(7));
    assert_eq!(named.actor.login.as_deref(), Some("octocat"));
    assert_eq!(named.principal.kind, PrincipalKind::WebhookHmac);
    assert_eq!(named.principal.id.as_deref(), Some("99"));
    assert_eq!(named.actor_id(), Some(7));

    let login_only = AuditIdentity::webhook_sender(None, Some("octocat".to_string()), Some(99));
    assert_eq!(login_only.actor.id, None);
    assert_eq!(login_only.actor.login.as_deref(), Some("octocat"));
    assert_eq!(
        login_only.actor_id(),
        None,
        "a login without an immutable id proves no ownership"
    );

    let unnamed = AuditIdentity::webhook_sender(None, None, None);
    assert_eq!(unnamed.actor.kind, ActorKind::GithubWebhookSender);
    assert_eq!(unnamed.actor.id, None);
    assert_eq!(unnamed.actor.login, None);
    assert_eq!(unnamed.principal.id, None);
    assert_eq!(
        unnamed.actor_id(),
        None,
        "a sender-less delivery must not look like a person"
    );
}

#[test]
fn reconciler_is_system_actor_with_a_credential_free_principal_id() {
    let with_installation = AuditIdentity::reconciler(Some(1234));
    assert_eq!(with_installation.actor.kind, ActorKind::System);
    assert_eq!(
        with_installation.principal.kind,
        PrincipalKind::GithubAppInstallation
    );
    assert_eq!(with_installation.principal.id.as_deref(), Some("1234"));
    assert_eq!(
        with_installation.actor_id(),
        None,
        "a system actor never claims a human id"
    );

    let loop_only = AuditIdentity::reconciler(None);
    assert_eq!(loop_only.principal.kind, PrincipalKind::Reconciler);
    assert_eq!(loop_only.principal.id, None);
}

#[test]
fn slot_is_write_once_and_shared_between_clones() {
    let slot = AuditIdentitySlot::new();
    assert!(slot.get().is_none());

    let handle = slot.clone();
    handle.record(AuditIdentity::github_bearer(42, "alice"));
    let seen = slot.get().expect("a write through a clone is visible");
    assert_eq!(seen.actor.id, Some(42));

    // A second write must not be able to relabel the request's initiator.
    handle.record(AuditIdentity::github_bearer(999, "mallory"));
    assert_eq!(
        slot.get().expect("still filled").actor.id,
        Some(42),
        "first write wins"
    );
}

#[test]
fn record_identity_is_a_no_op_without_an_installed_slot() {
    let extensions = Extensions::new();
    // Must not panic: every identity-proving site calls this unconditionally.
    record_identity(&extensions, AuditIdentity::github_bearer(42, "alice"));
    assert!(extensions.get::<AuditIdentitySlot>().is_none());
}

#[test]
fn record_identity_fills_an_installed_slot() {
    let mut extensions = Extensions::new();
    let slot = AuditIdentitySlot::new();
    extensions.insert(slot.clone());

    record_identity(&extensions, AuditIdentity::github_oauth(7, "octocat"));
    let seen = slot.get().expect("slot filled through the extensions");
    assert_eq!(seen.actor.id, Some(7));
    assert_eq!(seen.principal.kind, PrincipalKind::OauthSession);
}

#[test]
fn debug_output_exposes_neither_identity_values_nor_credentials() {
    let slot = AuditIdentitySlot::new();
    slot.record(AuditIdentity::github_bearer(583231, TOKEN_CANARY));
    let rendered = format!("{slot:?}");
    assert!(rendered.contains("github_user"), "{rendered}");
    assert!(!rendered.contains(TOKEN_CANARY), "{rendered}");
    assert!(!rendered.contains("583231"), "{rendered}");
}

#[test]
fn no_identity_constructor_has_anywhere_to_put_a_credential() {
    // A structural canary: every field of every constructed identity is either a
    // closed enum, a numeric id, or a login snapshot. If a future change adds a
    // credential-bearing field, this rendering starts carrying it.
    for identity in [
        AuditIdentity::anonymous(),
        AuditIdentity::github_bearer(1, "alice"),
        AuditIdentity::github_oauth(2, "bob"),
        AuditIdentity::webhook_sender(Some(3), Some("carol".to_string()), Some(5)),
        AuditIdentity::reconciler(Some(6)),
    ] {
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains(TOKEN_CANARY), "{rendered}");
        assert!(!rendered.to_lowercase().contains("token_"), "{rendered}");
    }
}
