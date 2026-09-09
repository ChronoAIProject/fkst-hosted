//! The three write endpoints. Each answers only after its SQLite transaction is
//! durably committed.
//!
//! The order inside every handler is the same and is the contract:
//!
//! 1. authenticate with the WRITE token (constant-time);
//! 2. parse the wire body into the domain type — an unknown enum spelling or a
//!    malformed instant is a `400`, never a coerced default;
//! 3. **re-run the server-side audit validation**, in EVERY handler, including
//!    the start: a completion and a lifecycle event go through
//!    [`crate::audit::validate`] / [`crate::audit::lifecycle_validate`], and a
//!    start through the field validators those share
//!    ([`crate::audit::validate::validate_request_identity`] and
//!    [`crate::audit::validate::validate_service_identity`]). The control plane
//!    already validated, but the relay is a separate trust boundary and must not
//!    rely on that: this is what keeps a raw URI, a free-text error string, or a
//!    record whose canonical and nested actor ids disagree out of durable
//!    storage. The start is not the weak link people assume it is — it is stored
//!    verbatim AND copied into the synthesized `incomplete` projection, so an
//!    unvalidated start reaches the read API twice over;
//! 4. refuse for capacity BEFORE touching the writer queue;
//! 5. commit inside one transaction and answer with the durable instant.
//!
//! Nothing here logs a body, a field value, an event id, a token, or a caller
//! address. A rejection logs the endpoint and a bounded reason.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use k8s_openapi::chrono::Utc;

use super::super::auth::TokenRole;
use super::super::db::ingest::{self, Ingested};
use super::super::metrics::{IngressKind, IngressResult};
use super::super::protocol::{
    format_instant, DurableAck, LifecycleEventV1, RequestCompletionV1, RequestStartV1,
};
use super::error::{RelayError, RelayResult};
use super::RelayState;

/// What one write handler produced: the answer, plus what actually happened in
/// storage. The two are separate because the HTTP status cannot express the
/// second — see [`IngressResult`].
struct Committed {
    status: StatusCode,
    ack: DurableAck,
    result: IngressResult,
}

/// `POST /internal/v1/audit/request-starts`
pub async fn post_request_start(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Json(body): Json<RequestStartV1>,
) -> RelayResult<(StatusCode, Json<DurableAck>)> {
    let kind = IngressKind::RequestStart;
    let outcome = commit_start(&state, &headers, body).await;
    finish(&state, kind, outcome)
}

async fn commit_start(
    state: &RelayState,
    headers: &HeaderMap,
    body: RequestStartV1,
) -> RelayResult<Committed> {
    authorize(state, headers, TokenRole::Write)?;
    let identity = body.to_identity().map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a request start");
        RelayError::Invalid("the request start does not satisfy the protocol contract")
    })?;
    // Step 3 for the START path. `to_identity` only proves the ids and instants
    // parse; this is what bounds the strings and — the reason it cannot be
    // skipped — refuses a `route_template` that is a raw query-bearing URI. A
    // start is stored verbatim and later COPIED into the synthesized incomplete
    // projection, so anything accepted here reaches the read API unchanged.
    crate::audit::validate::validate_request_identity(
        &body.request_id,
        &body.method,
        &body.route_template,
        &body.operation_id,
    )
    .and_then(|()| {
        crate::audit::validate::validate_service_identity(
            &body.service_version,
            &body.deployment_environment,
        )
    })
    .map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a request start by contract");
        RelayError::Invalid("the request start does not satisfy the audit event contract")
    })?;
    guard_capacity(state)?;

    let now = Utc::now();
    let event_id = identity.event_id.to_string();
    let ingested = state
        .db
        .write(move |transaction| ingest::register_start(transaction, &body, &identity, now))
        .await?;
    Ok(ack(event_id, ingested, now))
}

/// `PUT /internal/v1/audit/requests/{event_id}/completion`
pub async fn put_request_completion(
    State(state): State<RelayState>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RequestCompletionV1>,
) -> RelayResult<(StatusCode, Json<DurableAck>)> {
    let kind = IngressKind::RequestCompletion;
    let outcome = commit_completion(&state, &headers, event_id, body).await;
    finish(&state, kind, outcome)
}

async fn commit_completion(
    state: &RelayState,
    headers: &HeaderMap,
    event_id: String,
    body: RequestCompletionV1,
) -> RelayResult<Committed> {
    authorize(state, headers, TokenRole::Write)?;
    // The path is the idempotency key; a body naming a different id would make
    // the URL a lie and the dedupe key ambiguous.
    if event_id != body.event_id {
        return Err(RelayError::Invalid(
            "the path event id must equal the body event id",
        ));
    }
    let domain = body.to_domain().map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a completion body");
        RelayError::Invalid("the completion does not satisfy the protocol contract")
    })?;
    crate::audit::validate::validate(&domain).map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a completion by contract");
        RelayError::Invalid("the completion does not satisfy the audit event contract")
    })?;

    let now = Utc::now();
    // The PARSED instant, not the caller's rendering of it: `terminal_at` is the
    // sort and range column, and it is compared as TEXT.
    let terminal_at = domain.completed_at;
    let ingested = state
        .db
        .write(move |transaction| ingest::commit_completion(transaction, &body, terminal_at, now))
        .await?;
    // A completion always answers 200: the record it terminates already existed,
    // so `created` would be a lie about the RECORD even when this call is what
    // committed its terminal projection. The metric label keeps that distinction.
    Ok(Committed {
        status: StatusCode::OK,
        ack: DurableAck {
            event_id: domain.event_id.to_string(),
            durable_at: format_instant(now),
            state: ingested.state().as_str().to_string(),
        },
        result: ingress_result(ingested),
    })
}

/// `POST /internal/v1/audit/events`
pub async fn post_event(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Json(body): Json<LifecycleEventV1>,
) -> RelayResult<(StatusCode, Json<DurableAck>)> {
    let kind = IngressKind::LifecycleEvent;
    let outcome = commit_event(&state, &headers, body).await;
    finish(&state, kind, outcome)
}

async fn commit_event(
    state: &RelayState,
    headers: &HeaderMap,
    body: LifecycleEventV1,
) -> RelayResult<Committed> {
    authorize(state, headers, TokenRole::Write)?;
    let domain = body.to_domain().map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a lifecycle body");
        RelayError::Invalid("the lifecycle event does not satisfy the protocol contract")
    })?;
    crate::audit::validate_lifecycle(&domain).map_err(|error| {
        tracing::warn!(reason = %error, "audit relay: rejected a lifecycle event by contract");
        RelayError::Invalid("the lifecycle event does not satisfy the audit event contract")
    })?;
    guard_capacity(state)?;

    let now = Utc::now();
    let event_id = domain.event_id.to_string();
    // A lifecycle event is terminal on arrival, so its occurrence instant IS the
    // sort column — normalized for the same reason a completion's is.
    let occurred_at = domain.occurred_at;
    let ingested = state
        .db
        .write(move |transaction| ingest::commit_lifecycle(transaction, &body, occurred_at, now))
        .await?;
    Ok(ack(event_id, ingested, now))
}

/// Shared: check credentials for `role`.
fn authorize(state: &RelayState, headers: &HeaderMap, role: TokenRole) -> RelayResult<()> {
    state
        .tokens
        .authorize(headers, role)
        .map_err(|_| RelayError::Unauthorized)
}

/// Shared: refuse before queueing when the outbox is full.
///
/// The flag is published by the worker's sweep, so this costs one atomic load
/// rather than a `COUNT(*)` per request — and it fails CLOSED: a relay that kept
/// accepting past its ceiling would fill the volume and lose the records it
/// already holds.
fn guard_capacity(state: &RelayState) -> RelayResult<()> {
    if state.is_at_capacity() {
        tracing::error!(
            max_records = state.db.max_records(),
            "audit relay: refusing ingress at the configured record capacity"
        );
        return Err(RelayError::Capacity);
    }
    Ok(())
}

/// Shared: the created/replayed answer.
fn ack(event_id: String, ingested: Ingested, now: k8s_openapi::chrono::DateTime<Utc>) -> Committed {
    let status = if ingested.created() {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Committed {
        status,
        ack: DurableAck {
            event_id,
            durable_at: format_instant(now),
            state: ingested.state().as_str().to_string(),
        },
        result: ingress_result(ingested),
    }
}

/// Shared: the bounded label for what storage did.
fn ingress_result(ingested: Ingested) -> IngressResult {
    if ingested.created() {
        IngressResult::Created
    } else {
        IngressResult::Replayed
    }
}

/// Shared: count the outcome, then hand back the HTTP answer.
fn finish(
    state: &RelayState,
    kind: IngressKind,
    outcome: RelayResult<Committed>,
) -> RelayResult<(StatusCode, Json<DurableAck>)> {
    let result = match &outcome {
        // Taken from the ingest outcome, NOT from the status: `commit_completion`
        // answers `200` for a first commit and for a retry alike, and an operator
        // watching for retry storms has to be able to tell them apart.
        Ok(committed) => committed.result,
        Err(error) => error.ingress_result(),
    };
    state.metrics.record_ingress(kind, result);
    outcome.map(|committed| (committed.status, Json(committed.ack)))
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
