//! The fixed, server-owned HogQL for the activity query.
//!
//! **The whole security argument of this module is that a request cannot
//! contribute one character to the query TEXT.** Everything a caller supplies
//! travels in `values`, PostHog's HogQL placeholder map; the query source is
//! assembled exclusively from `&'static str` snippets and compile-time
//! placeholder NAMES. There is no field name, operator, event name, sort
//! expression, or fragment a caller can choose — only which of the fixed
//! predicates are switched on.
//!
//! ## The mandatory predicate goes in before `LIMIT`
//!
//! The viewer predicate is part of the `WHERE` clause, so the source decides the
//! page from rows the caller may already see. Fetching a global page and
//! filtering it in Rust would be wrong twice over: the page boundary — and
//! therefore `next_cursor` and page fullness — would be decided by rows the
//! caller may not see, which leaks their existence without ever showing them
//! (epic `AUTH-06`).
//!
//! ## Why `record_kind=all` is a parenthesized union
//!
//! ```text
//! (   ( event IN (…request events…) AND actor_id = {viewer} AND session_id = {session} )
//!  OR ( event = 'fkst sandbox lifecycle'            AND session_id = {session} )   )
//! ```
//!
//! The outer parentheses are mandatory and tested. Without them `AND … OR …`
//! would bind so that the lifecycle branch escapes the time window AND the actor
//! predicate — which is exactly the shape that turns "my calls plus this
//! session's system events" into "everybody's calls".
//!
//! ## One query, one column list
//!
//! The two contracts are selected through ONE column superset rather than a
//! `UNION ALL` of two shapes: a missing property is `NULL`, `event` says which
//! contract a row belongs to, and one `ORDER BY` then produces a single
//! deterministic timeline. A union would need two identically-shaped SELECTs and
//! a wrapper, all of which is more fixed text to get subtly wrong.

use serde_json::{json, Map, Value};

use crate::audit::event::{EVENT_NAME, INCOMPLETE_EVENT_NAME};
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;
use crate::session_access::ActivityVisibilityConstraint;

use super::filters::RecordKind;
use super::source::SourceQuery;

/// The projected columns, aliased so the decoder can key on stable NAMES rather
/// than on positions — a source that reorders its columns must not silently
/// re-map every field.
const SELECT_COLUMNS: &str = "\
    event AS event, \
    timestamp AS row_timestamp, \
    properties.event_id AS event_id, \
    properties.request_id AS request_id, \
    properties.started_at AS started_at, \
    properties.completed_at AS completed_at, \
    properties.occurred_at AS occurred_at, \
    properties.method AS method, \
    properties.route_template AS route_template, \
    properties.operation_id AS operation_id, \
    properties.arguments AS arguments, \
    properties.arguments_parse_status AS arguments_parse_status, \
    properties.status_code AS status_code, \
    properties.outcome AS outcome, \
    properties.error_code AS error_code, \
    properties.duration_ms AS duration_ms, \
    properties.actor_kind AS actor_kind, \
    properties.actor_id AS actor_id, \
    properties.actor_login AS actor_login, \
    properties.principal_kind AS principal_kind, \
    properties.principal_id AS principal_id, \
    properties.session_id AS session_id, \
    properties.repo_full_name AS repo_full_name, \
    properties.installation_id AS installation_id, \
    properties.trigger_issue AS trigger_issue, \
    properties.lifecycle_action AS lifecycle_action, \
    properties.backend AS backend, \
    properties.runtime_id AS runtime_id, \
    properties.creator_id AS creator_id, \
    properties.creator_login AS creator_login, \
    properties.trigger_author_id AS trigger_author_id, \
    properties.trigger_author_login AS trigger_author_login, \
    properties.created_at AS created_at, \
    properties.reason_code AS reason_code";

/// The total order every page is cut from. `event_id` breaks ties so rows
/// sharing a millisecond still page deterministically.
const ORDER_BY: &str = "ORDER BY timestamp DESC, properties.event_id DESC";

/// A rendered HogQL query and its parameter map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HogqlQuery {
    /// Fixed, server-owned text. Contains placeholder NAMES only.
    pub query: String,
    /// Every caller-influenced value, by placeholder name.
    pub values: Map<String, Value>,
}

impl HogqlQuery {
    /// The `POST /api/projects/{id}/query/` request body.
    pub fn request_body(&self) -> Value {
        json!({
            "query": {
                "kind": "HogQLQuery",
                "query": self.query,
                "values": Value::Object(self.values.clone()),
            }
        })
    }
}

/// Collects fixed predicate snippets and their parameter bindings.
///
/// A binding's NAME is always a compile-time constant, so the query text this
/// produces is a function of the request's SHAPE only — never of its content.
#[derive(Default)]
struct Bindings {
    values: Map<String, Value>,
}

impl Bindings {
    /// Bind `value` to the fixed `name` and return its placeholder.
    fn bind(&mut self, name: &'static str, value: Value) -> String {
        self.values.insert(name.to_string(), value);
        format!("{{{name}}}")
    }
}

/// Build the fixed query for one already-authorized source read.
pub fn build(query: &SourceQuery) -> HogqlQuery {
    let mut bindings = Bindings::default();
    let mut predicates = vec![
        format!(
            "timestamp >= {}",
            bindings.bind("range_from", json!(query.range.from_rfc3339()))
        ),
        format!(
            "timestamp < {}",
            bindings.bind("range_to", json!(query.range.to_rfc3339()))
        ),
        visibility_predicate(query, &mut bindings),
    ];
    predicates.extend(filter_predicates(query, &mut bindings));
    if let Some(cursor) = &query.cursor {
        let timestamp = bindings.bind("cursor_timestamp", json!(cursor.timestamp_rfc3339()));
        let event_id = bindings.bind("cursor_event_id", json!(cursor.event_id.clone()));
        predicates.push(format!(
            "(timestamp < {timestamp} OR (timestamp = {timestamp} \
             AND properties.event_id < {event_id}))"
        ));
    }
    let limit = bindings.bind("page_limit", json!(query.fetch_limit));

    HogqlQuery {
        query: format!(
            "SELECT {SELECT_COLUMNS} FROM events WHERE {} {ORDER_BY} LIMIT {limit}",
            predicates.join(" AND ")
        ),
        values: bindings.values,
    }
}

/// The mandatory row-visibility predicate: the kind/event allowlist joined with
/// the verified actor and the authorized session.
fn visibility_predicate(query: &SourceQuery, bindings: &mut Bindings) -> String {
    let viewer_id = match &query.constraint {
        ActivityVisibilityConstraint::Mine(scope) => Some(scope.actor_id()),
        ActivityVisibilityConstraint::All(_) => None,
    };
    let session_id = query.authorized_session_id().map(str::to_string);

    match query.record_kind {
        RecordKind::ApiRequest => api_request_branch(bindings, viewer_id, session_id.as_deref()),
        RecordKind::SandboxLifecycle => lifecycle_branch(bindings, session_id.as_deref()),
        // The parentheses below are load-bearing; see the module docs.
        RecordKind::All => format!(
            "({} OR {})",
            api_request_branch(bindings, viewer_id, session_id.as_deref()),
            lifecycle_branch(bindings, session_id.as_deref())
        ),
    }
}

/// The API-request branch: the two request event names, the verified actor id in
/// personal scope, and the authorized session when one was required.
///
/// The branch is parenthesized in its own right so it can be OR-ed with the
/// lifecycle branch without any predicate escaping it.
fn api_request_branch(
    bindings: &mut Bindings,
    viewer_id: Option<i64>,
    session_id: Option<&str>,
) -> String {
    let completed = bindings.bind("event_request_completed", json!(EVENT_NAME));
    let incomplete = bindings.bind("event_request_incomplete", json!(INCOMPLETE_EVENT_NAME));
    let mut parts = vec![format!("event IN ({completed}, {incomplete})")];
    if let Some(viewer_id) = viewer_id {
        parts.push(format!(
            "properties.actor_id = {}",
            bindings.bind("viewer_actor_id", json!(viewer_id))
        ));
    }
    if let Some(session_id) = session_id {
        parts.push(format!(
            "properties.session_id = {}",
            bindings.bind("authorized_session_id", json!(session_id))
        ));
    }
    format!("({})", parts.join(" AND "))
}

/// The lifecycle branch: the system transition event, plus the authorized session
/// when the caller is not a global administrator.
fn lifecycle_branch(bindings: &mut Bindings, session_id: Option<&str>) -> String {
    let lifecycle = bindings.bind("event_sandbox_lifecycle", json!(LIFECYCLE_EVENT_NAME));
    let mut parts = vec![format!("event = {lifecycle}")];
    if let Some(session_id) = session_id {
        parts.push(format!(
            "properties.session_id = {}",
            bindings.bind("authorized_session_id", json!(session_id))
        ));
    }
    format!("({})", parts.join(" AND "))
}

/// The optional narrowing predicates. Each is a fixed snippet selected by the
/// PRESENCE of a validated filter; the value itself only ever reaches `values`.
fn filter_predicates(query: &SourceQuery, bindings: &mut Bindings) -> Vec<String> {
    let filters = &query.filters;
    let mut predicates = Vec::new();
    if let Some(actor_id) = filters.actor_id {
        predicates.push(format!(
            "properties.actor_id = {}",
            bindings.bind("filter_actor_id", json!(actor_id))
        ));
    }
    if let Some(actor_login) = &filters.actor_login {
        predicates.push(format!(
            "properties.actor_login = {}",
            bindings.bind("filter_actor_login", json!(actor_login))
        ));
    }
    if let Some(operation_id) = &filters.operation_id {
        predicates.push(format!(
            "properties.operation_id = {}",
            bindings.bind("filter_operation_id", json!(operation_id))
        ));
    }
    if let Some(method) = &filters.method {
        predicates.push(format!(
            "properties.method = {}",
            bindings.bind("filter_method", json!(method))
        ));
    }
    if let Some(status_code) = filters.status_code {
        predicates.push(format!(
            "properties.status_code = {}",
            bindings.bind("filter_status_code", json!(status_code))
        ));
    }
    if let Some(status_class) = filters.status_class {
        let (low, high) = status_class.bounds();
        predicates.push(format!(
            "(properties.status_code >= {} AND properties.status_code < {})",
            bindings.bind("filter_status_low", json!(low)),
            bindings.bind("filter_status_high", json!(high))
        ));
    }
    if let Some(outcome) = filters.outcome {
        predicates.push(format!(
            "properties.outcome = {}",
            bindings.bind("filter_outcome", json!(outcome.as_str()))
        ));
    }
    if let Some(session_id) = &filters.session_id {
        predicates.push(format!(
            "properties.session_id = {}",
            bindings.bind("filter_session_id", json!(session_id))
        ));
    }
    if let Some(repo) = &filters.repo_full_name {
        predicates.push(format!(
            "properties.repo_full_name = {}",
            bindings.bind("filter_repo_full_name", json!(repo))
        ));
    }
    if let Some(trigger_issue) = filters.trigger_issue {
        predicates.push(format!(
            "properties.trigger_issue = {}",
            bindings.bind("filter_trigger_issue", json!(trigger_issue))
        ));
    }
    if let Some(request_id) = &filters.request_id {
        predicates.push(format!(
            "properties.request_id = {}",
            bindings.bind("filter_request_id", json!(request_id))
        ));
    }
    predicates
}

#[cfg(test)]
#[path = "hogql_tests.rs"]
mod tests;
