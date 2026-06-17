//! Skip-set bootstrap + rolling activity-comment tests for the [`Journaler`]
//! (split out of `flush.rs` to keep every `journal/*.rs` under 500 lines,
//! #139). Declared from `flush.rs` via `#[path]`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::{default_identity_pointers, JournalConfig};
use crate::keys::idem_key;
use crate::model::{CompletedEntry, ProgressRecord};
use crate::test_support::{contents_body, ctx, github_cfg, raised};
use crate::{FlushOutcome, Journaler};

fn pointers() -> Vec<String> {
    default_identity_pointers()
}

// ---- journaler: skip-set bootstrap ---------------------------------------------------

#[tokio::test]
async fn skip_set_rebuilt_from_committed_completed() {
    let server = MockServer::start().await;
    let event1 = json!({"department":"d","source":"raiser","name":"e1","corr":"c-1"});
    let event2 = json!({"department":"d","source":"raiser","name":"e2","corr":"c-1"});
    let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    remote.completed = vec![
        CompletedEntry {
            idem_key: idem_key("demo", &event1, &pointers()),
            event: event1.clone(),
            at: "t1".to_string(),
        },
        CompletedEntry {
            idem_key: idem_key("demo", &event2, &pointers()),
            event: event2.clone(),
            at: "t2".to_string(),
        },
    ];
    remote.max_fencing_token = 1;
    remote.issue_number = Some(55);
    remote.last_comment_id = Some(88);
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s9")))
        .mount(&server)
        .await;

    // The redo writer carries a HIGHER fencing token (a fresh lease).
    let mut journaler = Journaler::start(ctx(2), github_cfg(&server.uri()))
        .await
        .expect("start");
    let skip = journaler.load_skip_set().await.expect("bootstrap");
    assert_eq!(skip.len(), 2);
    assert!(skip.contains(&idem_key("demo", &event1, &pointers())));
    // The run-head pointers hydrated from the committed file.
    assert_eq!(journaler.issue_number, Some(55));
    assert_eq!(journaler.last_comment_id, Some(88));
    assert_eq!(journaler.known_max_token, 1);

    // Re-emitting the bootstrapped events is a no-op (skip-set hit).
    journaler
        .record(raised("d", "e1"))
        .await
        .expect("re-emit 1");
    journaler
        .record(raised("d", "e2"))
        .await
        .expect("re-emit 2");
    assert_eq!(journaler.buffered(), 0, "nothing newly completed");
}

#[tokio::test]
async fn bootstrap_retries_on_404_then_empty() {
    let server = MockServer::start().await;
    // Every GET is a 404: the bootstrap loop retries then concludes fresh.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .expect(3) // bootstrap_read_retries default = 3 reads
        .mount(&server)
        .await;

    let cfg = JournalConfig {
        bootstrap_read_retries: 3,
        ..github_cfg(&server.uri())
    };
    let mut journaler = Journaler::start(ctx(1), cfg).await.expect("start");
    let skip = journaler
        .load_skip_set()
        .await
        .expect("fail-open after retries");
    assert!(skip.is_empty());
}

#[tokio::test]
async fn unreachable_github_at_bootstrap_fails_open() {
    let cfg = JournalConfig {
        github_api_base: "http://127.0.0.1:1".to_string(),
        ..github_cfg("http://127.0.0.1:1")
    };
    let mut journaler = Journaler::start(ctx(1), cfg).await.expect("start");
    let skip = journaler.load_skip_set().await.expect("fail-open");
    assert!(skip.is_empty());
    // The session still proceeds: recording works (buffered in RAM).
    journaler.record(raised("d", "e1")).await.expect("record");
    assert_eq!(journaler.buffered(), 1);
}

#[tokio::test]
async fn corrupt_and_newer_schema_remotes_yield_safe_empty_skip_sets() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": STANDARD.encode(b"not json"), "sha": "c1"
        })))
        .mount(&server)
        .await;
    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    assert!(journaler.load_skip_set().await.expect("corrupt").is_empty());

    let server2 = MockServer::start().await;
    let mut newer = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    newer.schema = "fkst-hosted/progress-record@9".to_string();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&newer, "n1")))
        .mount(&server2)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server2)
        .await;
    let mut journaler2 = Journaler::start(ctx(1), github_cfg(&server2.uri()))
        .await
        .expect("start");
    assert!(journaler2.load_skip_set().await.expect("newer").is_empty());
    // Forward-compat guard: it must now refuse to write (github_disabled).
    journaler2.record(raised("d", "e1")).await.expect("record");
    let outcome = journaler2.flush(true).await.expect("disabled");
    assert_eq!(outcome, FlushOutcome::skipped());
}

// ---- journaler: activity comment -----------------------------------------------------

#[tokio::test]
async fn flush_upserts_one_activity_comment_per_flush() {
    let server = MockServer::start().await;
    // Seed a record carrying an issue_number so the activity comment fires.
    let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    remote.issue_number = Some(12);
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&remote, "s1")))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "s2" } })),
        )
        .mount(&server)
        .await;
    // EXACTLY ONE comment POST per committed flush (first flush => create).
    Mock::given(method("POST"))
        .and(path("/repos/owner/name/issues/12/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 77})))
        .expect(1)
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    // Bootstrap hydrates issue_number from the committed file.
    journaler.load_skip_set().await.expect("bootstrap");
    assert_eq!(journaler.issue_number, Some(12));

    // ZERO comment calls during record.
    journaler.record(raised("d", "e1")).await.expect("record");

    let outcome = journaler.flush(true).await.expect("flush");
    assert_eq!(outcome.committed, 1);
    // The new comment id was folded into the journaler for the next flush.
    assert_eq!(journaler.last_comment_id, Some(77));
    // wiremock's `.expect(1)` on POST verifies exactly-one on drop.
}

#[tokio::test]
async fn flush_skips_activity_comment_without_issue_number() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "s1" } })),
        )
        .mount(&server)
        .await;
    // No issue_number anywhere: NO comment call may happen.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .expect(0)
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(ctx(1), github_cfg(&server.uri()))
        .await
        .expect("start");
    journaler.record(raised("d", "e1")).await.expect("record");
    let outcome = journaler.flush(true).await.expect("flush");
    assert_eq!(outcome.committed, 1);
    assert_eq!(journaler.last_comment_id, None);
}
