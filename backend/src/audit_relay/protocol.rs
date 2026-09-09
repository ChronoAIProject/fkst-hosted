//! The internal relay protocol: the exact bytes the control plane and the
//! durable relay exchange.
//!
//! ```text
//! POST /internal/v1/audit/request-starts            RequestStartV1  -> DurableAck
//! PUT  /internal/v1/audit/requests/{id}/completion  RequestCompletionV1 -> DurableAck
//! POST /internal/v1/audit/events                    LifecycleEventV1 -> DurableAck
//! GET  /internal/v1/audit/records                   RecordsQueryV1  -> RecordsPageV1
//! ```
//!
//! ## Why the wire types are their own structs
//!
//! [`crate::audit::event::ApiRequestCompletedV1`] and
//! [`crate::audit::lifecycle::SandboxLifecycleV1`] are DOMAIN types: they carry
//! `chrono` instants, `Uuid`s, and closed Rust enums, and they are deliberately
//! not `serde`-derived, because a `Deserialize` on the domain type would make
//! "whatever arrived on the wire" indistinguishable from "a record this process
//! constructed and validated". The mirrors here are the untrusted edge: they
//! parse into the domain type through [`RequestCompletionV1::to_domain`] /
//! [`LifecycleEventV1::to_domain`], which reject an unknown enum spelling, a
//! malformed timestamp, or a non-object argument bag — and the relay then runs
//! the SAME [`crate::audit::validate`] pass the capture sink runs before
//! anything is committed.
//!
//! ## What may never appear here
//!
//! There is no field for a raw body, a URI or query string, a header, a
//! credential, or an upstream error string, and the relay never stores one:
//! the arguments bag was already filtered by the operation's sealed safe-argument
//! contract on the control plane (see [`crate::audit::arguments`]) and is stored
//! verbatim, not re-derived. Bearer tokens ride the `Authorization` header and
//! never appear in any struct in this module.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::audit::event::{
    Actor, ActorKind, ApiRequestCompletedV1, ArgumentsParseStatus, AuditOutcome,
    AuthenticationMethod, Correlation, Principal, PrincipalKind, ServiceIdentity,
};
use crate::audit::lifecycle::{
    LifecycleAction, LifecycleAttribution, LifecycleCorrelation, LifecycleReason, LifecycleRuntime,
    SandboxLifecycleV1,
};
use crate::runtime_identity::RuntimeBackendKind;

/// Path of the request-start registration endpoint.
pub const REQUEST_STARTS_PATH: &str = "/internal/v1/audit/request-starts";
/// Path template of the completion endpoint (`{event_id}` is the idempotency key).
pub const REQUEST_COMPLETION_PATH: &str = "/internal/v1/audit/requests/{event_id}/completion";
/// Path of the non-request (lifecycle) event endpoint.
pub const EVENTS_PATH: &str = "/internal/v1/audit/events";
/// Path of the scoped read endpoint.
pub const RECORDS_PATH: &str = "/internal/v1/audit/records";

/// Wire schema version of every body in this module.
pub const PROTOCOL_SCHEMA_VERSION: u32 = 1;

/// Stable code returned when an event id is replayed with different immutable
/// content. It is a CONFLICT, never an overwrite: audit history is append-only.
pub const EVENT_ID_CONFLICT: &str = "event_id_conflict";

/// Why a submitted body could not become a durable record. Every variant is
/// developer-facing text about a FIELD, never about a value.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProtocolError {
    #[error("field `{field}` is invalid: {reason}")]
    Invalid {
        field: &'static str,
        reason: &'static str,
    },
    #[error("unsupported schema_version {0}")]
    UnsupportedSchema(u32),
}

impl ProtocolError {
    fn invalid(field: &'static str, reason: &'static str) -> Self {
        Self::Invalid { field, reason }
    }
}

/// RFC3339 UTC with millisecond precision — the one timestamp form on this wire,
/// matching what the capture projection writes so a relay row and a PostHog row
/// compare byte-for-byte.
pub fn format_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_instant(field: &'static str, raw: &str) -> Result<DateTime<Utc>, ProtocolError> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| ProtocolError::invalid(field, "must be an RFC3339 UTC timestamp"))
}

fn parse_uuid(field: &'static str, raw: &str) -> Result<Uuid, ProtocolError> {
    Uuid::parse_str(raw.trim()).map_err(|_| ProtocolError::invalid(field, "must be a UUID"))
}

fn parse_enum<T>(
    field: &'static str,
    raw: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, ProtocolError> {
    parse(raw).ok_or_else(|| ProtocolError::invalid(field, "is not a value of the closed enum"))
}

/// Registration of an in-flight request, sent BEFORE its handler runs.
///
/// Every field is immutable for the life of the record: a replay carrying
/// different content is [`EVENT_ID_CONFLICT`], never an update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestStartV1 {
    pub schema_version: u32,
    pub event_id: String,
    pub request_id: String,
    pub started_at: String,
    pub method: String,
    pub route_template: String,
    pub operation_id: String,
    pub service_version: String,
    pub deployment_environment: String,
    /// When the control plane stops waiting for a completion. Past this instant
    /// plus the configured grace, the relay synthesizes an `incomplete` terminal
    /// projection — it never invents a status.
    pub completion_deadline_at: String,
}

/// The immutable identity a completion must agree with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartIdentity {
    pub event_id: Uuid,
    pub request_id: String,
    pub started_at: DateTime<Utc>,
    pub method: String,
    pub route_template: String,
    pub operation_id: String,
    /// The parsed deadline. Carried so storage writes the CANONICAL rendering of
    /// it: the column is compared as text by the overdue sweep, so a caller's
    /// equally-valid alternative rendering would sort wrong.
    pub completion_deadline_at: DateTime<Utc>,
}

impl RequestStartV1 {
    /// Parse and bound-check the start, yielding its immutable identity.
    pub fn to_identity(&self) -> Result<StartIdentity, ProtocolError> {
        if self.schema_version != PROTOCOL_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedSchema(self.schema_version));
        }
        let event_id = parse_uuid("event_id", &self.event_id)?;
        let started_at = parse_instant("started_at", &self.started_at)?;
        let deadline = parse_instant("completion_deadline_at", &self.completion_deadline_at)?;
        if deadline < started_at {
            return Err(ProtocolError::invalid(
                "completion_deadline_at",
                "must not precede started_at",
            ));
        }
        Ok(StartIdentity {
            event_id,
            request_id: self.request_id.clone(),
            started_at,
            method: self.method.clone(),
            route_template: self.route_template.clone(),
            operation_id: self.operation_id.clone(),
            completion_deadline_at: deadline,
        })
    }

    /// The deadline instant, already validated by [`RequestStartV1::to_identity`].
    pub fn deadline(&self) -> Result<DateTime<Utc>, ProtocolError> {
        parse_instant("completion_deadline_at", &self.completion_deadline_at)
    }
}

/// The initiating identity, on the wire.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorV1 {
    pub kind: String,
    pub id: Option<i64>,
    pub login: Option<String>,
    pub authentication: String,
}

/// The executing identity, on the wire.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrincipalV1 {
    pub kind: String,
    pub id: Option<String>,
}

/// Correlation keys, on the wire.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrelationV1 {
    pub session_id: Option<String>,
    pub repo_full_name: Option<String>,
    pub installation_id: Option<i64>,
    pub trigger_issue: Option<i64>,
    pub webhook_delivery_id: Option<String>,
    /// Present on lifecycle events only (an API record carries its own
    /// top-level `request_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// The terminal record of one API request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RequestCompletionV1 {
    pub schema_version: u32,
    pub event_id: String,
    pub request_id: String,
    pub started_at: String,
    pub completed_at: String,
    pub method: String,
    pub route_template: String,
    pub operation_id: String,
    #[serde(default)]
    pub arguments: Map<String, Value>,
    pub arguments_parse_status: String,
    pub actor_id: Option<i64>,
    pub actor: ActorV1,
    pub principal: PrincipalV1,
    pub status_code: Option<u16>,
    pub outcome: String,
    pub error_code: Option<String>,
    pub duration_ms: u64,
    pub session_id: Option<String>,
    #[serde(default)]
    pub correlation: CorrelationV1,
    pub service_version: String,
    pub deployment_environment: String,
}

impl RequestCompletionV1 {
    /// Project a domain record onto the wire.
    pub fn from_domain(event: &ApiRequestCompletedV1) -> Self {
        Self {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event.event_id.to_string(),
            request_id: event.request_id.clone(),
            started_at: event.started_at_rfc3339(),
            completed_at: event.completed_at_rfc3339(),
            method: event.method.clone(),
            route_template: event.route_template.clone(),
            operation_id: event.operation_id.clone(),
            arguments: event.arguments.clone(),
            arguments_parse_status: event.arguments_parse_status.as_str().to_string(),
            actor_id: event.actor_id,
            actor: ActorV1 {
                kind: event.actor.kind.as_str().to_string(),
                id: event.actor.id,
                login: event.actor.login.clone(),
                authentication: event.actor.authentication.as_str().to_string(),
            },
            principal: PrincipalV1 {
                kind: event.principal.kind.as_str().to_string(),
                id: event.principal.id.clone(),
            },
            status_code: event.status_code,
            outcome: event.outcome.as_str().to_string(),
            error_code: event.error_code.clone(),
            duration_ms: event.duration_ms,
            session_id: event.session_id.clone(),
            correlation: CorrelationV1 {
                session_id: event.correlation.session_id.clone(),
                repo_full_name: event.correlation.repo_full_name.clone(),
                installation_id: event.correlation.installation_id,
                trigger_issue: event.correlation.trigger_issue,
                webhook_delivery_id: event.correlation.webhook_delivery_id.clone(),
                request_id: None,
            },
            service_version: event.service.version.clone(),
            deployment_environment: event.service.environment.clone(),
        }
    }

    /// Parse the wire form back into the domain record.
    ///
    /// Structural only: the caller runs [`crate::audit::validate::validate`] on
    /// the result, so the contract that governs a locally-built record governs a
    /// submitted one identically.
    pub fn to_domain(&self) -> Result<ApiRequestCompletedV1, ProtocolError> {
        if self.schema_version != PROTOCOL_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedSchema(self.schema_version));
        }
        Ok(ApiRequestCompletedV1 {
            schema_version: crate::audit::event::SCHEMA_VERSION,
            event_id: parse_uuid("event_id", &self.event_id)?,
            request_id: self.request_id.clone(),
            started_at: parse_instant("started_at", &self.started_at)?,
            completed_at: parse_instant("completed_at", &self.completed_at)?,
            method: self.method.clone(),
            route_template: self.route_template.clone(),
            operation_id: self.operation_id.clone(),
            arguments: self.arguments.clone(),
            arguments_parse_status: parse_enum(
                "arguments_parse_status",
                &self.arguments_parse_status,
                ArgumentsParseStatus::parse,
            )?,
            actor_id: self.actor_id,
            actor: Actor {
                kind: parse_enum("actor.kind", &self.actor.kind, ActorKind::parse)?,
                id: self.actor.id,
                login: self.actor.login.clone(),
                authentication: parse_enum(
                    "actor.authentication",
                    &self.actor.authentication,
                    AuthenticationMethod::parse,
                )?,
            },
            principal: Principal {
                kind: parse_enum("principal.kind", &self.principal.kind, PrincipalKind::parse)?,
                id: self.principal.id.clone(),
            },
            status_code: self.status_code,
            outcome: parse_enum("outcome", &self.outcome, AuditOutcome::parse)?,
            error_code: self.error_code.clone(),
            duration_ms: self.duration_ms,
            session_id: self.session_id.clone(),
            correlation: Correlation {
                session_id: self.correlation.session_id.clone(),
                repo_full_name: self.correlation.repo_full_name.clone(),
                installation_id: self.correlation.installation_id,
                trigger_issue: self.correlation.trigger_issue,
                webhook_delivery_id: self.correlation.webhook_delivery_id.clone(),
            },
            service: ServiceIdentity {
                version: self.service_version.clone(),
                environment: self.deployment_environment.clone(),
            },
        })
    }
}

/// One sandbox lifecycle transition, on the wire.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LifecycleEventV1 {
    pub schema_version: u32,
    pub event_id: String,
    pub occurred_at: String,
    pub lifecycle_action: String,
    pub actor: ActorV1,
    pub principal: PrincipalV1,
    pub session_id: String,
    pub backend: String,
    pub runtime_id: Option<String>,
    pub runtime_created_at: Option<String>,
    pub incarnation_hint: Option<String>,
    pub creator_id: Option<i64>,
    pub creator_login: Option<String>,
    pub trigger_author_id: Option<i64>,
    pub trigger_author_login: Option<String>,
    #[serde(default)]
    pub correlation: CorrelationV1,
    pub reason_code: Option<String>,
    pub service_version: String,
    pub deployment_environment: String,
}

impl LifecycleEventV1 {
    /// Project a domain record onto the wire.
    pub fn from_domain(event: &SandboxLifecycleV1) -> Self {
        Self {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event.event_id.to_string(),
            occurred_at: event.occurred_at_rfc3339(),
            lifecycle_action: event.action.as_str().to_string(),
            actor: ActorV1 {
                kind: event.actor.kind.as_str().to_string(),
                id: event.actor.id,
                login: event.actor.login.clone(),
                authentication: event.actor.authentication.as_str().to_string(),
            },
            principal: PrincipalV1 {
                kind: event.principal.kind.as_str().to_string(),
                id: event.principal.id.clone(),
            },
            session_id: event.session_id.clone(),
            backend: event.backend.as_str().to_string(),
            runtime_id: event.runtime.runtime_id.clone(),
            runtime_created_at: event.runtime.created_at.map(format_instant),
            incarnation_hint: event.runtime.incarnation_hint.clone(),
            creator_id: event.attribution.creator_id,
            creator_login: event.attribution.creator_login.clone(),
            trigger_author_id: event.attribution.trigger_author_id,
            trigger_author_login: event.attribution.trigger_author_login.clone(),
            correlation: CorrelationV1 {
                session_id: Some(event.session_id.clone()),
                repo_full_name: event.correlation.repo_full_name.clone(),
                installation_id: event.correlation.installation_id,
                trigger_issue: event.correlation.trigger_issue,
                webhook_delivery_id: None,
                request_id: event.correlation.request_id.clone(),
            },
            reason_code: event.reason_code.map(|reason| reason.as_str().to_string()),
            service_version: event.service.version.clone(),
            deployment_environment: event.service.environment.clone(),
        }
    }

    /// Parse the wire form back into the domain record. The caller then runs
    /// [`crate::audit::lifecycle_validate::validate_lifecycle`].
    pub fn to_domain(&self) -> Result<SandboxLifecycleV1, ProtocolError> {
        if self.schema_version != PROTOCOL_SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedSchema(self.schema_version));
        }
        let reason_code = self
            .reason_code
            .as_deref()
            .map(|raw| parse_enum("reason_code", raw, LifecycleReason::parse))
            .transpose()?;
        let created_at = self
            .runtime_created_at
            .as_deref()
            .map(|raw| parse_instant("runtime_created_at", raw))
            .transpose()?;
        Ok(SandboxLifecycleV1 {
            schema_version: crate::audit::lifecycle::LIFECYCLE_SCHEMA_VERSION,
            event_id: parse_uuid("event_id", &self.event_id)?,
            occurred_at: parse_instant("occurred_at", &self.occurred_at)?,
            action: parse_enum(
                "lifecycle_action",
                &self.lifecycle_action,
                LifecycleAction::parse,
            )?,
            actor: Actor {
                kind: parse_enum("actor.kind", &self.actor.kind, ActorKind::parse)?,
                id: self.actor.id,
                login: self.actor.login.clone(),
                authentication: parse_enum(
                    "actor.authentication",
                    &self.actor.authentication,
                    AuthenticationMethod::parse,
                )?,
            },
            principal: Principal {
                kind: parse_enum("principal.kind", &self.principal.kind, PrincipalKind::parse)?,
                id: self.principal.id.clone(),
            },
            session_id: self.session_id.clone(),
            backend: parse_enum("backend", &self.backend, RuntimeBackendKind::parse)?,
            runtime: LifecycleRuntime {
                runtime_id: self.runtime_id.clone(),
                created_at,
                incarnation_hint: self.incarnation_hint.clone(),
            },
            attribution: LifecycleAttribution {
                creator_id: self.creator_id,
                creator_login: self.creator_login.clone(),
                trigger_author_id: self.trigger_author_id,
                trigger_author_login: self.trigger_author_login.clone(),
            },
            correlation: LifecycleCorrelation {
                repo_full_name: self.correlation.repo_full_name.clone(),
                installation_id: self.correlation.installation_id,
                trigger_issue: self.correlation.trigger_issue,
                request_id: self.correlation.request_id.clone(),
            },
            reason_code,
            service: ServiceIdentity {
                version: self.service_version.clone(),
                environment: self.deployment_environment.clone(),
            },
        })
    }
}

/// The relay's answer to any of the three write endpoints.
///
/// Receiving one means the SQLite transaction committed under the configured
/// `synchronous` policy — not that the record was queued in relay memory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurableAck {
    pub event_id: String,
    /// When the commit completed, RFC3339 UTC.
    pub durable_at: String,
    /// The record's state after the commit.
    pub state: String,
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
