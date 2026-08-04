//! The relay's internal HTTP surface.
//!
//! ```text
//! POST /internal/v1/audit/request-starts            [write]  write token
//! PUT  /internal/v1/audit/requests/{id}/completion  [write]  write token
//! POST /internal/v1/audit/events                    [write]  write token
//! GET  /internal/v1/audit/records                   [read]   read token
//! GET  /health  /ready  /metrics                    [ops]    unauthenticated
//! ```
//!
//! ## Why this is a plain `axum::Router`, not an `OpenApiRouter`
//!
//! The repository's OpenAPI contract covers the control plane's PUBLIC surface —
//! the document served at `/openapi.json` from `build_router`. This is a
//! different process with no public surface at all: a ClusterIP service, no
//! Ingress, and a NetworkPolicy that admits only the control plane's
//! ServiceAccount and Prometheus. Publishing it into the product's client-facing
//! contract would advertise an API no client may call.
//!
//! ## The ops endpoints are deliberately unauthenticated
//!
//! `/health`, `/ready`, and `/metrics` carry no record content — only bounded
//! counters and a boolean — and kubelet probes cannot present a bearer token.
//! `/metrics` is reachable only from Prometheus by NetworkPolicy, and every
//! series it exposes has closed-enum labels, so scraping it reveals nothing
//! about any actor, session, or request.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::Router;

use super::auth::RelayTokens;
use super::config::RelayConfig;
use super::db::Database;
use super::metrics::RelayMetrics;

mod error;
mod ops;
mod read;
mod write;

pub use error::{RelayError, RelayResult};

/// The state every relay handler shares.
#[derive(Clone, Debug)]
pub struct RelayState {
    pub db: Database,
    pub tokens: RelayTokens,
    pub metrics: RelayMetrics,
    pub config: Arc<RelayConfig>,
    /// Set by the worker's sweep when the record count reaches
    /// `FKST_AUDIT_RELAY_MAX_RECORDS`. Checked per ingress call, so refusing is
    /// one atomic read rather than a `COUNT(*)` on the hot path.
    pub at_capacity: Arc<AtomicBool>,
}

impl RelayState {
    /// Build the shared state for a configured relay.
    pub fn new(db: Database, config: Arc<RelayConfig>, metrics: RelayMetrics) -> Self {
        let tokens = RelayTokens::new(config.write_token.clone(), config.read_token.clone());
        Self {
            db,
            tokens,
            metrics,
            config,
            at_capacity: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether ingress must be refused for capacity.
    pub fn is_at_capacity(&self) -> bool {
        self.at_capacity.load(Ordering::Relaxed)
    }
}

/// Build the relay router.
pub fn build_router(state: RelayState) -> Router {
    let max_body = state.config.max_body_bytes;
    Router::new()
        .route(
            "/internal/v1/audit/request-starts",
            post(write::post_request_start),
        )
        .route(
            "/internal/v1/audit/requests/:event_id/completion",
            put(write::put_request_completion),
        )
        .route("/internal/v1/audit/events", post(write::post_event))
        .route("/internal/v1/audit/records", get(read::get_records))
        .route("/health", get(ops::health))
        .route("/ready", get(ops::ready))
        .route("/metrics", get(ops::metrics))
        // One bounded body limit for the whole surface: a relay that buffered an
        // unbounded body would let one caller consume the memory every other
        // caller's durable commit needs.
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(state)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
