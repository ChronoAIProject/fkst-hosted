//! A tracing span for HTTP requests that cannot leak a URI.
//!
//! `tower_http`'s [`DefaultMakeSpan`](tower_http::trace::DefaultMakeSpan) records
//! `method`, `uri`, and `version` — and `uri` is the RAW request target,
//! query string included. On this surface that single field would put an OAuth
//! `?code=…&state=…`, a presigned storage URL, and any secret a client
//! mistakenly placed in a query into every request span, at `DEBUG` level, in
//! plain text. Since a span's fields are attached to every event recorded inside
//! it, that leak would then follow the request through every log line the
//! handler emits.
//!
//! So the default is replaced outright. This span carries the method and the
//! normalized request id and nothing else. Route template, operation id, status,
//! duration, and the stable error code are emitted once by the audit middleware
//! itself, which has resolved them from the matched route rather than the URI.

use axum::http::Request;
use tower_http::trace::MakeSpan;
use tracing::Span;

use super::id::{is_acceptable, REQUEST_ID_HEADER};

/// A [`MakeSpan`] that records no URI, path, query, or header value.
#[derive(Clone, Copy, Debug, Default)]
pub struct SafeHttpSpan;

impl<B> MakeSpan<B> for SafeHttpSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        // The audit middleware runs OUTSIDE this layer and has already replaced
        // any unacceptable client value, so the header is normalized by the time
        // it is read here. The acceptance check is kept anyway: this type must
        // be safe wherever it is mounted, not only below that middleware.
        let request_id = request
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| is_acceptable(value))
            .unwrap_or("");
        tracing::info_span!(
            "http_request",
            method = %request.method(),
            request_id = %request_id,
        )
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
