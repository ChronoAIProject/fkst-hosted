//! The versioned audit event contract.
//!
//! One completed product request produces exactly one [`ApiRequestCompletedV1`].
//! This module owns the domain model and its construction invariants; validation
//! lives in [`super::validate`] and the PostHog wire projection in
//! [`super::projection`], so transport concerns never leak into the contract.
//!
//! ## Why the top-level `actor_id` / `session_id` duplicate their nested homes
//!
//! Authorization for the read side (epic `AUTH-03`) is a *source-level*
//! predicate: fixed HogQL and relay SQL must be able to filter on the verified
//! actor without parsing nested JSON, or a regular user's scope would depend on a
//! query-time JSON path that is easy to get subtly wrong. So the canonical filter
//! fields are first-class on the event and on the flattened PostHog properties,
//! while the structured `actor`/`principal`/`correlation` objects are preserved
//! for display and forward compatibility. [`super::validate`] rejects any record
//! whose canonical and nested identifiers disagree, which is what makes the flat
//! field trustworthy.
//!
//! ## Identity, not credentials
//!
//! `actor` is who initiated the call; `principal` is which credential/identity
//! actually executed it (epic `AUTH-02`). The GitHub numeric id is authoritative
//! and the login is a mutable historical snapshot — never an authorization input.
//! No field on this event may ever carry a token, cookie, OAuth material, raw
//! body, URI/query string, or free error text.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

/// Schema version stamped on every record. Bump only with a new `…V2` type.
pub const SCHEMA_VERSION: u32 = 1;

/// The PostHog event name. Stable: dashboards, HogQL, and the relay all key on it.
pub const EVENT_NAME: &str = "fkst api request completed";

/// The `operation_id` recorded for a request that matched no documented route.
/// The raw path/query is deliberately never recorded in its place.
pub const UNMATCHED_OPERATION_ID: &str = "<unmatched>";

/// Namespace for the deterministic UUIDv5 event id. A fixed random constant, so
/// the same terminal request always derives the same id in every replica and on
/// every retry — which is what makes PostHog's UUID deduplication work for
/// at-least-once delivery (epic `AUD-07`).
const EVENT_ID_NAMESPACE: Uuid = Uuid::from_u128(0x0f5b_9a41_7d2e_4c8b_9c31_6b0a_2f77_51d4);

/// Who initiated the request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorKind {
    /// A GitHub user verified through the deployment's identity path.
    GithubUser,
    /// The GitHub user named as the sender of a signature-verified webhook.
    GithubWebhookSender,
    /// No identity was presented, or it could not be verified.
    Anonymous,
    /// A non-human machine caller (deployment tooling, probes with credentials).
    Service,
    /// The control plane acting on its own behalf (reconciler, background loop).
    System,
}

/// How the actor authenticated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticationMethod {
    Bearer,
    Oauth,
    WebhookHmac,
    Internal,
    None,
}

/// Which identity actually executed the work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalKind {
    GithubUserToken,
    OauthSession,
    GithubAppInstallation,
    WebhookHmac,
    Reconciler,
    Anonymous,
    None,
}

/// The terminal classification of the request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    Success,
    Redirect,
    ClientError,
    ServerError,
    Timeout,
    /// Rejected before the product handler ran (auth or leader-readiness gate).
    Rejected,
    /// No response was ever produced (process abort/partition). `status_code`
    /// stays `None`: no system may fabricate a status it never returned.
    Incomplete,
}

/// Whether the safe-argument contract could produce arguments for this call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgumentsParseStatus {
    Parsed,
    Invalid,
    NotApplicable,
    Unavailable,
}

macro_rules! as_str_impl {
    ($ty:ty { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $ty {
            /// The stable wire string. Bounded closed enum, so it is also the
            /// only value ever safe to use as a metric label.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(<$ty>::$variant => $text),+
                }
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

as_str_impl!(ActorKind {
    GithubUser => "github_user",
    GithubWebhookSender => "github_webhook_sender",
    Anonymous => "anonymous",
    Service => "service",
    System => "system",
});

as_str_impl!(AuthenticationMethod {
    Bearer => "bearer",
    Oauth => "oauth",
    WebhookHmac => "webhook_hmac",
    Internal => "internal",
    None => "none",
});

as_str_impl!(PrincipalKind {
    GithubUserToken => "github_user_token",
    OauthSession => "oauth_session",
    GithubAppInstallation => "github_app_installation",
    WebhookHmac => "webhook_hmac",
    Reconciler => "reconciler",
    Anonymous => "anonymous",
    None => "none",
});

as_str_impl!(AuditOutcome {
    Success => "success",
    Redirect => "redirect",
    ClientError => "client_error",
    ServerError => "server_error",
    Timeout => "timeout",
    Rejected => "rejected",
    Incomplete => "incomplete",
});

as_str_impl!(ArgumentsParseStatus {
    Parsed => "parsed",
    Invalid => "invalid",
    NotApplicable => "not_applicable",
    Unavailable => "unavailable",
});

impl ActorKind {
    /// True for the two kinds that have a real GitHub person behind them. Only
    /// these may carry an immutable numeric id, and only these get a PostHog
    /// person profile. A verified [`ActorKind::GithubUser`] must carry that id;
    /// a [`ActorKind::GithubWebhookSender`] may lack it when GitHub's payload
    /// named no resolvable sender, and then behaves like non-human traffic.
    pub fn is_human(self) -> bool {
        matches!(self, ActorKind::GithubUser | ActorKind::GithubWebhookSender)
    }
}

/// The initiating identity. `id` is the immutable GitHub numeric id; `login` is a
/// historical display snapshot that must never be used to authorize anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: Option<i64>,
    pub login: Option<String>,
    pub authentication: AuthenticationMethod,
}

impl Actor {
    /// An unattributed caller: no identity presented or none verifiable.
    pub fn anonymous() -> Self {
        Self {
            kind: ActorKind::Anonymous,
            id: None,
            login: None,
            authentication: AuthenticationMethod::None,
        }
    }

    /// A verified GitHub human.
    pub fn github_user(id: i64, login: impl Into<String>, method: AuthenticationMethod) -> Self {
        Self {
            kind: ActorKind::GithubUser,
            id: Some(id),
            login: Some(login.into()),
            authentication: method,
        }
    }

    /// The control plane acting on its own behalf.
    pub fn system() -> Self {
        Self {
            kind: ActorKind::System,
            id: None,
            login: None,
            authentication: AuthenticationMethod::Internal,
        }
    }
}

/// The executing identity. `id` is a bounded identifier (installation id, bot
/// login, loop name) — never a credential, and never a token fingerprint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Principal {
    pub kind: PrincipalKind,
    pub id: Option<String>,
}

impl Principal {
    pub fn new(kind: PrincipalKind, id: Option<String>) -> Self {
        Self { kind, id }
    }

    pub fn none() -> Self {
        Self {
            kind: PrincipalKind::None,
            id: None,
        }
    }
}

/// Correlation keys (epic `AUD-05`). Every field is optional: a record is still
/// valid — and still complete — when a call has no session or repository context.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Correlation {
    pub session_id: Option<String>,
    /// `owner/name`.
    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    pub trigger_issue: Option<i64>,
    /// GitHub's `X-GitHub-Delivery` UUID.
    pub webhook_delivery_id: Option<String>,
}

/// The emitting deployment. Neither field is a secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceIdentity {
    pub version: String,
    pub environment: String,
}

/// Route/operation identity of the request. Grouped so the constructor keeps a
/// readable signature as the contract grows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestIdentity {
    /// The propagated `X-Request-Id`.
    pub request_id: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// The normalized matched route template (`/api/v1/logs/{session_id}`) —
    /// never the raw URI, which would carry query values.
    pub route_template: String,
    /// The generated OpenAPI operation id, or [`UNMATCHED_OPERATION_ID`].
    pub operation_id: String,
}

/// Start/completion instants. Duration is derived, never supplied, so the two can
/// never disagree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestTiming {
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

/// The terminal result of the request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestResult {
    /// `None` only for an incomplete/aborted record.
    pub status_code: Option<u16>,
    pub outcome: AuditOutcome,
    /// A stable application error code (`forbidden`, `not_found`, …). Never
    /// error text.
    pub error_code: Option<String>,
}

/// One completed product request, schema version 1.
///
/// Construct through [`ApiRequestCompletedV1::new`] (plus the `with_*` builders)
/// so the canonical/nested identifier invariants hold by construction; hand-built
/// values are still checked by [`super::validate::validate`].
#[derive(Clone, Debug, PartialEq)]
pub struct ApiRequestCompletedV1 {
    pub schema_version: u32,
    /// Deterministic UUIDv5 over the request's identity + start instant, so a
    /// retry of the same record deduplicates in PostHog.
    pub event_id: Uuid,
    pub request_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub method: String,
    pub route_template: String,
    pub operation_id: String,
    /// The allowlisted, typed arguments produced by the safe-argument contract.
    pub arguments: serde_json::Map<String, serde_json::Value>,
    pub arguments_parse_status: ArgumentsParseStatus,
    /// Canonical, authoritative actor id (see the module docs). Mirrors
    /// `actor.id`; `None` for every non-human actor.
    pub actor_id: Option<i64>,
    pub actor: Actor,
    pub principal: Principal,
    pub status_code: Option<u16>,
    pub outcome: AuditOutcome,
    pub error_code: Option<String>,
    pub duration_ms: u64,
    /// Canonical session id for lifecycle/session timelines. Mirrors
    /// `correlation.session_id`.
    pub session_id: Option<String>,
    pub correlation: Correlation,
    pub service: ServiceIdentity,
}

impl ApiRequestCompletedV1 {
    /// Build a record with the canonical identifiers derived from the nested
    /// ones and a deterministic event id.
    pub fn new(
        identity: RequestIdentity,
        timing: RequestTiming,
        actor: Actor,
        principal: Principal,
        result: RequestResult,
        service: ServiceIdentity,
    ) -> Self {
        let event_id = derive_event_id(&identity, timing.started_at);
        let duration_ms = u64::try_from(
            (timing.completed_at - timing.started_at)
                .num_milliseconds()
                .max(0),
        )
        .unwrap_or(u64::MAX);
        Self {
            schema_version: SCHEMA_VERSION,
            event_id,
            request_id: identity.request_id,
            started_at: timing.started_at,
            completed_at: timing.completed_at,
            method: identity.method,
            route_template: identity.route_template,
            operation_id: identity.operation_id,
            arguments: serde_json::Map::new(),
            arguments_parse_status: ArgumentsParseStatus::NotApplicable,
            // Only a human actor may claim an id; a service/system record must
            // never appear to belong to a person.
            actor_id: actor.kind.is_human().then_some(actor.id).flatten(),
            actor,
            principal,
            status_code: result.status_code,
            outcome: result.outcome,
            error_code: result.error_code,
            duration_ms,
            session_id: None,
            correlation: Correlation::default(),
            service,
        }
    }

    /// Attach the safe arguments and how they were obtained.
    pub fn with_arguments(
        mut self,
        arguments: serde_json::Map<String, serde_json::Value>,
        status: ArgumentsParseStatus,
    ) -> Self {
        self.arguments = arguments;
        self.arguments_parse_status = status;
        self
    }

    /// Attach correlation keys, keeping the canonical `session_id` in step.
    pub fn with_correlation(mut self, correlation: Correlation) -> Self {
        self.session_id = correlation.session_id.clone();
        self.correlation = correlation;
        self
    }

    /// RFC3339 UTC with millisecond precision, the exact form sent to PostHog.
    pub fn started_at_rfc3339(&self) -> String {
        self.started_at.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// RFC3339 UTC with millisecond precision, the exact form sent to PostHog.
    pub fn completed_at_rfc3339(&self) -> String {
        self.completed_at
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

/// Derive the deterministic event id for a terminal record.
///
/// The key includes the schema version, so a future schema cannot collide with a
/// v1 record for the same request, and the start instant, so a client that reuses
/// an `X-Request-Id` across calls still yields distinct events.
pub fn derive_event_id(identity: &RequestIdentity, started_at: DateTime<Utc>) -> Uuid {
    let key = format!(
        "{SCHEMA_VERSION}|{}|{}|{}|{}|{}",
        identity.request_id,
        started_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
        identity.method,
        identity.route_template,
        identity.operation_id,
    );
    Uuid::new_v5(&EVENT_ID_NAMESPACE, key.as_bytes())
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
