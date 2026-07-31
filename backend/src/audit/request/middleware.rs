//! The outer audit middleware: one terminal record per in-scope request.
//!
//! The ordering below is the contract, not an implementation detail — the
//! durable relay issue adds a pre-handler acknowledgement between steps 3 and 4,
//! which is only possible because the request's identity and event id are fully
//! resolved *before* the inner service is invoked:
//!
//! 1. validate or generate `X-Request-Id`;
//! 2. stamp the UTC start, the monotonic start, and the deterministic `event_id`;
//! 3. resolve the matched route's operation and audit policy;
//! 4. install the shared context and invoke the inner service;
//! 5. observe the FINAL response (timeout, leader gate, extractor, handler);
//! 6. freeze the context;
//! 7. derive the terminal status/outcome/error code and the completion instant;
//! 8. enqueue exactly one event;
//! 9. return the response unchanged apart from the normalized request-id header.
//!
//! Two timing sources are used on purpose: UTC wall time for the displayed
//! start/completion instants, and [`Instant`] for the duration. A clock step
//! between entry and exit can therefore skew the displayed timestamps but can
//! never produce a negative duration.
//!
//! Delivery is best-effort in this mode: admission is non-blocking, and a full
//! queue, a disabled sink, or a transport failure increments a bounded metric and
//! logs once — it never rewrites a business response that already succeeded, and
//! it never spawns a task per request.

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
use super::response::{codes, error_code_of, is_rejected};
use crate::audit::event::{
    derive_event_id, ApiRequestCompletedV1, RequestIdentity, RequestResult, RequestTiming,
    ServiceIdentity,
};
use crate::audit::AuditHandle;

/// The cloneable state the middleware needs: the verified operation catalog, the
/// sink handle, and the emitting deployment's identity.
#[derive(Clone, Debug)]
pub struct AuditMiddleware {
    catalog: Arc<OperationCatalog>,
    audit: AuditHandle,
    service: ServiceIdentity,
}

impl AuditMiddleware {
    pub fn new(
        catalog: Arc<OperationCatalog>,
        audit: AuditHandle,
        service: ServiceIdentity,
    ) -> Self {
        Self {
            catalog,
            audit,
            service,
        }
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

    // --- 4. install the shared context and dispatch --------------------------
    // Installed for excluded traffic too: uniform behaviour is cheaper to reason
    // about than a conditional that must be re-checked at every write site.
    let context = AuditRequestContext::new();
    context.install(request.extensions_mut());
    let mut response = next.run(request).await;

    // --- 9 (header half). Propagate the normalized id -----------------------
    if let Some(value) = header_value {
        response.headers_mut().insert(header_name, value);
    }

    // --- 5. observe the final response --------------------------------------
    let Some(audited) = audited else {
        if let RouteDecision::Skip(reason) = decision {
            tracing::trace!(reason = reason.as_str(), "request excluded from audit");
        }
        return response;
    };
    let status = response.status();
    let rejected = is_rejected(&response);
    let explicit_code = error_code_of(&response);

    // --- 6/7. freeze, then derive the terminal classification ---------------
    let frozen = context.freeze();
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

    // --- 8. exactly one event ------------------------------------------------
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

    // Admission is non-blocking and self-reporting: `AuditHandle::submit` counts
    // and logs every drop, so ignoring the result here cannot hide one — and a
    // sink failure must never alter a business response that already completed.
    let _ = middleware.audit.submit(event);

    // --- 9. the response, unchanged -----------------------------------------
    response
}

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
