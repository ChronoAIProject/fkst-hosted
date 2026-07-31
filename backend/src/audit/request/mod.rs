//! The outer request-audit lifecycle: one terminal record per in-scope request.
//!
//! ```text
//! inbound request
//!   -> [id]        validate or generate `X-Request-Id`
//!   -> [catalog]   MatchedPath -> normalized template -> operationId
//!   -> [policy]    Audited | Excluded(reason)               (explicit, per operation)
//!   -> [context]   install the write-once AuditRequestContext
//!   -> inner service: CORS, trace, timeout, leader gate, extractors, handler
//!   -> [response]  read the typed stable error code / rejection marker
//!   -> [outcome]   status + markers -> AuditOutcome
//!   -> AuditHandle::submit -> AuditSink                     (never awaited)
//! ```
//!
//! ## Why the middleware is the outermost layer
//!
//! Everything that can answer a request without reaching a handler — the CORS
//! preflight short-circuit, the route-scoped `TimeoutLayer`, the leader-readiness
//! gate, extractor rejections, `AppError` conversion, and axum's own `404`/`405`
//! — is *inside* it. That is the only placement from which "every in-scope
//! request produces exactly one terminal record" (epic `AUD-01`) can be true,
//! and it is asserted by router-level tests rather than inferred from Tower's
//! layer order.
//!
//! It is applied through [`axum::Router::layer`], which axum maps over every
//! route service *and* the fallback/catch-all, so an unmatched path is audited
//! too — while still running after routing, which is what makes
//! [`axum::extract::MatchedPath`] available on entry.
//!
//! ## What never enters a record
//!
//! The raw URI, the query string, headers, the request or response body, and any
//! error text. The route identity comes from the matched template resolved
//! against the generated OpenAPI document ([`catalog`]); an unmatched path is
//! recorded as the `<unmatched>` sentinel precisely because its raw path may
//! carry OAuth material. Error *codes* arrive as a typed response extension
//! ([`response`]), so a streaming or oversized body is never buffered to inspect
//! it.

pub mod catalog;
pub mod context;
pub mod id;
pub mod middleware;
pub mod outcome;
pub mod policy;
pub mod response;
pub mod trace;

pub use catalog::{
    normalize_matched_path, CatalogEntry, CatalogError, OperationCatalog, RouteDecision,
};
pub use context::{with_context, AuditArguments, AuditRequestContext, FrozenRequestContext};
pub use id::{normalize_request_id, NormalizedRequestId, REQUEST_ID_HEADER};
pub use middleware::{audit_requests, AuditMiddleware};
pub use outcome::{derive_outcome, framework_error_code};
pub use policy::{
    arguments_policy_for, declared_operation_ids, default_arguments_status, operation_for,
    policy_for, ArgumentsPolicy, AuditOperation, ExclusionReason, OperationPolicy,
};
pub use response::{
    codes, tag_error_code, tag_rejected, with_browser_error, with_error_code, with_rejection,
    AuditErrorCode, AuditRejection,
};
pub use trace::SafeHttpSpan;
