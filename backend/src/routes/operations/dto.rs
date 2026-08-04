//! The response contract of `GET /api/v1/operations/activity`.
//!
//! Two things about the shape are deliberate.
//!
//! **A tagged union, not one flat row.** `record_kind` discriminates, and each
//! arm carries only the fields its contract actually has. A lifecycle transition
//! has no HTTP method, status, or duration; giving it null ones would invite
//! every reader — a dashboard, a support engineer, a future filter — to treat "no
//! status" and "status unknown" as the same thing.
//!
//! **No total count.** A count is a number derived from rows, and the moment it
//! exists somebody has to prove it was derived only from AUTHORIZED rows. There
//! is no total, no "hidden" count, and no row-error figure that describes
//! anything but already-authorized candidates: `source_status` reports bounded
//! DEPLOYMENT health, never statistics about records the caller may not see
//! (epic `AUTH-06`).
//!
//! Every value here comes from the audit contract's own allowlisted properties.
//! `arguments` is verbatim the operation's safe DTO — the write side already
//! filtered it to that operation's documented field list — so there is no path
//! by which a raw body, URL, header, or credential reaches this response.

use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::operations::merge::{MergedPage, SourceHealth};
use crate::operations::record::ActivityRecord;

/// The effective scope a page was produced under.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveScope {
    /// Only rows whose verified actor id equals the caller's, plus system
    /// lifecycle rows for one authorized session.
    Mine,
    /// Every actor and record kind. Global administrators only.
    All,
}

/// The initiating identity, as recorded.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct ActorView {
    /// `github_user`, `github_webhook_sender`, `anonymous`, `service`, `system`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The immutable GitHub numeric id. The only ownership proof.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// A historical login snapshot. Display only — never an authorization input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// The executing identity, as recorded. Never a credential or a token
/// fingerprint.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct PrincipalView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// The correlation keys a record carries (epic `AUD-05`).
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct CorrelationView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `owner/name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_issue: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The GitHub delivery id, when the record was driven by a webhook. Without
    /// it a webhook-triggered request cannot be traced back to the delivery that
    /// caused it (epic `AUD-05`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_delivery_id: Option<String>,
}

/// One recorded API request.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ApiRequestActivityItem {
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// RFC3339 UTC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 UTC. The row's sort key.
    pub completed_at: String,
    pub method: String,
    /// The normalized route template, never a raw URI.
    pub route_template: String,
    pub operation_id: String,
    pub actor: ActorView,
    pub principal: PrincipalView,
    /// The operation's own allowlisted safe arguments, verbatim.
    #[schema(value_type = Object)]
    pub arguments: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments_parse_status: Option<String>,
    /// `null` for an incomplete record: no system fabricates a status it never
    /// returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    pub outcome: String,
    /// A stable application error code, never error text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub correlation: CorrelationView,
    /// How far through delivery the record is.
    pub delivery_state: String,
    /// Which source produced it: `posthog` or `relay`.
    pub source: String,
}

/// One recorded sandbox lifecycle transition.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SandboxLifecycleActivityItem {
    pub event_id: String,
    /// RFC3339 UTC. The row's sort key.
    pub occurred_at: String,
    pub lifecycle_action: String,
    pub actor: ActorView,
    pub principal: PrincipalView,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// The session's effective creator. Display and correlation only — session
    /// authorization is decided by the access registry, never by this field.
    pub creator: ActorView,
    pub trigger_author: ActorView,
    pub correlation: CorrelationView,
    /// When the runtime was created, RFC3339 UTC.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// A closed reason code, never an upstream error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub delivery_state: String,
    pub source: String,
}

/// One row of a merged timeline, discriminated by `record_kind`.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(tag = "record_kind", rename_all = "snake_case")]
pub enum ActivityItem {
    ApiRequest(ApiRequestActivityItem),
    SandboxLifecycle(SandboxLifecycleActivityItem),
}

/// Per-source health for one page. Bounded deployment health only.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SourceStatusView {
    /// `healthy` | `degraded` | `unavailable` | `not_configured`.
    pub posthog: String,
    /// As above; `not_configured` until the durable relay lands.
    pub relay: String,
    /// True when at least one source could not fully answer. A partial page is
    /// never presented as a complete empty one.
    pub partial: bool,
    /// A bounded, stable code explaining `partial`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_code: Option<String>,
}

/// One page of activity.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ActivityPage {
    /// When the server assembled this page, RFC3339 UTC.
    pub queried_at: String,
    /// The normalized inclusive lower bound, RFC3339 UTC.
    pub from: String,
    /// The normalized exclusive upper bound, RFC3339 UTC.
    pub to: String,
    pub effective_scope: EffectiveScope,
    /// Whether this caller may select the global scope. A server fact the UI uses
    /// to LABEL controls; it is never an authorization input, and changing it
    /// client-side widens nothing.
    pub can_view_all: bool,
    pub items: Vec<ActivityItem>,
    /// The opaque keyset cursor for the next page, when one may exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub source_status: SourceStatusView,
    /// The deployment's own ceiling on `to - from`, in days
    /// (`FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS`).
    ///
    /// It is stated on every page because a client that guesses it either
    /// refuses windows this deployment would have answered, or issues windows it
    /// is guaranteed to refuse — and a `400` at that boundary is a worse user
    /// experience than a control that never offers the impossible range. It is
    /// deployment policy, not a secret: the same bound is already observable by
    /// sending one over-wide request.
    pub max_range_days: u64,
}

impl SourceStatusView {
    /// Project the merge layer's health block.
    pub fn from_merged(status: &crate::operations::merge::SourceStatus) -> Self {
        Self {
            posthog: health(status.posthog),
            relay: health(status.relay),
            partial: status.partial,
            message_code: status.message_code.map(str::to_string),
        }
    }
}

fn health(value: SourceHealth) -> String {
    value.as_str().to_string()
}

/// Project one already-authorized record onto its wire item.
pub fn item_from_record(record: &ActivityRecord) -> ActivityItem {
    match record {
        ActivityRecord::ApiRequest {
            record,
            delivery_state,
            source,
        } => ActivityItem::ApiRequest(ApiRequestActivityItem {
            event_id: record.event_id.clone(),
            request_id: record.request_id.clone(),
            started_at: record.started_at.map(rfc3339),
            completed_at: rfc3339(record.completed_at),
            method: record.method.clone(),
            route_template: record.route_template.clone(),
            operation_id: record.operation_id.clone(),
            actor: ActorView {
                kind: record.actor.kind.clone(),
                id: record.actor.id,
                login: record.actor.login.clone(),
            },
            principal: PrincipalView {
                kind: record.principal.kind.clone(),
                id: record.principal.id.clone(),
            },
            arguments: record.arguments.clone(),
            arguments_parse_status: record.arguments_parse_status.clone(),
            status_code: record.status_code,
            outcome: record.outcome.clone(),
            error_code: record.error_code.clone(),
            duration_ms: record.duration_ms,
            correlation: correlation(&record.correlation),
            delivery_state: delivery_state.as_str().to_string(),
            source: source.as_str().to_string(),
        }),
        ActivityRecord::SandboxLifecycle {
            record,
            delivery_state,
            source,
        } => ActivityItem::SandboxLifecycle(SandboxLifecycleActivityItem {
            event_id: record.event_id.clone(),
            occurred_at: rfc3339(record.occurred_at),
            lifecycle_action: record.lifecycle_action.clone(),
            actor: ActorView {
                kind: record.actor.kind.clone(),
                id: record.actor.id,
                login: record.actor.login.clone(),
            },
            principal: PrincipalView {
                kind: record.principal.kind.clone(),
                id: record.principal.id.clone(),
            },
            session_id: record.session_id.clone(),
            backend: record.backend.clone(),
            runtime_id: record.runtime_id.clone(),
            creator: ActorView {
                kind: None,
                id: record.creator_id,
                login: record.creator_login.clone(),
            },
            trigger_author: ActorView {
                kind: None,
                id: record.trigger_author_id,
                login: record.trigger_author_login.clone(),
            },
            correlation: correlation(&record.correlation),
            created_at: record.created_at.map(rfc3339),
            reason_code: record.reason_code.clone(),
            delivery_state: delivery_state.as_str().to_string(),
            source: source.as_str().to_string(),
        }),
    }
}

/// Everything about a page that is not derived from the merged rows.
pub struct PageEnvelope {
    /// When the server assembled this page, RFC3339 UTC.
    pub queried_at: String,
    /// The normalized window, RFC3339 UTC.
    pub from: String,
    pub to: String,
    pub effective_scope: EffectiveScope,
    pub can_view_all: bool,
    pub next_cursor: Option<String>,
    /// The deployment's configured `to - from` ceiling, in days.
    pub max_range_days: u64,
}

/// Assemble the page body from a merged result.
pub fn page_from_merged(merged: &MergedPage, envelope: PageEnvelope) -> ActivityPage {
    ActivityPage {
        queried_at: envelope.queried_at,
        from: envelope.from,
        to: envelope.to,
        effective_scope: envelope.effective_scope,
        can_view_all: envelope.can_view_all,
        items: merged.items.iter().map(item_from_record).collect(),
        next_cursor: envelope.next_cursor,
        source_status: SourceStatusView::from_merged(&merged.status),
        max_range_days: envelope.max_range_days,
    }
}

fn correlation(value: &crate::operations::record::RecordCorrelation) -> CorrelationView {
    CorrelationView {
        session_id: value.session_id.clone(),
        repo_full_name: value.repo_full_name.clone(),
        installation_id: value.installation_id,
        trigger_issue: value.trigger_issue,
        request_id: value.request_id.clone(),
        webhook_delivery_id: value.webhook_delivery_id.clone(),
    }
}

/// RFC3339 UTC with millisecond precision — the exact form the audit contract
/// writes, so a client comparing a timestamp to a recorded one gets an equality.
fn rfc3339(value: k8s_openapi::chrono::DateTime<k8s_openapi::chrono::Utc>) -> String {
    value.to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Millis, true)
}
