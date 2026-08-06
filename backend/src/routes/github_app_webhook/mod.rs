//! GitHub App webhook endpoint (issue #108): `POST /api/v1/github/app/webhook`.
//!
//! UNAUTHENTICATED at the app layer but signature-verified. This route is
//! mounted at the top level (like `/health`) and authenticates the *sender* by
//! an HMAC over the body:
//!
//! 1. Read the body as raw [`Bytes`] — verification MUST run on the exact bytes
//!    GitHub signed. Deserializing then reserializing changes the bytes and
//!    breaks the MAC, so the order is strictly: read raw -> verify -> parse.
//! 2. Compute `HMAC-SHA256(secret, raw_body)` and compare it in CONSTANT TIME
//!    against the `sha256=<hex>` value in `X-Hub-Signature-256`. A missing or
//!    mismatched signature is `401` (never reveals which check failed).
//! 3. Only then trust the body's `sender`/`installation` as the delivery's audit
//!    identity, and record the delivery's correlation handles (see [`sender`]) —
//!    in an unverified body those fields are attacker-controlled — and parse
//!    `X-GitHub-Event` to dispatch.
//!
//! Stateless cache-bust hint (#141). The handler keeps signature verification,
//! parses the event to derive the affected `owner/name` set and installer login,
//! then evicts the token service's in-memory caches, fails any active session that
//! depended on an affected repo, and optionally launches best-effort trigger
//! seeding. There is no durable installation record to read or write: the App
//! layer resolves installations on demand and a stale mapping self-corrects at the
//! next mint (the `InstallationGone` backstop). The in-memory eviction is also
//! broadcast cluster-wide via the controller→worker seam on
//! [`crate::github_app::GithubAppTokens::evict_repo`] (a no-op until the channel is
//! wired, #134/#151). The handler is idempotent (GitHub redelivers) and returns
//! `2xx` quickly.
//!
//! Secret discipline: the webhook secret is never logged; the payload is parsed
//! only for the non-secret installation, repository, and sender fields used below.

mod evolution_trigger;
mod installation;
mod issue_trigger;
mod sender;
mod verify;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use secrecy::ExposeSecret;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::arguments::record;
use crate::audit::arguments::record_safe;
use crate::audit::arguments::webhook::{
    SafeGithubAppWebhook, VerifiedDeliveryInput, WebhookHandling,
};
use crate::audit::request::{codes, with_error_code, with_rejection};
use crate::state::AppState;

use installation::{handle_installation, handle_installation_repositories};
use verify::verify_signature;

/// Header carrying the event type (`installation`, `installation_repositories`).
const EVENT_HEADER: &str = "x-github-event";

/// Outcome of handling one event, for logging and the response code. Every arm
/// is a `2xx` to GitHub (even "ignored"): a non-2xx triggers a redelivery
/// storm, and an unknown/irrelevant event is not an error.
#[derive(Debug)]
enum Handled {
    /// Caches were busted (eviction + session-fail) for one or more repos / an
    /// owner. The stateless model has nothing durable to record.
    CacheBusted,
    /// Acknowledged but not acted on (unknown action, or a `created`/`unsuspend`
    /// that needs no cache bust — the next on-demand resolve picks it up).
    Ignored,
    /// The event's `(installation, repo)` was enqueued onto the Model B reconcile
    /// queue (issue #359, PR6). The webhook is a level-based *nudge*: the
    /// reconciler re-reads the repo's trigger issues + live pods and decides
    /// spawn/kill itself.
    Reconciled,
}

// ---- Handler ---------------------------------------------------------------

/// `POST /api/v1/github/app/webhook`. See the module docs for the strict
/// verify-then-parse ordering. The route is only mounted when a webhook secret
/// is configured (see `router.rs`), so a `None` secret here is defensive.
#[utoipa::path(
    post,
    path = "/api/v1/github/app/webhook",
    tag = "webhooks",
    operation_id = "github_app_webhook",
    request_body(
        content = serde_json::Value,
        content_type = "application/json",
        description = "Raw GitHub App webhook event. Recognized events: `installation` / \
            `installation_repositories` (cache-bust + Model B reconcile nudge) and `issues` \
            (reconcile nudge — the reconciler re-reads the repo and decides spawn/kill). \
            `issue_comment` is inert (a session is controlled purely through its trigger issue's \
            open/close + work-label changes). Authenticated by the `X-Hub-Signature-256` HMAC \
            over the exact body."
    ),
    responses(
        (status = 200, description = "Event handled (e.g. installation caches busted)"),
        (status = 202, description = "Event accepted (no action required)"),
        (status = 401, description = "Missing or mismatched webhook signature"),
        (status = 503, description = "Webhook secret not configured, or this election-enabled replica is not the ready leader")
    )
)]
async fn webhook(
    State(state): State<AppState>,
    extensions: Extensions,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // The secret must be configured for this route to do anything; the router
    // only mounts the route when it is set, so this is a defensive 503.
    let Some(secret) = &state.github_app_webhook_secret else {
        tracing::warn!("github webhook received but no webhook secret configured");
        // Nothing about this delivery has been verified, so it records exactly
        // what an unverified delivery may say about itself.
        record_safe(&extensions, &SafeGithubAppWebhook::rejected());
        return with_error_code(
            StatusCode::SERVICE_UNAVAILABLE.into_response(),
            codes::WEBHOOK_NOT_CONFIGURED,
        );
    };

    // STEP 1+2: verify the HMAC over the RAW bytes BEFORE any JSON parse.
    if !verify_signature(secret.expose_secret().as_bytes(), &headers, &body) {
        // Do not distinguish missing vs mismatched: both are 401, no detail.
        tracing::warn!("github webhook signature verification failed");
        // The ONLY property a rejected delivery contributes. Its claimed sender,
        // installation, repository, and issue are attacker-controlled until the
        // HMAC verifies, so none of them is recorded — nor is the signature.
        record_safe(&extensions, &SafeGithubAppWebhook::rejected());
        // A signature failure is an identity rejection, so the audit record says
        // `rejected` — never `client_error` — and never names the sender the
        // unverified body claims.
        return with_rejection(
            StatusCode::UNAUTHORIZED.into_response(),
            codes::WEBHOOK_SIGNATURE_INVALID,
        );
    }

    // STEP 3: the body is now PROVEN to be GitHub's, so — and only now — its
    // `sender`/`installation` fields may be trusted as identity, and the
    // delivery's own correlation handles may be recorded. A rejected delivery
    // never reaches this line, so a forged sender is never recorded.
    let delivery = sender::record_verified_delivery(&extensions, &headers, &body);

    // STEP 4: parse the event type, then dispatch on the verified body.
    let event = headers
        .get(EVENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let result = dispatch_event(&state, event.as_str(), &body).await;

    // Recorded after dispatch because `handling` is part of the safe argument
    // set, and only dispatch knows it. Everything else comes from the verified
    // body; the issue title/body and the repository lists never do.
    record(
        &extensions,
        &VerifiedDeliveryInput {
            event: event.as_str(),
            action: delivery.action.as_deref(),
            installation_id: delivery.installation_id,
            repo_owner: delivery.repo_owner.as_deref(),
            repo_name: delivery.repo_name.as_deref(),
            issue_number: delivery.issue_number,
            delivery_id: delivery.delivery_id.as_deref(),
            handling: match &result {
                Ok(handled) => handled.audit_handling(),
                Err(_) => WebhookHandling::ParseFailed,
            },
        },
    );

    match result {
        Ok(handled) => {
            tracing::info!(event = %event, outcome = handled.as_str(), "github webhook handled");
            StatusCode::OK.into_response()
        }
        Err(detail) => {
            // A processing failure (e.g. a malformed body or a store error) is
            // logged; we still return 202 so GitHub does not hammer redeliveries
            // for a payload we cannot act on. The detail never contains a secret.
            tracing::error!(event = %event, detail = %detail, "github webhook processing failed");
            StatusCode::ACCEPTED.into_response()
        }
    }
}

/// Route a verified webhook event to its handler. Split out of [`webhook`] so the
/// routing (which event → which outcome) is unit-testable without signature
/// verification. `installation` / `installation_repositories` keep the stateless
/// cache-bust AND additionally nudge the Model B reconciler; `issues` is a pure
/// reconcile nudge (PR6 flip); `issue_comment` is inert (the `/stop` + `/status`
/// control path was removed with Model A — a session is driven purely through its
/// trigger issue's open/close + work-label changes, which the reconciler reacts to).
async fn dispatch_event(state: &AppState, event: &str, body: &[u8]) -> Result<Handled, String> {
    match event {
        "installation" => handle_installation(state, body).await,
        "installation_repositories" => handle_installation_repositories(state, body).await,
        "issues" => issue_trigger::classify_and_enqueue(state, body).await,
        "issue_comment" => Ok(Handled::Ignored),
        // FKST Evolution reacts to what reaches the trusted branch, not only to
        // issue activity. Each of these is a thin classifier over the payload:
        // relevant events enqueue the same repository hint the issue path uses,
        // and the level-triggered reconciler decides what to do with it.
        "push" => evolution_trigger::classify_push(state, body).await,
        "pull_request" => evolution_trigger::classify_pull_request(state, body).await,
        "release" => evolution_trigger::classify_release(state, body).await,
        "repository" => evolution_trigger::classify_repository(state, body).await,
        other => {
            // ping / membership / etc. — acknowledged but not acted on.
            tracing::debug!(event = %other, "github webhook event ignored");
            Ok(Handled::Ignored)
        }
    }
}
impl Handled {
    fn as_str(&self) -> &'static str {
        match self {
            Handled::CacheBusted => "cache_busted",
            Handled::Ignored => "ignored",
            Handled::Reconciled => "reconciled",
        }
    }

    /// The audit contract's closed `handling` value for this outcome.
    ///
    /// Mapped rather than reused so the log string and the analytics property
    /// can evolve independently — a log line is read by a human, a property by a
    /// dashboard that must never see an unbounded value.
    fn audit_handling(&self) -> WebhookHandling {
        match self {
            Handled::CacheBusted => WebhookHandling::CacheBusted,
            Handled::Ignored => WebhookHandling::Ignored,
            Handled::Reconciled => WebhookHandling::Reconciled,
        }
    }
}

/// The webhook route, mounted UNAUTHENTICATED in `router.rs` (outside the
/// `/api/v1` auth nest) but signature-verified inside the handler.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(webhook))
}

#[cfg(test)]
#[path = "mod_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
