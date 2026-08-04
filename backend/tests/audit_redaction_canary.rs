//! The ABSENCE half of the safe-argument contract: unique hostile values in
//! every location a request can carry one, then a search of every observability
//! surface the epic names.
//!
//! Surfaces covered here:
//!
//! - the recorded events and their exact PostHog capture payloads;
//! - the metrics exposition (epic `OPS-04`: bounded labels only);
//! - structured trace output at TRACE, the most verbose level anything runs at;
//! - the request context's own `Debug` rendering — what a panic, a `{:?}` of a
//!   request, or a rejection dump would print.
//!
//! Its sibling `audit_safe_arguments` asserts the values that must still BE
//! there, so nothing here can pass by recording nothing at all.

mod audit_canary;

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use audit_canary::{assert_no_canaries, plant_every_canary, Canary, CANARIES};
use fkst_control_plane::audit::request::AuditRequestContext;
use fkst_control_plane::audit::ArgumentsParseStatus;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;

/// An in-memory `tracing` writer.
#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn lock(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.lock()).into_owned()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The absence half, across every recorded record and its exact PostHog payload.
#[tokio::test]
async fn no_canary_reaches_a_record_or_its_posthog_payload() {
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;
    assert_no_canaries(&canary);
}

/// The CONTENT half of the log canary, which only means something if the read
/// really happened: the served response must carry the canary, and the record
/// must carry the file's CLASS instead.
#[tokio::test]
async fn a_served_log_file_keeps_its_content_out_of_the_record() {
    let canary = Canary::start().await;
    let response = canary
        .call(canary.authenticated(axum::http::Request::get(format!(
            "/api/v1/logs/{}/file?path={}&tail_bytes=4096",
            audit_canary::log_bundle::SESSION,
            audit_canary::log_bundle::FILE_PATH
        ))))
        .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let served = audit_canary::body_text(response).await;
    assert!(
        served.contains(audit_canary::log_bundle::FILE_CONTENT),
        "the fixture must actually serve the canary, or this test proves nothing: {served}"
    );

    let event = canary.event("session_log_file");
    let rendered = audit_canary::rendered(&event);
    assert!(
        !rendered.contains(audit_canary::log_bundle::FILE_CONTENT),
        "log content reached the record:\n{rendered}"
    );
    assert!(
        !rendered.contains(audit_canary::log_bundle::FILE_PATH),
        "the requested path reached the record:\n{rendered}"
    );
    // What it DOES carry: the bundle's own bounded class for that path.
    assert_eq!(
        event.arguments.get("file_class"),
        Some(&serde_json::json!("codex"))
    );
}

/// The metrics exposition is a separate surface with its own label rules (epic
/// `OPS-04`): no request value may become a label there either.
#[tokio::test]
async fn no_canary_reaches_the_metrics_exposition() {
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;
    let metrics = canary.metrics_text().await;
    assert!(
        metrics.contains("fkst_audit_"),
        "the metrics surface produced no audit series at all:\n{metrics}"
    );
    for planted in CANARIES {
        assert!(
            !metrics.contains(planted),
            "{planted} reached the metrics exposition:\n{metrics}"
        );
    }
}

/// Some canaries reach an APPLICATION log line by existing, deliberate design,
/// and are therefore excluded from the trace assertion below rather than
/// silently passing. They fall into exactly two families:
///
/// - **an upstream error body, surfaced at `warn` so an operator can diagnose a
///   GitHub outage**: `canary-upstream-body` and the `canary-stack-frame` beside
///   it in the same body. The body is logged verbatim, so anything an upstream
///   puts in it is logged; that is the diagnostic value and also its limit.
/// - **the caller's OWN rejected input, quoted back inside an `AppError`
///   message** which [`fkst_control_plane::error`] logs at `debug`:
///   `canary-invalid-branch` ("branch `…` is invalid"),
///   `canary-invalid-package-ref` ("lists an invalid package reference `…`"),
///   and `canary-log-path` ("no such log file: …"). None is a credential, and
///   none is another user's data — quoting the rejected value is what makes the
///   error actionable.
///
/// None of them is an audit property: all are covered — and asserted absent — by
/// [`no_canary_reaches_a_record_or_its_posthog_payload`], which excludes nothing,
/// and the log path is separately proven to be replaced by its class in
/// `audit_safe_arguments`. What this test proves is the narrower, harder
/// property: no request value reaches the request SPAN or the audit pipeline's
/// own log lines.
///
/// Every CREDENTIAL canary is deliberately absent from this list. A token in a
/// log line is a leak whatever level it was written at, so the assertion below
/// covers `canary-access-token`, `canary-refresh-token`,
/// `canary-rotated-refresh-token`, `canary-llm-api-key`,
/// `canary-posthog-query-key`, `canary-storage-client-secret`, and the two URL
/// canaries with no exception at all.
const TRACED_BY_DESIGN: &[&str] = &[
    "canary-upstream-body",
    "canary-stack-frame",
    "canary-invalid-branch",
    "canary-invalid-package-ref",
    "canary-log-path",
];

/// Structured logging at TRACE, the most verbose level anything is ever run at.
#[tokio::test]
async fn no_request_canary_reaches_structured_trace_output() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(false)
        .with_target(true)
        .with_writer(logs.clone())
        .finish();
    // A thread-local default: `#[tokio::test]` uses the current-thread runtime,
    // so every poll of the router future happens on this thread and is covered.
    let guard = tracing::subscriber::set_default(subscriber);
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;
    drop(guard);

    let captured = logs.text();
    assert!(
        !captured.is_empty(),
        "the capture harness recorded nothing, so it proves nothing"
    );
    for planted in CANARIES {
        if TRACED_BY_DESIGN.contains(planted) {
            continue;
        }
        assert!(
            !captured.contains(planted),
            "{planted} leaked into trace output:\n{captured}"
        );
    }
}

/// The safe correlation fields must actually be logged — a span that carried
/// nothing would pass the canary assertion above for entirely the wrong reason.
#[tokio::test]
async fn the_safe_request_fields_are_still_traced() {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let canary = Canary::start().await;
    canary
        .call(canary.authenticated(axum::http::Request::get("/api/v1/overview")))
        .await;
    drop(guard);

    let captured = logs.text();
    assert!(captured.contains("canvas_overview"), "{captured}");
    assert!(captured.contains("/api/v1/overview"), "{captured}");
}

/// The request context's own `Debug` is what a panic, a `{:?}` of a request, or
/// a rejection dump prints. It renders WHICH slots are filled and never what is
/// in them, so no amount of debug logging can turn it into a leak.
#[test]
fn the_request_context_debug_rendering_names_no_value() {
    let context = AuditRequestContext::new();
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        "session_id".to_string(),
        serde_json::json!("canary-context-session"),
    );
    context.record_arguments(arguments, ArgumentsParseStatus::Parsed);
    context.record_session_id("canary-context-session");
    context.record_repo_full_name("canary-context-owner/canary-context-repo");
    context.record_error_code("forbidden");
    context.record_webhook_delivery_id("canary-context-delivery");
    context.record_installation_id(7);
    context.record_trigger_issue(42);

    let rendered = format!("{context:?}");
    for planted in [
        "canary-context-session",
        "canary-context-owner",
        "canary-context-repo",
        "canary-context-delivery",
    ] {
        assert!(!rendered.contains(planted), "{planted} in {rendered}");
    }
    // What it DOES say: which slots are filled, so the rendering is still useful.
    assert!(rendered.contains("session_id: true"), "{rendered}");
    assert!(rendered.contains("arguments: true"), "{rendered}");
    assert!(rendered.contains("conflicts: 0"), "{rendered}");
}
