//! Audited extractors: the seam that records `invalid` for input a parser
//! rejected BEFORE any handler runs.
//!
//! A malformed body, an unparseable path segment, or a bad query value is
//! answered by axum's own extractor, so the handler never executes and never
//! gets to describe its arguments. Without a seam here those requests would all
//! record as `unavailable` — indistinguishable from an authentication rejection,
//! which is a very different operational story.
//!
//! Each wrapper below delegates to the axum extractor VERBATIM and returns its
//! rejection unchanged, so status codes, bodies, and content types are
//! byte-identical to before. The only additions are two things the middleware
//! cannot see for itself:
//!
//! - the bounded transport metadata of the rejected input
//!   ([`super::InvalidInput`] — normalized content type, declared length,
//!   observed bounded size, and nothing else);
//! - a stable `error_code` on the response, so the record names the failure
//!   class without anyone parsing a rejection message.
//!
//! ## What is deliberately NOT captured
//!
//! The raw body bytes, a lossy string of them, the serde error's message (which
//! quotes the offending input), the raw path segment, and the query string. All
//! of them are exactly the "never echo invalid material" rule, and the ONE
//! reason this module exists is to make the rejection legible without them.
//!
//! ## Why the body is buffered here
//!
//! [`AuditedJson`] buffers with `Bytes::from_request` — under the SAME
//! `DefaultBodyLimit` axum would apply — and then hands the bytes to
//! [`axum::Json::from_bytes`]. That is the identical sequence
//! `Json::<T>::from_request` performs internally, so the parse, the rejection
//! variants, and the limit behaviour are unchanged; buffering it a step earlier
//! is what lets a syntax error report `body_bytes_observed` instead of guessing.

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;

use super::{record_invalid, InvalidInput};
use crate::audit::request::{codes, with_error_code};

/// `Json<T>`, with the rejected-body metadata recorded.
///
/// The inner value is public so a handler destructures it exactly like
/// `Json(value)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuditedJson<T>(pub T);

/// `Query<T>`, with a query-parse rejection recorded.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuditedQuery<T>(pub T);

/// `Path<T>`, with a path-parse rejection recorded.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuditedPath<T>(pub T);

/// A rejection that has already been recorded, rendered exactly as axum would.
///
/// Wrapping (rather than returning the axum rejection directly) is what lets the
/// stable error code be attached: the code is a `&'static str` constant, so no
/// rejection message can ride along with it.
#[derive(Debug)]
pub struct RecordedRejection<R> {
    rejection: R,
    code: &'static str,
}

impl<R: IntoResponse> IntoResponse for RecordedRejection<R> {
    fn into_response(self) -> Response {
        with_error_code(self.rejection.into_response(), self.code)
    }
}

/// The stable code for a body rejection, derived from the status axum chose.
///
/// A status is a bounded value, which is the whole reason it — and never the
/// rejection's text — is what selects the code.
fn body_rejection_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => codes::PAYLOAD_TOO_LARGE,
        StatusCode::UNSUPPORTED_MEDIA_TYPE => codes::UNSUPPORTED_MEDIA_TYPE,
        StatusCode::UNPROCESSABLE_ENTITY => codes::UNPROCESSABLE,
        _ => codes::INVALID_REQUEST,
    }
}

#[axum::async_trait]
impl<T, S> FromRequest<S> for AuditedJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = RecordedRejection<JsonRejection>;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Snapshot everything the audit record may need before the request is
        // consumed. `Extensions` is cloneable and carries the audit context plus
        // the route's `DefaultBodyLimit`, so the buffering step below applies the
        // same limit this route already had.
        let (parts, body) = request.into_parts();
        let extensions = parts.extensions.clone();
        let headers = parts.headers.clone();
        let metadata = InvalidInput::from_headers(&headers);

        let bytes = match Bytes::from_request(Request::from_parts(parts, body), state).await {
            Ok(bytes) => bytes,
            Err(rejection) => {
                // The limit fired (or the stream failed): nothing was observed,
                // so only the declared metadata is recorded.
                record_invalid(&extensions, &metadata);
                let rejection = JsonRejection::from(rejection);
                let code = body_rejection_code(rejection.status());
                return Err(RecordedRejection { rejection, code });
            }
        };
        let metadata = metadata.with_observed_bytes(bytes.len());

        // The content-type gate axum applies before parsing. Rebuilding a
        // request around the buffered bytes keeps that check — and the parse —
        // in axum's hands rather than reimplemented here.
        let mut rebuilt = Request::new(axum::body::Body::from(bytes));
        *rebuilt.headers_mut() = headers;
        *rebuilt.extensions_mut() = extensions.clone();
        match Json::<T>::from_request(rebuilt, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                record_invalid(&extensions, &metadata);
                let code = body_rejection_code(rejection.status());
                Err(RecordedRejection { rejection, code })
            }
        }
    }
}

#[axum::async_trait]
impl<T, S> FromRequestParts<S> for AuditedQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = RecordedRejection<QueryRejection>;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            // A query rejection contributes NO query metadata at all: the only
            // thing a failed query parse knows is the query string itself, which
            // is the one thing that may never be recorded.
            Err(rejection) => Err(reject_without_body(parts, rejection)),
        }
    }
}

#[axum::async_trait]
impl<T, S> FromRequestParts<S> for AuditedPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = RecordedRejection<PathRejection>;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => Ok(Self(value)),
            Err(rejection) => Err(reject_without_body(parts, rejection)),
        }
    }
}

/// Record a path/query rejection and wrap it with its stable code.
///
/// The request's headers still supply the declared transport metadata; no part
/// of the path or the query is read.
fn reject_without_body<R: IntoResponse>(parts: &Parts, rejection: R) -> RecordedRejection<R> {
    record_invalid(
        &parts.extensions,
        &InvalidInput::from_headers(&parts.headers),
    );
    RecordedRejection {
        rejection,
        code: codes::INVALID_REQUEST,
    }
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
