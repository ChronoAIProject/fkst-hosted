//! Runtime OpenAPI 3 document for the control-plane's public HTTP surface.
//!
//! There is **no static spec file**: the document is assembled at startup from
//! the live `#[utoipa::path]`-annotated handlers and `ToSchema` types as they are
//! registered on the [`OpenApiRouter`](utoipa_axum::router::OpenApiRouter) in
//! [`crate::router::build_router`]. The route registration IS the documented
//! path, so the spec can never drift from the code. The assembled value is
//! served verbatim at `GET /openapi.json`.
//!
//! [`ApiDoc`] contributes only the document-level metadata (info + tags); the
//! paths and component schemas are collected from the routers. The crate version
//! flows in automatically (utoipa defaults `info.version` to `CARGO_PKG_VERSION`,
//! the workspace's unified version).

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use utoipa::OpenApi;

use crate::error::AppError;
use crate::state::AppState;

/// Document-level OpenAPI metadata. Paths + component schemas are NOT listed
/// here — they are collected from the live routers at assembly time so the spec
/// tracks the code with zero drift.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "fkst-hosted control plane API",
        description = "Public REST surface of the fkst-hosted control plane \
            (ChronoAI's hosted fkst cloud). This document is generated at runtime \
            from the live Axum routes and Rust types — it is never a hand-written \
            file. The `/api/v1/*` session routes are open, read-only, and \
            network-isolated (no application-level auth); `/health`, `/metrics`, \
            and `/openapi.json` are likewise public; the only authenticated \
            inbound is the signature-verified GitHub App webhook. The fleet-only \
            `/internal/v1/*` worker protocol is intentionally NOT part of this \
            public contract."
    ),
    tags(
        (name = "users", description = "Per-user environment + secret store (GitHub-token authenticated)."),
        (name = "system", description = "Liveness and Prometheus metrics (public)."),
        (name = "webhooks", description = "Inbound GitHub App webhook (signature-verified, public).")
    )
)]
pub struct ApiDoc;

/// The document-level metadata seed handed to the `OpenApiRouter`. The routers
/// merge their collected paths + schemas onto this base.
pub fn api_doc() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// Build the `GET /openapi.json` route that serves the assembled spec.
///
/// The document is serialized to JSON ONCE here (a serialization failure is a
/// startup-time 500 surfaced as [`AppError`], never a per-request cost) and the
/// rendered body is shared behind an [`Arc`]; each request clones the string and
/// returns it with an `application/json` content type. The route is mounted at
/// the TOP level (unauthenticated, like `/health`) so any client — including the
/// frontend's codegen — can fetch the contract without an identity.
pub fn spec_route(spec: utoipa::openapi::OpenApi) -> Result<Router<AppState>, AppError> {
    let body = spec.to_json().map_err(|error| {
        AppError::Internal(anyhow::anyhow!("failed to render OpenAPI spec: {error}"))
    })?;
    let body = Arc::new(body);

    let route = get(move || {
        let body = body.clone();
        async move {
            (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                (*body).clone(),
            )
        }
    });

    Ok(Router::new().route("/openapi.json", route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_doc_carries_info_and_no_security_scheme() {
        let doc = api_doc();
        assert_eq!(doc.info.title, "fkst-hosted control plane API");
        // utoipa defaults info.version to the crate version (the unified version).
        assert_eq!(doc.info.version, env!("CARGO_PKG_VERSION"));
        // No application-level auth: the document registers no security scheme
        // (the API is open, read-only, and network-isolated).
        if let Some(components) = doc.components {
            assert!(
                components.security_schemes.is_empty(),
                "no security scheme should be registered, found: {:?}",
                components.security_schemes.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn spec_route_renders_without_error() {
        // A well-formed OpenAPI value must serialize; the route builds.
        let _router = spec_route(api_doc()).expect("spec route builds");
    }
}
