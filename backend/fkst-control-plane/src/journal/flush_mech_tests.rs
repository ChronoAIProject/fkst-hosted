//! Flush-mechanism + fencing + secret-hygiene tests for the [`Journaler`]
//! (split out of `flush.rs` to keep every `journal/*.rs` under 500 lines,
//! #139). Declared from `flush.rs` via `#[path]`.

use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::journal::config::JournalConfig;
use crate::journal::model::ProgressRecord;
use crate::journal::test_support::{contents_body, ctx, github_cfg, mongo_only_cfg, raised};
use crate::journal::{FlushOutcome, JournalError, Journaler};

// ---- journaler: flush --------------------------------------------------------------

#[tokio::test]
async fn flush_is_debounced_and_force_creates_the_remote_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-1" } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");

    // Below the batch size and inside the interval: deferred.
    let deferred = journaler.flush(false).await.expect("deferred flush");
    assert_eq!(deferred, FlushOutcome::skipped());
    assert_eq!(journaler.buffered(), 1, "buffer retained");

    let outcome = journaler.flush(true).await.expect("forced flush");
    assert_eq!(outcome.committed, 1);
    assert_eq!(outcome.commit_sha.as_deref(), Some("sha-1"));
    assert!(!outcome.fenced);
    assert_eq!(journaler.buffered(), 0);
}

#[tokio::test]
async fn flush_merges_with_the_remote_record_and_sends_the_prior_sha() {
    let server = MockServer::start().await;
    let mut remote = ProgressRecord::new("ignored", "demo", "fp", "t0".to_string());
    remote.completed = vec![crate::journal::test_support::completed(
        "remote-key",
        "2026-06-09T00:00:00Z",
    )];
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "prev")))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(body_partial_json(json!({ "sha": "prev" })))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "next" } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let outcome = journaler.flush(true).await.expect("flush");
    assert_eq!(outcome.commit_sha.as_deref(), Some("next"));
    // The merge unioned remote + ours (1 + 1).
    assert_eq!(outcome.committed, 1);
}

#[tokio::test]
async fn flush_retries_on_cas_conflict_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    // First PUT loses the race; the re-read + second PUT wins.
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(409))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-2" } })),
        )
        .with_priority(5)
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let outcome = journaler.flush(true).await.expect("flush must converge");
    assert_eq!(outcome.commit_sha.as_deref(), Some("sha-2"));
}

#[tokio::test]
async fn flush_exhausts_cas_retries_keeps_the_buffer_and_recovers_next_tick() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let err = journaler.flush(true).await.expect_err("must exhaust");
    assert!(matches!(err, JournalError::CasExhausted(_)), "got {err:?}");
    assert_eq!(journaler.buffered(), 1, "buffer retained for the next tick");

    // The conflict clears (e.g. the competing writer finished): the next
    // forced flush commits the retained buffer.
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "sha-3" } })),
        )
        .mount(&server)
        .await;
    let outcome = journaler.flush(true).await.expect("retry tick");
    assert_eq!(outcome.committed, 1);
}

#[tokio::test]
async fn auth_failure_disables_github_for_the_session() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let err = journaler.flush(true).await.expect_err("auth must fail");
    assert!(matches!(err, JournalError::GithubAuth), "got {err:?}");

    // GitHub now disabled: a subsequent flush makes NO further GitHub calls
    // and drops the buffer (no durable floor) with a warn.
    server.reset().await;
    journaler.record(raised("d", "e2")).await.expect("record 2");
    let outcome = journaler
        .flush(true)
        .await
        .expect("disabled flush must succeed");
    assert_eq!(outcome, FlushOutcome::skipped());
    assert_eq!(journaler.buffered(), 0, "buffers dropped when disabled");
}

#[tokio::test]
async fn flush_with_no_github_drops_buffer_and_warns() {
    // github_enabled=false => no client => the no-floor path drops the buffer
    // (the work is re-derivable on redo).
    let mut journaler = Journaler::start(ctx(1), mongo_only_cfg())
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    assert_eq!(journaler.buffered(), 1);
    let outcome = journaler.flush(true).await.expect("no-floor flush");
    assert_eq!(outcome, FlushOutcome::skipped());
    assert_eq!(journaler.buffered(), 0, "buffer dropped (no durable floor)");
}

// ---- journaler: fencing ----------------------------------------------------------

#[tokio::test]
async fn stale_writer_is_fenced_off_and_never_puts() {
    let server = MockServer::start().await;
    let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    remote.max_fencing_token = 5;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s1")))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0) // the load-bearing assertion: NO write happens
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(3), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let outcome = journaler.flush(true).await.expect("fenced is not an error");
    assert!(outcome.fenced);
    assert_eq!(outcome.committed, 0);
    assert!(outcome.commit_sha.is_none());
}

#[tokio::test]
async fn equal_token_proceeds_and_greater_token_bumps_known() {
    for (token, expected_max) in [(5i64, 5i64), (7, 7)] {
        let server = MockServer::start().await;
        let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        remote.max_fencing_token = 5;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s1")))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(body_partial_json(json!({ "sha": "s1" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "s2" } })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut journaler = Journaler::start(ctx(token), github_cfg(&server.uri()))
            .await
            .expect("start");
        journaler.record(raised("d", "e1")).await.expect("record");
        let outcome = journaler.flush(true).await.expect("must proceed");
        assert!(!outcome.fenced, "token {token} must not be fenced");
        assert_eq!(outcome.committed, 1);
        assert_eq!(journaler.known_max_token, expected_max);
    }
}

// ---- secret hygiene: tracing capture --------------------------------------------------------

/// A shared, in-memory sink that captures every byte a tracing subscriber
/// writes. Hand-rolled so this stays dependency-free (no `tracing-test`).
#[derive(Clone, Default)]
struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

impl CaptureBuffer {
    fn contents(&self) -> Vec<u8> {
        self.0.lock().expect("capture lock poisoned").clone()
    }
}

impl std::io::Write for CaptureBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture lock poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureBuffer {
    type Writer = CaptureBuffer;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Drive a flush against one GitHub error path with a canary token; the caller
/// wraps this in a capturing subscriber. `status = None` forces a raw reqwest
/// network error (a closed port).
async fn flush_canary(status: Option<u16>, token: &str) {
    let api_base = match status {
        Some(code) => {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;
            let put = if code == 403 {
                // 403 WITH rate-limit headers => GithubRateLimited.
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "30")
            } else {
                ResponseTemplate::new(code)
            };
            Mock::given(method("PUT"))
                .respond_with(put)
                .mount(&server)
                .await;
            let uri = server.uri();
            // Keep the mock alive across the flush; the test process is
            // short-lived so leaking it is harmless and avoids a borrow.
            std::mem::forget(server);
            uri
        }
        None => "http://127.0.0.1:1".to_string(),
    };
    let cfg = JournalConfig {
        github_repo: Some("owner/name".to_string()),
        github_api_base: api_base,
        github_token: Some(SecretString::from(token.to_string())),
        cas_max_retries: 2,
        ..JournalConfig::default()
    };
    let mut journaler = Journaler::start(ctx(1), cfg).await.expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    // Every arm is an error/fenced path; we only care that whatever it logs is
    // token-free, so swallow the outcome.
    let _ = journaler.flush(true).await;
}

/// Spec §Testing: "tracing capture for a flush never contains the token".
/// Capture ALL tracing output at TRACE level while flushes traverse every
/// GitHub error path (auth 401, rate-limit 403, a transient 500 with retries,
/// and a raw network failure) with a canary token installed, and assert the
/// captured bytes contain neither the token value nor the
/// `Bearer`/`Authorization` markers an accidental header-or-token
/// interpolation would leak. Deterministic: wiremock + a closed port, no real
/// network.
#[test]
fn no_error_variant_or_debug_ever_contains_the_token() {
    const CANARY: &str = "ghp_tracing_canary_value";

    let capture = CaptureBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            flush_canary(Some(401), CANARY).await; // auth
            flush_canary(Some(403), CANARY).await; // rate limit
            flush_canary(Some(500), CANARY).await; // transient 5xx + retries
            flush_canary(None, CANARY).await; // raw network error
        });
    });

    let bytes = capture.contents();
    assert!(
        !bytes.is_empty(),
        "the flush error paths must have logged SOMETHING"
    );
    let text = String::from_utf8_lossy(&bytes);
    for needle in [CANARY, "Bearer", "Authorization", "authorization"] {
        assert!(
            !text.contains(needle),
            "tracing output leaked {needle:?}:\n{text}"
        );
    }
}
