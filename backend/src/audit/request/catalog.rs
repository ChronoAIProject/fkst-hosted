//! The verified `(method, route template) -> operationId` catalog.
//!
//! The catalog is built from the SAME [`utoipa::openapi::OpenApi`] value that is
//! served at `/openapi.json`, so a record's `operation_id` is by construction the
//! id a client reads in the published contract — there is no second, hand-kept
//! list to drift.
//!
//! Three properties are enforced at build time, because each of them would
//! silently corrupt the audit trail rather than fail loudly:
//!
//! - **every operation declares an `operationId`** (an unnamed operation would
//!   record as `<unmatched>` and look like unknown traffic);
//! - **ids are globally unique** (a duplicate would merge two endpoints into one
//!   row set in every dashboard and scoped query);
//! - **every id has an explicit audit policy AND, when audited, an explicit
//!   safe-argument policy** (see [`super::policy`]) — the two halves of the
//!   decision travel together, so an endpoint cannot be recorded without anyone
//!   having decided what its record may contain.
//!
//! The lookup key is the *normalized* template. axum's matcher speaks `:param` /
//! `*wildcard` while OpenAPI speaks `{param}`, so [`normalize_matched_path`]
//! translates the former into the latter — that translation is the only place
//! the two vocabularies meet.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::Method;
use utoipa::openapi::path::Operation;
use utoipa::openapi::OpenApi;

use super::policy::{
    operation_for, undocumented_route_policy, ArgumentsPolicy, ExclusionReason, OperationPolicy,
};
use crate::audit::event::{UNMATCHED_OPERATION_ID, UNMATCHED_ROUTE_TEMPLATE};

/// Why a catalog cannot be built. Every variant names a documented template or
/// operation id — public contract values, never request data.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CatalogError {
    #[error("openapi operation {method} {path} declares no operationId")]
    MissingOperationId { method: &'static str, path: String },
    #[error("openapi operationId `{operation_id}` is declared by more than one operation")]
    DuplicateOperationId { operation_id: String },
    #[error("openapi declares {method} {path} more than once")]
    DuplicateRoute { method: &'static str, path: String },
    #[error(
        "openapi operationId `{operation_id}` has no audit policy; add an explicit \
         Audited or Excluded entry to audit::request::policy::OPERATION_POLICIES"
    )]
    UnpolicedOperation { operation_id: String },
    #[error(
        "openapi operationId `{operation_id}` is audited but declares no safe-argument \
         policy; give it an ArgumentsPolicy::Safe(..) naming its DTO, or \
         ArgumentsPolicy::None when it genuinely takes no arguments"
    )]
    MissingArgumentPolicy { operation_id: String },
}

/// One documented operation and its audit policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntry {
    /// Shared so a per-request lookup clones a pointer, not a string.
    pub operation_id: Arc<str>,
    pub policy: OperationPolicy,
}

/// What the middleware should do with a request, decided before the handler runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteDecision {
    /// Record a terminal event under this identity.
    Record {
        operation_id: Arc<str>,
        route_template: Arc<str>,
    },
    /// Produce no record, for a stated bounded reason.
    Skip(ExclusionReason),
}

/// The verified operation catalog.
#[derive(Debug)]
pub struct OperationCatalog {
    entries: HashMap<(&'static str, String), CatalogEntry>,
    unmatched_operation: Arc<str>,
    unmatched_template: Arc<str>,
}

impl Default for OperationCatalog {
    /// An empty catalog: every matched route resolves as `<unmatched>`.
    ///
    /// Written by hand rather than derived because the sentinels must be the
    /// contract's constants — a derived `Default` would hand out empty strings,
    /// which the event contract rejects.
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            unmatched_operation: Arc::from(UNMATCHED_OPERATION_ID),
            unmatched_template: Arc::from(UNMATCHED_ROUTE_TEMPLATE),
        }
    }
}

/// The HTTP methods an OpenAPI `PathItem` can carry, paired with their field.
/// Enumerated rather than derived because utoipa models them as eight distinct
/// `Option<Operation>` fields.
fn operations(item: &utoipa::openapi::path::PathItem) -> [(&'static str, &Option<Operation>); 8] {
    [
        ("GET", &item.get),
        ("PUT", &item.put),
        ("POST", &item.post),
        ("DELETE", &item.delete),
        ("OPTIONS", &item.options),
        ("HEAD", &item.head),
        ("PATCH", &item.patch),
        ("TRACE", &item.trace),
    ]
}

impl OperationCatalog {
    /// Build and validate the catalog from the assembled document.
    pub fn from_openapi(doc: &OpenApi) -> Result<Self, CatalogError> {
        let mut catalog = Self::default();
        let entries = &mut catalog.entries;
        let mut seen_ids: HashMap<String, ()> = HashMap::new();

        for (path, item) in &doc.paths.paths {
            for (method, operation) in operations(item) {
                let Some(operation) = operation else { continue };
                let operation_id = operation.operation_id.clone().ok_or_else(|| {
                    CatalogError::MissingOperationId {
                        method,
                        path: path.clone(),
                    }
                })?;
                if seen_ids.insert(operation_id.clone(), ()).is_some() {
                    return Err(CatalogError::DuplicateOperationId { operation_id });
                }
                let declared = operation_for(&operation_id).ok_or_else(|| {
                    CatalogError::UnpolicedOperation {
                        operation_id: operation_id.clone(),
                    }
                })?;
                // The second half of the decision. An audited operation that
                // never chose an argument boundary would record an empty
                // `arguments` object forever and nobody would notice.
                if declared.policy.is_audited()
                    && declared.arguments == ArgumentsPolicy::NotRecorded
                {
                    return Err(CatalogError::MissingArgumentPolicy { operation_id });
                }
                let entry = CatalogEntry {
                    operation_id: Arc::from(operation_id.as_str()),
                    policy: declared.policy,
                };
                if entries.insert((method, path.clone()), entry).is_some() {
                    return Err(CatalogError::DuplicateRoute {
                        method,
                        path: path.clone(),
                    });
                }
            }
        }

        Ok(catalog)
    }

    /// How many operations the document declared.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every declared `operationId`, for coverage guards.
    pub fn operation_ids(&self) -> impl Iterator<Item = &str> {
        self.entries.values().map(|entry| &*entry.operation_id)
    }

    /// The entry for an already-normalized template, if the document declares it.
    pub fn lookup(&self, method: &Method, route_template: &str) -> Option<&CatalogEntry> {
        self.entries
            .get(&(static_method(method)?, route_template.to_string()))
    }

    /// Decide a request's audit identity from its method and axum matched path.
    ///
    /// `matched` is `None` for a request that reached the router's fallback —
    /// i.e. an unknown path, whose raw value is deliberately discarded.
    pub fn resolve(&self, method: &Method, matched: Option<&str>) -> RouteDecision {
        // A preflight is answered by the CORS layer before any handler and
        // expresses no user intent, so it is excluded regardless of target.
        if method == Method::OPTIONS {
            return RouteDecision::Skip(ExclusionReason::CorsPreflight);
        }
        let Some(matched) = matched else {
            return RouteDecision::Record {
                operation_id: self.unmatched_operation.clone(),
                route_template: self.unmatched_template.clone(),
            };
        };
        let template = normalize_matched_path(matched);
        if let Some(decision) = self.declared_decision(method, &template) {
            return decision;
        }
        // axum answers HEAD from the GET handler unless a route registers its own
        // (`MethodRouter::get` installs both), so a HEAD request must resolve to
        // the SAME policy as the GET it is actually served by. Without this
        // fall-back a HEAD uptime probe or load-balancer check against `/health`,
        // `/ready`, `/metrics`, or `/openapi.json` would miss its exclusion entry
        // and be recorded as unknown traffic — exactly the probe/scrape noise the
        // exclusions exist to keep out.
        if *method == Method::HEAD {
            if let Some(decision) = self.declared_decision(&Method::GET, &template) {
                return decision;
            }
        }
        // A matched route with no documented operation for this method or its GET
        // fall-back: a method axum matched the path for but no handler serves (the
        // `405` path). The template is a documented constant, so it is safe to
        // keep even though the operation is unknown.
        RouteDecision::Record {
            operation_id: self.unmatched_operation.clone(),
            route_template: Arc::from(template.as_str()),
        }
    }

    /// The decision that a documented operation — or an explicitly policed route
    /// that carries no OpenAPI operation — dictates for `method`, or `None` when
    /// neither names this `(method, template)` pair.
    fn declared_decision(&self, method: &Method, template: &str) -> Option<RouteDecision> {
        let policy = match self.lookup(method, template) {
            Some(entry) => {
                return Some(match entry.policy {
                    OperationPolicy::Audited => RouteDecision::Record {
                        operation_id: entry.operation_id.clone(),
                        route_template: Arc::from(template),
                    },
                    OperationPolicy::Excluded(reason) => RouteDecision::Skip(reason),
                })
            }
            None => undocumented_route_policy(method.as_str(), template)?,
        };
        Some(match policy {
            OperationPolicy::Excluded(reason) => RouteDecision::Skip(reason),
            OperationPolicy::Audited => RouteDecision::Record {
                operation_id: self.unmatched_operation.clone(),
                route_template: Arc::from(template),
            },
        })
    }
}

/// axum's matcher vocabulary is not OpenAPI's: translate `:param` and `*rest`
/// path segments into `{param}` / `{rest}` so a matched path can be looked up in
/// the generated document.
pub fn normalize_matched_path(matched: &str) -> String {
    if !matched.contains(':') && !matched.contains('*') {
        return matched.to_string();
    }
    matched
        .split('/')
        .map(|segment| match segment.as_bytes().first() {
            Some(b':') | Some(b'*') => format!("{{{}}}", &segment[1..]),
            _ => segment.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Map a [`Method`] onto the `'static` string the catalog keys on. `None` for an
/// extension method, which no documented operation can declare.
fn static_method(method: &Method) -> Option<&'static str> {
    Some(match *method {
        Method::GET => "GET",
        Method::PUT => "PUT",
        Method::POST => "POST",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        Method::HEAD => "HEAD",
        Method::PATCH => "PATCH",
        Method::TRACE => "TRACE",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
