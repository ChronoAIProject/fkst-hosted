//! Projection of a validated audit record onto the PostHog capture wire format.
//!
//! Two shapes coexist on purpose:
//!
//! - **flattened first-level properties** (`actor_id`, `session_id`, `outcome`,
//!   …) are what fixed HogQL and the relay's SQL filter on. They must be plain
//!   scalars at the top level, because a mandatory viewer predicate that has to
//!   dig through nested JSON is a predicate that will eventually be written
//!   wrong;
//! - **structured objects** (`arguments`, `actor`, `principal`, `correlation`)
//!   are preserved for display and forward compatibility, so a later schema can
//!   add a nested field without breaking the flat filter contract.
//!
//! `distinct_id` is an ANALYTICS grouping key, never an authorization input
//! (epic `AUTH-03`). A known GitHub human gets `github:<numeric-id>` — the login
//! is deliberately never used, because logins can be renamed and reassigned.
//! Anonymous/service/system records get a stable non-human distinct id plus
//! `$process_person_profile=false`, so probes and background loops never create
//! PostHog person profiles. `$set`/`$set_once` are never emitted at all: person
//! properties would be a second, mutable copy of authorization-adjacent state.

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::event::{ActorKind, ApiRequestCompletedV1, EVENT_NAME};
use super::validate::{validate, EventError};

/// Distinct id for an unattributed caller.
pub const ANONYMOUS_DISTINCT_ID: &str = "fkst:anonymous";
/// Distinct id for a machine caller that is not a GitHub person.
pub const SERVICE_DISTINCT_ID: &str = "fkst:service";
/// Distinct id for the control plane acting on its own behalf.
pub const SYSTEM_DISTINCT_ID: &str = "fkst:system";
/// Distinct id for a webhook whose sender could not be resolved to a GitHub id.
pub const WEBHOOK_DISTINCT_ID: &str = "fkst:webhook";
/// Prefix of the only human distinct-id form.
pub const GITHUB_DISTINCT_ID_PREFIX: &str = "github:";

/// PostHog's per-event opt-out for person-profile processing.
const PROCESS_PERSON_PROFILE: &str = "$process_person_profile";

/// Bounds applied at projection time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventLimits {
    /// Maximum serialized size of one projected event, in bytes.
    pub max_event_bytes: usize,
}

impl EventLimits {
    pub fn new(max_event_bytes: usize) -> Self {
        Self { max_event_bytes }
    }
}

/// One event in PostHog's capture/batch payload. The project token is NOT part
/// of this struct: it is added by the transport, so an event can be logged,
/// snapshotted, or handed to a relay without ever carrying a credential.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CaptureEvent {
    pub event: &'static str,
    pub distinct_id: String,
    /// The record's deterministic event id — PostHog deduplicates on it, which
    /// is what makes at-least-once retries safe.
    pub uuid: String,
    /// Completion instant, RFC3339 UTC with millisecond precision.
    pub timestamp: String,
    pub properties: Map<String, Value>,
}

impl ApiRequestCompletedV1 {
    /// Validate this record and project it onto the PostHog wire format.
    ///
    /// Fails (never truncates) when the record violates the contract or the
    /// serialized event exceeds `limits.max_event_bytes`; the caller turns that
    /// into a drop metric plus a structured log.
    pub fn to_capture_event(&self, limits: EventLimits) -> Result<CaptureEvent, EventError> {
        validate(self)?;
        let projected = CaptureEvent {
            event: EVENT_NAME,
            distinct_id: self.distinct_id(),
            uuid: self.event_id.to_string(),
            timestamp: self.completed_at_rfc3339(),
            properties: self.properties(),
        };
        let encoded = serde_json::to_vec(&projected)
            .map_err(|e| EventError::Unserializable(e.to_string()))?;
        if encoded.len() > limits.max_event_bytes {
            return Err(EventError::TooLarge {
                actual: encoded.len(),
                limit: limits.max_event_bytes,
            });
        }
        Ok(projected)
    }

    /// The PostHog distinct id. Human ids are `github:<numeric-id>`; every other
    /// actor kind maps to one stable non-human constant.
    pub fn distinct_id(&self) -> String {
        match (self.actor.kind, self.actor_id) {
            (kind, Some(id)) if kind.is_human() => format!("{GITHUB_DISTINCT_ID_PREFIX}{id}"),
            (ActorKind::GithubWebhookSender, None) => WEBHOOK_DISTINCT_ID.to_string(),
            (ActorKind::Service, _) => SERVICE_DISTINCT_ID.to_string(),
            (ActorKind::System, _) => SYSTEM_DISTINCT_ID.to_string(),
            _ => ANONYMOUS_DISTINCT_ID.to_string(),
        }
    }

    /// True when this record maps to a real GitHub person and may therefore
    /// carry a PostHog person profile.
    fn has_person_profile(&self) -> bool {
        self.actor.kind.is_human() && self.actor_id.is_some()
    }

    /// The flattened + structured property bag.
    fn properties(&self) -> Map<String, Value> {
        let mut properties = Map::new();
        let mut put = |key: &str, value: Value| {
            properties.insert(key.to_string(), value);
        };

        // --- identity of the record itself -------------------------------
        put("schema_version", json!(self.schema_version));
        put("event_id", json!(self.event_id.to_string()));
        put("request_id", json!(self.request_id));

        // --- what was called ---------------------------------------------
        put("method", json!(self.method));
        put("route_template", json!(self.route_template));
        put("operation_id", json!(self.operation_id));

        // --- when and for how long ---------------------------------------
        put("started_at", json!(self.started_at_rfc3339()));
        put("completed_at", json!(self.completed_at_rfc3339()));
        put("duration_ms", json!(self.duration_ms));

        // --- how it ended --------------------------------------------------
        put("status_code", json!(self.status_code));
        put("outcome", json!(self.outcome.as_str()));
        put("error_code", json!(self.error_code));

        // --- who (canonical, authorization-supporting) ---------------------
        put("actor_kind", json!(self.actor.kind.as_str()));
        put("actor_id", json!(self.actor_id));
        put("actor_login", json!(self.actor.login));
        put("principal_kind", json!(self.principal.kind.as_str()));
        // Flat as well as nested: the read surface projects `principal.id` from
        // this column (`operations/hogql.rs`), and a value that exists only
        // inside the structured `principal` object below would be silently
        // absent from every API-request row a user ever sees. The lifecycle
        // contract already writes it flat; these two must not disagree.
        put("principal_id", json!(self.principal.id));

        // --- correlation (canonical) ---------------------------------------
        put("session_id", json!(self.session_id));
        put("repo_full_name", json!(self.correlation.repo_full_name));
        put("trigger_issue", json!(self.correlation.trigger_issue));
        put("installation_id", json!(self.correlation.installation_id));
        put(
            "webhook_delivery_id",
            json!(self.correlation.webhook_delivery_id),
        );

        // --- emitting deployment -------------------------------------------
        put("service_version", json!(self.service.version));
        put("service_environment", json!(self.service.environment));

        // --- structured objects (display / forward compatibility) ----------
        put(
            "arguments_parse_status",
            json!(self.arguments_parse_status.as_str()),
        );
        put("arguments", Value::Object(self.arguments.clone()));
        put(
            "actor",
            json!({
                "kind": self.actor.kind.as_str(),
                "id": self.actor.id,
                "login": self.actor.login,
                "authentication": self.actor.authentication.as_str(),
            }),
        );
        put(
            "principal",
            json!({
                "kind": self.principal.kind.as_str(),
                "id": self.principal.id,
            }),
        );
        put(
            "correlation",
            json!({
                "session_id": self.correlation.session_id,
                "repo_full_name": self.correlation.repo_full_name,
                "installation_id": self.correlation.installation_id,
                "trigger_issue": self.correlation.trigger_issue,
                "webhook_delivery_id": self.correlation.webhook_delivery_id,
            }),
        );

        // Non-human traffic must not create person profiles.
        if !self.has_person_profile() {
            put(PROCESS_PERSON_PROFILE, json!(false));
        }
        properties
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
