//! `GET /api/v1/operations/activity` — the scoped historical activity query.
//!
//! The handler is deliberately a sequence of gates, in this exact order:
//!
//! ```text
//! 1. AuthenticatedViewer        401 missing/invalid identity, 403 not admitted
//! 2. normalize the parameters   400  (pure; no authorization, no source call)
//! 3. resolve the scope          403 operations_scope_forbidden
//! 4. record the safe arguments  (once, on BOTH the allowed and refused paths)
//! 5. authorize the session      404 activity_session_not_found / 503
//! 6. decode the cursor          400 invalid_activity_cursor
//! 7. query the sources          200 / 429 / 502 / 503
//! ```
//!
//! Steps 2–6 all complete before a single upstream call, so a refused request
//! costs the deployment nothing and a caller learns nothing from timing.
//!
//! Step 4 sits where it does because a DENIED cross-user probe is exactly the
//! thing an audit trail must record. It records the caller's own effective scope
//! plus the scope they ASKED for and a boolean `actor_filter_present` — never the
//! login or id they guessed at, and never the cursor text or the generated query.
//!
//! ## What this handler never does
//!
//! It never fetches rows and filters them, never hands a browser a PostHog
//! credential or host URL, never returns a total count, and never turns a source
//! outage into a confident empty page.

use axum::extract::State;
use axum::http::Extensions;
use axum::Json;
use k8s_openapi::chrono::{SecondsFormat, Utc};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::arguments::operations::{
    ActivityRecordKind, ActivityScope, SafeOperationsListActivity,
};
use crate::audit::arguments::{record_safe, AuditedQuery};
use crate::error::{AppError, ErrorEnvelope};
use crate::operations::cursor::{self, CursorBinding, CursorKey};
use crate::operations::filters::{RecordKind, TimeRange};
use crate::operations::metrics::{QueryResult, RejectionReason};
use crate::operations::{self, ActivityQueryRequest};
use crate::session_access::{
    authorize_lifecycle_session, authorize_session_visibility, resolve_operations_scope,
    ActivityVisibilityConstraint, AuthenticatedViewer, AuthorizedSessionId, ScopeRequest,
    ViewerScope,
};
use crate::state::AppState;

use super::dto::{page_from_merged, ActivityPage, EffectiveScope};
use super::query::{normalize, ActivityQueryParams, NormalizedActivityRequest};

/// Fixed client text for an unresolvable lifecycle session. Identical whether the
/// id is unknown, unauthorized, or absent — see [`lifecycle_session`].
const SESSION_NOT_FOUND: &str = "no such session";

/// Serve one page of scoped historical activity.
#[utoipa::path(
    get,
    path = "/operations/activity",
    tag = "operations",
    operation_id = "operations_list_activity",
    params(ActivityQueryParams),
    responses(
        (status = 200, description = "One keyset page of activity the caller is authorized to see", body = ActivityPage),
        (status = 400, description = "A malformed parameter, an invalid time range, or a cursor that is not valid for this query", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub identity", body = ErrorEnvelope),
        (status = 403, description = "A regular caller selected the global scope or a cross-user actor filter (`operations_scope_forbidden`), or is not admitted by the deployment access policy", body = ErrorEnvelope),
        (status = 404, description = "The exact lifecycle `session_id` is unknown or not visible to this caller (`activity_session_not_found`); the two are indistinguishable by design", body = ErrorEnvelope),
        (status = 429, description = "Local activity-query capacity is exhausted; a bounded `Retry-After` is returned", body = ErrorEnvelope),
        (status = 502, description = "The activity source refused the query (authentication or schema); retrying will not help", body = ErrorEnvelope),
        (status = 503, description = "No query credentials are configured (`audit_query_not_configured`), the session-visibility projection is cold (`session_visibility_unavailable`), or every source is temporarily unavailable", body = ErrorEnvelope),
    )
)]
async fn operations_list_activity(
    State(state): State<AppState>,
    extensions: Extensions,
    AuditedQuery(params): AuditedQuery<ActivityQueryParams>,
    viewer: AuthenticatedViewer,
) -> Result<Json<ActivityPage>, AppError> {
    let config = &state.config.activity_query;
    let now = Utc::now();
    // Pure validation first: it needs no identity beyond the one already proven,
    // touches no source, and cannot reveal anything about other callers.
    let request = normalize(
        &params,
        now,
        config.activity_default_limit,
        config.activity_max_limit,
        config.activity_max_range_days,
    )
    .inspect_err(|_| {
        state.operations.record_rejected(
            natural_scope(&viewer).as_str(),
            RecordKind::default(),
            QueryResult::InvalidRequest,
            None,
        );
    })?;

    let scope_request = scope_request(&request);
    let resolved =
        resolve_operations_scope(&viewer, scope_request, &state.session_access.scope_metrics);
    // Recorded on BOTH paths, exactly once: a refused probe is as much a fact
    // worth auditing as an allowed query.
    record_safe(&extensions, &safe_arguments(&request, &resolved, &viewer));
    let scope = resolved.inspect_err(|_| {
        state.operations.record_rejected(
            natural_scope(&viewer).as_str(),
            request.record_kind,
            QueryResult::Forbidden,
            Some(if request.cross_actor_filter {
                RejectionReason::CrossActorFilter
            } else {
                RejectionReason::GlobalScope
            }),
        );
    })?;

    let session = lifecycle_session(&state, &viewer, &scope, &request)?;
    let constraint = ActivityVisibilityConstraint::for_scope(&scope, session.clone());
    // A resumed page keeps the window its cursor was issued for, so consecutive
    // pages tile over ONE window instead of a `now` that moves between requests.
    let range = resumed_range(&state, &scope, &request)?;
    let binding = CursorBinding {
        scope: scope.as_str(),
        viewer_id: (!scope.is_global()).then(|| viewer.id()),
        session_id: session.as_ref().map(|id| id.as_str().to_string()),
        record_kind: request.record_kind,
        range,
        filters: request.filters.clone(),
    };
    let cursor = decode_cursor(&state, &scope, &request, &binding)?;

    let merged = operations::run(
        &state.operations,
        viewer.id(),
        ActivityQueryRequest {
            constraint,
            record_kind: request.record_kind,
            range,
            filters: request.filters.clone(),
            cursor,
            limit: request.limit,
        },
    )
    .await?;

    let next_cursor = merged
        .next_key
        .as_ref()
        .map(|key| cursor::encode(key, &binding))
        .transpose()?;
    Ok(Json(page_from_merged(
        &merged,
        now.to_rfc3339_opts(SecondsFormat::Millis, true),
        range.from_rfc3339(),
        range.to_rfc3339(),
        effective_scope(&scope),
        viewer.is_global_admin(),
        next_cursor,
    )))
}

/// Resolve the ONE exact lifecycle session a regular caller may add to their
/// timeline.
///
/// A global administrator needs none: they already see every session's lifecycle
/// rows, and requiring one would only invite the belief that it narrowed
/// authorization rather than results.
///
/// An absent, unknown, and unauthorized session id all answer the SAME
/// `404 activity_session_not_found`. Distinguishing them would turn the endpoint
/// into a session-existence oracle, and "you forgot a parameter" is not worth
/// that (epic `AUTH-06`).
fn lifecycle_session(
    state: &AppState,
    viewer: &AuthenticatedViewer,
    scope: &ViewerScope,
    request: &NormalizedActivityRequest,
) -> Result<Option<AuthorizedSessionId>, AppError> {
    if scope.is_global() || !request.record_kind.includes_lifecycle() {
        return Ok(None);
    }
    let not_found = || {
        state.operations.record_rejected(
            scope.as_str(),
            request.record_kind,
            QueryResult::NotFound,
            Some(RejectionReason::LifecycleSession),
        );
        AppError::ActivitySessionNotFound(SESSION_NOT_FOUND.to_string())
    };
    let Some(session_id) = request.session_id() else {
        return Err(not_found());
    };
    let decision = authorize_session_visibility(
        &state.session_access.registry,
        viewer,
        scope,
        &state.config.access,
        &state.config.log.admins,
        session_id,
    )
    .map_err(|error| match error {
        // The gate's own anti-enumeration `404` is re-coded to this endpoint's
        // stable one; a cold projection keeps its distinct `503`, because "retry
        // shortly" and "no such session" need different remedies.
        AppError::NotFound(_) => not_found(),
        other => other,
    })?;
    authorize_lifecycle_session(session_id, &decision)
        .map(Some)
        .ok_or_else(not_found)
}

/// The window this request runs over.
///
/// Without a cursor it is the normalized range. WITH one it is the window the
/// cursor was issued for — and if the caller ALSO stated an explicit `from`/`to`
/// that disagrees, that is a different query and the cursor is refused rather
/// than quietly re-windowed.
fn resumed_range(
    state: &AppState,
    scope: &ViewerScope,
    request: &NormalizedActivityRequest,
) -> Result<TimeRange, AppError> {
    let Some(raw) = request.cursor.as_deref() else {
        return Ok(request.range);
    };
    let refuse = |error: AppError| {
        state.operations.record_rejected(
            scope.as_str(),
            request.record_kind,
            QueryResult::InvalidRequest,
            Some(RejectionReason::Cursor),
        );
        error
    };
    let range = cursor::peek_range(raw).map_err(refuse)?;
    if request.range_explicit && range != request.range {
        return Err(refuse(AppError::InvalidActivityCursor(
            "cursor is not valid for this query; start a new page".to_string(),
        )));
    }
    Ok(range)
}

/// Decode the caller's cursor against this exact query's binding.
fn decode_cursor(
    state: &AppState,
    scope: &ViewerScope,
    request: &NormalizedActivityRequest,
    binding: &CursorBinding,
) -> Result<Option<CursorKey>, AppError> {
    let Some(raw) = request.cursor.as_deref() else {
        return Ok(None);
    };
    cursor::decode(raw, binding).map(Some).inspect_err(|_| {
        state.operations.record_rejected(
            scope.as_str(),
            request.record_kind,
            QueryResult::InvalidRequest,
            Some(RejectionReason::Cursor),
        );
    })
}

/// The scope inputs, normalized for the shared resolver.
fn scope_request(request: &NormalizedActivityRequest) -> ScopeRequest {
    let base = ScopeRequest::new(request.requested_scope);
    if request.cross_actor_filter {
        base.with_cross_actor_filter()
    } else {
        base
    }
}

/// The scope a caller has WITHOUT asking for anything — what a refused request
/// is recorded and counted under.
fn natural_scope(viewer: &AuthenticatedViewer) -> ActivityScope {
    if viewer.is_global_admin() {
        ActivityScope::All
    } else {
        ActivityScope::Mine
    }
}

fn effective_scope(scope: &ViewerScope) -> EffectiveScope {
    if scope.is_global() {
        EffectiveScope::All
    } else {
        EffectiveScope::Mine
    }
}

/// Project the normalized request onto its reviewed safe-argument boundary.
///
/// The EFFECTIVE scope is the resolved one when the request was allowed, and the
/// caller's natural scope when it was refused — so a denial reads as "this
/// caller, whose scope is `mine`, asked for `all`" without ever recording who
/// they tried to impersonate.
fn safe_arguments(
    request: &NormalizedActivityRequest,
    resolved: &Result<ViewerScope, AppError>,
    viewer: &AuthenticatedViewer,
) -> SafeOperationsListActivity {
    let effective = match resolved {
        Ok(scope) if scope.is_global() => ActivityScope::All,
        Ok(_) => ActivityScope::Mine,
        Err(_) => natural_scope(viewer),
    };
    let requested = request.requested_scope.map(|requested| match requested {
        crate::session_access::RequestedScope::Global => ActivityScope::All,
        crate::session_access::RequestedScope::Personal => ActivityScope::Mine,
    });
    SafeOperationsListActivity {
        scope: effective,
        // Recorded only when it DIFFERS from the effective scope: an identical
        // pair carries no information, and a refused probe is exactly the case
        // where the two diverge.
        requested_scope: requested.filter(|requested| *requested != effective),
        record_kind: match request.record_kind {
            RecordKind::ApiRequest => ActivityRecordKind::ApiRequest,
            RecordKind::SandboxLifecycle => ActivityRecordKind::SandboxLifecycle,
            RecordKind::All => ActivityRecordKind::All,
        },
        from: Some(request.range.from_rfc3339()),
        to: Some(request.range.to_rfc3339()),
        limit: request.limit,
        cursor_present: request.cursor_present(),
        actor_filter_present: request.cross_actor_filter,
        session_id: request.filters.session_id.clone(),
        repo_full_name: request.filters.repo_full_name.clone(),
        trigger_issue: request.filters.trigger_issue,
        request_id: request.filters.request_id.clone(),
        method: request.filters.method.clone(),
        operation_id: request.filters.operation_id.clone(),
        status: request.filters.status_code,
    }
}

/// The operations router, merged into the `/api/v1` subtree.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(operations_list_activity))
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
