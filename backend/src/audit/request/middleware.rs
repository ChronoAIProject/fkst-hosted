//! The outer audit middleware: one terminal record per in-scope request.
//!
//! The ordering below is the contract, not an implementation detail. Steps 4 and
//! 10 are what `required` delivery mode adds, and both are only possible because
//! the request's identity and event id are fully resolved *before* the inner
//! service is invoked:
//!
//! 1. validate or generate `X-Request-Id`;
//! 2. stamp the UTC start, the monotonic start, and the deterministic `event_id`;
//! 3. resolve the matched route's operation and audit policy;
//! 4. **register the start with the durable relay** — in `required` mode a
//!    failure here returns `503 audit_ingress_unavailable` and the inner service
//!    is NEVER invoked;
//! 5. install the shared context and invoke the inner service;
//! 6. observe the FINAL response (timeout, leader gate, extractor, handler);
//! 7. freeze the context;
//! 8. derive the terminal status/outcome/error code and the completion instant;
//! 9. enqueue exactly one event;
//! 10. **commit the terminal event** — in `required` mode the response is held
//!     until the relay acknowledges, and an unconfirmed completion becomes
//!     `503 audit_completion_unconfirmed`;
//! 11. return the response unchanged apart from the normalized request-id header.
//!
//! Two timing sources are used on purpose: UTC wall time for the displayed
//! start/completion instants, and [`Instant`] for the duration. A clock step
//! between entry and exit can therefore skew the displayed timestamps but can
//! never produce a negative duration.
//!
//! ## Streaming bodies are never buffered
//!
//! Step 10 commits the status, headers, and handler outcome before the response
//! OBJECT is released. It does not wait for a streamed body to be consumed — log
//! downloads and the chat SSE stream would otherwise have to be materialized in
//! memory to be audited, which the epic explicitly forbids. "Completion" here
//! therefore means the handler's outcome is durable, not that the client finished
//! downloading.
//!
//! ## What each mode promises
//!
//! - `disabled` — no relay call; delivery is the configured [`AuditHandle`] sink,
//!   admission is non-blocking, and a drop is a metric plus one log line. This is
//!   exactly the behaviour that existed before the relay.
//! - `best_effort` — the relay is called and a failure is logged and ignored; the
//!   record ALSO goes to the local sink, so an outage cannot lose it.
//! - `required` — the two `503`s above. The local sink is not used: the relay
//!   owns delivery.
//!
//! A relay `409 event_id_conflict` counts as a failure in `required` mode, in
//! both phases. An exact replay is acknowledged with `200`, so a conflict is
//! never a harmless retry: it is the relay stating that what it holds under this
//! event id is a different fact from the one this process is carrying. Treating
//! it as success would let a handler run with no start describing it, or release
//! a status the durable trail contradicts.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use k8s_openapi::chrono::{Duration as WallDuration, Utc};
use uuid::Uuid;

use super::catalog::{OperationCatalog, RouteDecision};
use super::context::AuditRequestContext;
use super::id::{normalize_request_id, REQUEST_ID_HEADER};
use super::outcome::{derive_outcome, framework_error_code};
use super::policy::default_arguments_status;
use super::required::{completion_unconfirmed, ingress_unavailable};
use super::response::{codes, error_code_of, is_rejected};
use crate::audit::event::{
    derive_event_id, ApiRequestCompletedV1, RequestIdentity, RequestResult, RequestTiming,
    ServiceIdentity,
};
use crate::audit::relay::{AuditDelivery, RelayClientError, RequiredRejection};
use crate::audit::AuditHandle;

/// The cloneable state the middleware needs: the verified operation catalog, the
/// sink handle, the emitting deployment's identity, and the delivery policy.
#[derive(Clone, Debug)]
pub struct AuditMiddleware {
    catalog: Arc<OperationCatalog>,
    audit: AuditHandle,
    service: ServiceIdentity,
    delivery: AuditDelivery,
}

impl AuditMiddleware {
    /// The middleware with delivery DISABLED — no relay, today's behaviour.
    pub fn new(
        catalog: Arc<OperationCatalog>,
        audit: AuditHandle,
        service: ServiceIdentity,
    ) -> Self {
        Self {
            catalog,
            audit,
            service,
            delivery: AuditDelivery::disabled(),
        }
    }

    /// Attach a delivery policy (the router does this from configuration).
    pub fn with_delivery(mut self, delivery: AuditDelivery) -> Self {
        self.delivery = delivery;
        self
    }
}

/// What step 2/3 resolved, when the request is in scope.
struct AuditedRequest {
    identity: RequestIdentity,
    event_id: Uuid,
}

/// The audit middleware. Mount it as the OUTERMOST layer of the assembled
/// router (see [`crate::router::build_router`]).
pub async fn audit_requests(
    State(middleware): State<AuditMiddleware>,
    mut request: Request,
    next: Next,
) -> Response {
    // --- 1. request id ------------------------------------------------------
    let inbound = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let request_id = normalize_request_id(inbound.as_deref());
    let header_name = HeaderName::from_static(REQUEST_ID_HEADER);
    // The accepted character set is a strict subset of a valid header value, so
    // this cannot fail; a `None` would still be handled rather than unwrapped.
    let header_value = HeaderValue::from_str(&request_id.value).ok();
    if let Some(value) = header_value.clone() {
        request.headers_mut().insert(header_name.clone(), value);
    }

    // --- 2/3. timing, event id, and the route's audit policy -----------------
    let started_at = Utc::now();
    let monotonic_start = Instant::now();
    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string());
    let decision = middleware
        .catalog
        .resolve(request.method(), matched.as_deref());
    let audited = match &decision {
        RouteDecision::Record {
            operation_id,
            route_template,
        } => {
            let identity = RequestIdentity {
                request_id: request_id.value.clone(),
                method: request.method().as_str().to_string(),
                route_template: route_template.to_string(),
                operation_id: operation_id.to_string(),
            };
            let event_id = derive_event_id(&identity, started_at);
            Some(AuditedRequest { identity, event_id })
        }
        RouteDecision::Skip(_) => None,
    };

    // --- 4. durable pre-handler acknowledgement ------------------------------
    // The ONLY place a product handler can be prevented from running for audit
    // reasons. It happens before the context is installed and before dispatch, so
    // a refused request has run no extractor and performed no side effect.
    if let Some(audited) = &audited {
        let registered = middleware
            .delivery
            .register_start(
                &audited.identity,
                audited.event_id,
                started_at,
                &middleware.service.version,
                &middleware.service.environment,
            )
            .await;
        if let Err(error) = registered {
            middleware.delivery.metrics().record_rejection(
                if error == RelayClientError::Conflict {
                    RequiredRejection::IngressConflict
                } else {
                    RequiredRejection::IngressUnavailable
                },
            );
            // Emergency telemetry: this rejected request cannot itself be
            // durably recorded, so the metric and this line are the only trace
            // it will ever leave.
            tracing::error!(
                request_id = %audited.identity.request_id,
                operation_id = %audited.identity.operation_id,
                reason = error.kind(),
                "audit ingress unavailable; refusing the request without invoking its handler"
            );
            let mut response = ingress_unavailable();
            if let Some(value) = header_value.clone() {
                response.headers_mut().insert(header_name.clone(), value);
            }
            return response;
        }
    }

    // --- 5. install the shared context and dispatch --------------------------
    // Installed for excluded traffic too: uniform behaviour is cheaper to reason
    // about than a conditional that must be re-checked at every write site.
    let context = AuditRequestContext::new();
    context.install(request.extensions_mut());
    let mut response = next.run(request).await;

    // --- 11 (header half). Propagate the normalized id ----------------------
    if let Some(value) = header_value.clone() {
        response.headers_mut().insert(header_name.clone(), value);
    }

    // --- 6. observe the final response --------------------------------------
    let Some(audited) = audited else {
        if let RouteDecision::Skip(reason) = decision {
            tracing::trace!(reason = reason.as_str(), "request excluded from audit");
        }
        return response;
    };
    let status = response.status();
    let rejected = is_rejected(&response);
    let explicit_code = error_code_of(&response);

    // --- 7/8. freeze, then derive the terminal classification ---------------
    // The default classifies a request that recorded no arguments: `unavailable`
    // when the operation HAS a safe-argument contract that never got to run (an
    // authentication, leader-gate, or timeout rejection), `not_applicable` when
    // it genuinely takes none. Deriving it from the operation's own declaration
    // keeps every rejection site free of the question.
    let frozen =
        context.freeze_with_default(default_arguments_status(&audited.identity.operation_id));
    let outcome = derive_outcome(
        status,
        rejected,
        explicit_code == Some(codes::REQUEST_TIMEOUT),
    );
    let error_code = explicit_code
        .map(str::to_string)
        .or_else(|| frozen.error_code.clone())
        .or_else(|| framework_error_code(status, matched.is_some()).map(str::to_string));
    // Monotonic: a wall-clock adjustment during the request cannot make this
    // negative, and `completed_at` is derived FROM it so the recorded duration
    // and the recorded timestamps can never disagree.
    let elapsed = monotonic_start.elapsed();
    let elapsed_ms = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
    let completed_at =
        started_at + WallDuration::try_milliseconds(elapsed_ms).unwrap_or_else(WallDuration::zero);

    if frozen.conflicts > 0 {
        middleware
            .audit
            .record_context_conflicts(u64::from(frozen.conflicts));
    }
    tracing::debug!(
        request_id = %audited.identity.request_id,
        method = %audited.identity.method,
        route_template = %audited.identity.route_template,
        operation_id = %audited.identity.operation_id,
        status = status.as_u16(),
        outcome = outcome.as_str(),
        error_code = error_code.as_deref().unwrap_or(""),
        duration_ms = elapsed_ms,
        "request completed"
    );

    // --- 9. exactly one event ------------------------------------------------
    let mut event = ApiRequestCompletedV1::new(
        audited.identity,
        RequestTiming {
            started_at,
            completed_at,
        },
        frozen.identity.actor,
        frozen.identity.principal,
        RequestResult {
            status_code: Some(status.as_u16()),
            outcome,
            error_code,
        },
        middleware.service.clone(),
    )
    .with_arguments(frozen.arguments, frozen.arguments_parse_status)
    .with_correlation(frozen.correlation);
    // The constructor re-derives the same id from the same inputs; assigning the
    // ENTRY-time value keeps that id authoritative, which is what the durable
    // relay's pre-handler acknowledgement will key on.
    event.event_id = audited.event_id;

    // --- 10. durable terminal commit -----------------------------------------
    // Awaited BEFORE the response object is released. The response's streamed
    // body, if any, is untouched: what must be durable is the outcome, not the
    // bytes a client has yet to read.
    let committed = middleware.delivery.complete(&event).await;

    // Admission is non-blocking and self-reporting: `AuditHandle::submit` counts
    // and logs every drop, so ignoring the result here cannot hide one — and a
    // sink failure must never alter a business response that already completed.
    // In `required` mode the relay owns delivery, so the local sink is skipped
    // rather than given a second copy of the same event id.
    if middleware.delivery.use_local_sink() {
        let _ = middleware.audit.submit(event);
    }

    if let Err(error) = committed {
        middleware
            .delivery
            .metrics()
            .record_rejection(if error == RelayClientError::Conflict {
                RequiredRejection::CompletionConflict
            } else {
                RequiredRejection::CompletionUnconfirmed
            });
        // Emergency telemetry: the handler ran, so something may have happened,
        // and this process must not claim the returned status was recorded. The
        // durable START remains — either still open, so the relay will close it
        // as `incomplete`, or already closed as `incomplete`, which is exactly
        // what a conflict here proves.
        tracing::error!(
            request_id = %request_id.value,
            status = status.as_u16(),
            reason = error.kind(),
            "audit completion unconfirmed; refusing to report a status that was not durably \
             recorded"
        );
        let mut refusal = completion_unconfirmed();
        if let Some(value) = header_value {
            refusal.headers_mut().insert(header_name, value);
        }
        return refusal;
    }

    // --- 11. the response, unchanged ----------------------------------------
    response
}

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
