//! Transport behaviour against a mock PostHog: endpoint selection, payload
//! shape, response validation, and failure classification.

use std::time::Duration;

use serde_json::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::audit::config::AuditConfig;
use crate::audit::projection::EventLimits;
use crate::audit::test_support::{anonymous_event, human_event};

/// A capture client pointed at `server`. `FKST_DEPLOYMENT_ENVIRONMENT=test` is
/// what permits the mock's plaintext origin.
fn client_for(server: &MockServer, extra: &[(&str, &str)]) -> PostHogClient {
    let uri = server.uri();
    let pairs = crate::audit::test_support::merge_vars(
        &[
            ("FKST_POSTHOG_ENABLED", "true"),
            ("FKST_POSTHOG_HOST", uri.as_str()),
            ("FKST_POSTHOG_PROJECT_TOKEN", "phc_test_token"),
            ("FKST_DEPLOYMENT_ENVIRONMENT", "test"),
        ],
        extra,
    );
    let config = AuditConfig::from_vars(&pairs).expect("test config is valid");
    PostHogClient::from_config(&config).expect("client builds")
}

fn captured(event: crate::audit::event::ApiRequestCompletedV1) -> CaptureEvent {
    event
        .to_capture_event(EventLimits::new(65_536))
        .expect("fixture projects")
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("request body is JSON")
}

#[tokio::test]
async fn a_single_event_uses_the_capture_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capture/"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": 1})))
        .expect(1)
        .mount(&server)
        .await;

    let event = human_event();
    let projected = captured(event.clone());
    client_for(&server, &[])
        .capture(std::slice::from_ref(&projected))
        .await
        .expect("capture accepted");

    let requests = server.received_requests().await.expect("recorded");
    let body = body_of(&requests[0]);
    assert_eq!(body["api_key"], serde_json::json!("phc_test_token"));
    assert_eq!(
        body["event"],
        serde_json::json!("fkst api request completed")
    );
    assert_eq!(body["distinct_id"], serde_json::json!("github:583231"));
    assert_eq!(body["uuid"], serde_json::json!(event.event_id.to_string()));
    assert_eq!(
        body["timestamp"],
        serde_json::json!("2023-11-14T22:13:20.250Z")
    );
    assert_eq!(body["properties"]["actor_id"], serde_json::json!(583_231));
}

#[tokio::test]
async fn several_events_use_the_batch_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/batch/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": 1})))
        .expect(1)
        .mount(&server)
        .await;

    let batch = vec![captured(human_event()), captured(anonymous_event())];
    client_for(&server, &[])
        .capture(&batch)
        .await
        .expect("batch accepted");

    let requests = server.received_requests().await.expect("recorded");
    let body = body_of(&requests[0]);
    assert_eq!(body["api_key"], serde_json::json!("phc_test_token"));
    let items = body["batch"].as_array().expect("batch array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["distinct_id"], serde_json::json!("github:583231"));
    assert_eq!(items[1]["distinct_id"], serde_json::json!("fkst:anonymous"));
    // Every item carries its stable uuid so a retry deduplicates server-side.
    assert!(items.iter().all(|item| item["uuid"].is_string()));
}

#[tokio::test]
async fn an_empty_batch_makes_no_request() {
    let server = MockServer::start().await;
    client_for(&server, &[])
        .capture(&[])
        .await
        .expect("nothing to send is a success");
    assert!(server
        .received_requests()
        .await
        .expect("recorded")
        .is_empty());
}

#[tokio::test]
async fn a_slow_response_times_out_as_a_retryable_transport_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"status": 1}))
                .set_delay(Duration::from_millis(600)),
        )
        .mount(&server)
        .await;

    let error = client_for(&server, &[("FKST_POSTHOG_CAPTURE_TIMEOUT_MS", "60")])
        .capture(&[captured(human_event())])
        .await
        .expect_err("a slow capture must fail");
    assert!(matches!(error, CaptureError::Transport { .. }), "{error}");
    assert!(error.is_retryable());
}

#[tokio::test]
async fn retryable_statuses_are_classified_as_retryable() {
    for status in [408, 429, 500, 502, 503] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;

        let error = match client_for(&server, &[])
            .capture(&[captured(human_event())])
            .await
        {
            Err(error) => error,
            Ok(()) => panic!("status {status} must fail"),
        };
        assert!(
            matches!(error, CaptureError::RetryableStatus { status: got, .. } if got == status),
            "status {status}: {error}"
        );
        assert!(error.is_retryable(), "status {status}");
        assert_eq!(error.delivery_result(), DeliveryResult::Retryable);
    }
}

#[tokio::test]
async fn permanent_statuses_are_not_retried() {
    for status in [400, 401, 403, 404, 413] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;

        let error = match client_for(&server, &[])
            .capture(&[captured(human_event())])
            .await
        {
            Err(error) => error,
            Ok(()) => panic!("status {status} must fail"),
        };
        assert_eq!(
            error,
            CaptureError::PermanentStatus { status },
            "status {status}"
        );
        assert!(!error.is_retryable(), "status {status}");
        assert_eq!(error.delivery_result(), DeliveryResult::Permanent);
    }
}

#[tokio::test]
async fn a_numeric_retry_after_is_surfaced_to_the_caller() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .mount(&server)
        .await;

    let error = client_for(&server, &[])
        .capture(&[captured(human_event())])
        .await
        .expect_err("429 must fail");
    assert_eq!(
        error,
        CaptureError::RetryableStatus {
            status: 429,
            retry_after: Some(Duration::from_secs(7)),
        }
    );
    assert_eq!(error.retry_after(), Some(Duration::from_secs(7)));
}

#[tokio::test]
async fn a_non_numeric_retry_after_is_ignored_rather_than_parsed() {
    // The HTTP-date form is deliberately unsupported: the capped backoff is a
    // safe fallback, and a date parser on this path is needless attack surface.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT"),
        )
        .mount(&server)
        .await;

    let error = client_for(&server, &[])
        .capture(&[captured(human_event())])
        .await
        .expect_err("503 must fail");
    assert_eq!(error.retry_after(), None);
    assert!(error.is_retryable());
}

#[tokio::test]
async fn a_2xx_that_is_not_the_success_envelope_is_a_permanent_failure() {
    // A misrouted proxy answer or a rejected token can look like a 200; treating
    // it as success would silently lose the whole deployment's audit trail.
    for body in [
        serde_json::json!({"status": "Unauthorized"}),
        serde_json::json!({"error": "invalid api_key"}),
        serde_json::json!({}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(&server)
            .await;

        let error = client_for(&server, &[])
            .capture(&[captured(human_event())])
            .await
            .unwrap_err();
        assert_eq!(error, CaptureError::InvalidResponse, "{body}");
        assert!(!error.is_retryable());
    }
}

#[tokio::test]
async fn a_non_json_2xx_body_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>gateway</html>"))
        .mount(&server)
        .await;

    let error = client_for(&server, &[])
        .capture(&[captured(human_event())])
        .await
        .expect_err("html is not a capture response");
    assert_eq!(error, CaptureError::InvalidResponse);
}

#[tokio::test]
async fn the_legacy_ok_status_string_is_accepted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "Ok"})))
        .mount(&server)
        .await;

    client_for(&server, &[])
        .capture(&[captured(human_event())])
        .await
        .expect("older PostHog builds answer with a status string");
}

#[tokio::test]
async fn the_project_token_never_reaches_debug_output() {
    let server = MockServer::start().await;
    let client = client_for(&server, &[]);
    let debug = format!("{client:?}");
    assert!(!debug.contains("phc_test_token"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

/// A buffer holding everything `tracing` emitted on one test thread, so a test
/// can assert on the REAL log output rather than on the code that produced it.
#[derive(Clone, Default)]
struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

thread_local! {
    /// Where this thread's `tracing` output goes while a capture is active.
    static THREAD_SINK: std::cell::RefCell<Option<CapturedLogs>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes to whichever buffer the emitting THREAD registered, and discards the
/// rest.
struct ThreadRoutedWriter;

impl std::io::Write for ThreadRoutedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        THREAD_SINK.with(|sink| {
            if let Some(target) = sink.borrow().as_ref() {
                target
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stop recording when the capture goes out of scope.
struct LogCaptureGuard;

impl Drop for LogCaptureGuard {
    fn drop(&mut self) {
        THREAD_SINK.with(|sink| *sink.borrow_mut() = None);
    }
}

/// Start recording this thread's `tracing` output.
///
/// `tracing`'s dispatcher and callsite-interest state are process-global, so a
/// per-test thread-local subscriber races with the other tests emitting the same
/// callsites in parallel (the callsite's interest can be cached as "never"
/// before the test installs anything). Installing ONE global subscriber that
/// fans out to a thread-local buffer removes the race entirely: other threads'
/// events reach the writer and are discarded, and this thread's are recorded.
fn capture_logs() -> (CapturedLogs, LogCaptureGuard) {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(|| ThreadRoutedWriter)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        // A losing race with another initializer is harmless: either subscriber
        // routes through the same thread-local writer.
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    let logs = CapturedLogs::default();
    THREAD_SINK.with(|sink| *sink.borrow_mut() = Some(logs.clone()));
    (logs, LogCaptureGuard)
}

#[tokio::test]
async fn no_failure_log_carries_the_project_token_or_the_payload() {
    // The canary lives in the secret-typed field the transport actually sends;
    // every failure path must still log only bounded, credential-free detail.
    const CANARY: &str = "phc_canary_a1b2c3d4e5f6";

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string(CANARY))
        .mount(&server)
        .await;

    let (logs, _capture) = capture_logs();
    let client = client_for(&server, &[("FKST_POSTHOG_PROJECT_TOKEN", CANARY)]);
    let event = human_event();
    // `#[tokio::test]` runs a current-thread runtime, so the capture below stays
    // on the thread that registered the buffer.
    client
        .capture(&[captured(event.clone())])
        .await
        .expect_err("401 must fail");

    let text = logs.text();
    assert!(
        text.contains("audit capture rejected"),
        "the failure path must actually log: {text}"
    );
    assert!(
        !text.contains(CANARY),
        "a log leaked the project token: {text}"
    );
    // The response body may echo the input, so it is never logged either.
    assert!(
        !text.contains("octocat"),
        "a log leaked payload data: {text}"
    );
    // What IS logged is bounded: the numeric status and the event count.
    assert!(text.contains("401"), "{text}");
}
