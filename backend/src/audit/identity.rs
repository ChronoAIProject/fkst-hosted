//! The credential-free request identity carried from an extractor to the audit
//! record (epic `AUTH-02`).
//!
//! [`super::event`] already owns the wire-level [`Actor`] / [`Principal`] pair.
//! This module adds the *runtime* half of the same contract: how a verified
//! identity travels from the place that proved it (the [`GithubUser`] extractor,
//! an OAuth callback, the signature-verified webhook) to the outer audit
//! middleware that writes the terminal record after the handler returns.
//!
//! ## Why a slot and not a plain request extension
//!
//! An axum extractor may insert into `Parts::extensions`, but the outer
//! middleware has already moved the `Request` into `next.run(req)` by the time it
//! sees the response — the extension it inserted is gone. So the middleware
//! installs an [`AuditIdentitySlot`] (a cheap `Arc<OnceLock<_>>`) into the request
//! extensions *before* dispatch, and whoever proves identity fills it. The
//! middleware keeps its own clone and reads it afterwards.
//!
//! Filling is deliberately first-write-wins: one request has exactly one
//! initiating identity, so a later handler can never overwrite what an extractor
//! already verified.
//!
//! An UNFILLED slot is not an error — it is the honest representation of an
//! anonymous request. A rejected token, a missing header, and a tampered OAuth
//! state all leave the slot empty, and the middleware reads that as
//! [`AuditIdentity::anonymous`]: the canonical `401`/`400` still carries a
//! complete rejected audit record, just one with no actor to name.
//!
//! ## What may never be in here
//!
//! No bearer/access/refresh/installation token, OAuth code or state, HMAC value,
//! cookie, or token fingerprint. Only the immutable GitHub numeric id, a login
//! snapshot, and the two closed-enum kinds. A hash of a credential is still a
//! credential-derived value and is equally forbidden — matching a fingerprint is
//! enough to correlate a person across records.

use std::sync::{Arc, OnceLock};

use axum::http::Extensions;

use super::event::{Actor, ActorKind, AuthenticationMethod, Principal, PrincipalKind};

/// The initiating identity, in the exact shape the audit event carries.
///
/// A spec-facing alias: the epic names this type `AuditActor`, while the event
/// contract (issue #5666) already shipped it as [`Actor`]. One type, two names,
/// so neither document has to lie.
pub type AuditActor = Actor;

/// The executing identity. See [`AuditActor`] for why the alias exists.
pub type AuditPrincipal = Principal;

/// The verified `(actor, principal)` pair for one request.
///
/// `actor` is the human or webhook sender that initiated the action; `principal`
/// is the credential/service that executed it. They are separate because they
/// genuinely differ: a reconciler lifecycle action has a `system` actor and a
/// `github_app_installation` principal, and conflating them would make a
/// bot-executed action look like a human one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditIdentity {
    pub actor: AuditActor,
    pub principal: AuditPrincipal,
}

impl AuditIdentity {
    /// A caller whose identity was never presented or never verified.
    ///
    /// The only honest representation of a failed authentication: an unverified
    /// token must never be parsed to invent an actor.
    pub fn anonymous() -> Self {
        Self {
            actor: Actor::anonymous(),
            principal: Principal::new(PrincipalKind::Anonymous, None),
        }
    }

    /// A GitHub human verified by trading a bearer token for `GET /user`.
    pub fn github_bearer(id: i64, login: impl Into<String>) -> Self {
        Self {
            actor: Actor::github_user(id, login, AuthenticationMethod::Bearer),
            principal: Principal::new(PrincipalKind::GithubUserToken, None),
        }
    }

    /// A GitHub human verified after an OAuth exchange resolved `GET /user`.
    ///
    /// Distinct from [`Self::github_bearer`] because the *credential* differs: an
    /// OAuth session is refreshable and revocable independently of a
    /// caller-supplied bearer token, and incident response needs to tell them
    /// apart.
    pub fn github_oauth(id: i64, login: impl Into<String>) -> Self {
        Self {
            actor: Actor::github_user(id, login, AuthenticationMethod::Oauth),
            principal: Principal::new(PrincipalKind::OauthSession, None),
        }
    }

    /// The sender of a webhook whose HMAC over the raw bytes already verified.
    ///
    /// `id` is the immutable GitHub numeric id and is what makes the sender a
    /// person; `login` is a display snapshot. Both are optional because a
    /// delivery may name no resolvable sender (GitHub omits it on some shapes)
    /// or may carry a login with no id — the actor then keeps its
    /// `github_webhook_sender` kind without an id, which the event contract
    /// treats as non-human traffic rather than guessing. Never call this before
    /// signature verification: sender fields in an unverified body are
    /// attacker-controlled.
    pub fn webhook_sender(
        id: Option<i64>,
        login: Option<String>,
        installation_id: Option<i64>,
    ) -> Self {
        Self {
            actor: Actor {
                kind: ActorKind::GithubWebhookSender,
                id,
                login,
                authentication: AuthenticationMethod::WebhookHmac,
            },
            principal: Principal::new(
                PrincipalKind::WebhookHmac,
                installation_id.map(|id| id.to_string()),
            ),
        }
    }

    /// The control plane acting on its own behalf through an installation token.
    pub fn reconciler(installation_id: Option<i64>) -> Self {
        Self {
            actor: Actor::system(),
            principal: Principal::new(
                match installation_id {
                    Some(_) => PrincipalKind::GithubAppInstallation,
                    None => PrincipalKind::Reconciler,
                },
                installation_id.map(|id| id.to_string()),
            ),
        }
    }

    /// The verified numeric id, when the actor is a human GitHub identity.
    ///
    /// This — never the login — is what personal activity scope compares against.
    pub fn actor_id(&self) -> Option<i64> {
        self.actor
            .kind
            .is_human()
            .then_some(self.actor.id)
            .flatten()
    }
}

/// A write-once cell an outer middleware installs into the request extensions so
/// the identity proven *inside* the handler is readable *after* it returns.
///
/// Cloning shares the same cell, which is the entire point: the middleware and
/// the extractor hold independent handles onto one value.
#[derive(Clone, Default)]
pub struct AuditIdentitySlot {
    inner: Arc<OnceLock<AuditIdentity>>,
}

impl AuditIdentitySlot {
    /// An empty slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the verified identity. First write wins; later writes are dropped
    /// because a request has exactly one initiating identity.
    pub fn record(&self, identity: AuditIdentity) {
        // `set` returns the rejected value on a second write; discarding it is
        // the intended behaviour, not a swallowed error.
        let _ = self.inner.set(identity);
    }

    /// The recorded identity, if anything proved one.
    pub fn get(&self) -> Option<AuditIdentity> {
        self.inner.get().cloned()
    }
}

impl std::fmt::Debug for AuditIdentitySlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Render only whether the slot is filled and the bounded actor kind: a
        // `{:?}` of a request/state must never dump a login or a numeric id into
        // a log line that was not asked for one.
        f.debug_struct("AuditIdentitySlot")
            .field(
                "actor_kind",
                &self
                    .inner
                    .get()
                    .map(|identity| identity.actor.kind.as_str()),
            )
            .finish()
    }
}

/// Record `identity` into the slot carried by `extensions`, if one is installed.
///
/// A no-op when no middleware installed a slot — every identity-proving site can
/// call this unconditionally, so auditing can be switched on without editing
/// each of them again.
pub fn record_identity(extensions: &Extensions, identity: AuditIdentity) {
    if let Some(slot) = extensions.get::<AuditIdentitySlot>() {
        slot.record(identity);
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
