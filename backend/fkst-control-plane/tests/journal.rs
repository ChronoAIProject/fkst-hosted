//! Journaling integration tests against a mocked GitHub Contents API
//! (wiremock). Since the committed GitHub journal file is the SOLE
//! machine-truth (#139), there is no datastore to stand up — the journaler is
//! store-free, so these tests need no Mongo container and always run.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use fkst_control_plane::journal::model::{CompletedEntry, ProgressRecord, UNVERIFIED_SHA};
use fkst_control_plane::journal::{
    default_identity_pointers, idem_key, JournalConfig, Journaler, LifecycleEvent, ProgressSignal,
    SessionCtx, Transition,
};
use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx(session_id: &str, token: i64, fingerprint: &str) -> SessionCtx {
    SessionCtx {
        session_id: session_id.to_string(),
        package_name: "demo".to_string(),
        package_fingerprint: fingerprint.to_string(),
        pod_id: Some("pod-a".to_string()),
        fencing_token: token,
    }
}

fn github_cfg(server_uri: &str) -> JournalConfig {
    JournalConfig {
        github_repo: Some("owner/name".to_string()),
        github_api_base: server_uri.to_string(),
        github_token: Some(SecretString::from("test-token".to_string())),
        ..JournalConfig::default()
    }
}

/// Contents-API GET body for a record.
fn contents_body(record: &ProgressRecord, sha: &str) -> serde_json::Value {
    json!({
        "content": STANDARD.encode(serde_json::to_vec(record).expect("json")),
        "sha": sha,
        "encoding": "base64"
    })
}

// ---------------------------------------------------------------------------
// Local idempotency: the in-RAM skip-set replaces the Mongo unique index
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_idem_key_is_a_benign_no_op_within_a_session() {
    let mut journaler = Journaler::start(
        ctx("11111111-1111-4111-8111-111111111111", 1, "fp"),
        JournalConfig {
            github_enabled: false,
            ..JournalConfig::default()
        },
    )
    .await
    .expect("start");

    let event = json!({"department":"d","source":"s","name":"e1","corr":"c"});
    journaler
        .record(ProgressSignal::Raised {
            event_json: event.clone(),
        })
        .await
        .expect("first record");
    // The in-RAM skip-set dedupes; the journaler answers Ok with no new buffer.
    journaler
        .record(ProgressSignal::Raised { event_json: event })
        .await
        .expect("duplicate must be a benign no-op");
    assert_eq!(
        journaler.buffered(),
        1,
        "duplicate adds no second completion"
    );

    // Lifecycle signals are NOT idempotency-constrained: both buffer.
    for _ in 0..2 {
        journaler
            .record(ProgressSignal::Lifecycle(LifecycleEvent::now(
                Transition::Running,
            )))
            .await
            .expect("lifecycle records");
    }
    // buffered() counts only completions, still 1.
    assert_eq!(journaler.buffered(), 1);
}

// ---------------------------------------------------------------------------
// Redo contract against the committed GitHub file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redo_rebuilds_the_skip_set_and_reemission_buffers_nothing() {
    let pointers = default_identity_pointers();
    let events: Vec<serde_json::Value> = (0..3)
        .map(|i| json!({"department":"d","source":"s","name":format!("e{i}"),"corr":"c"}))
        .collect();

    // "Session A on another pod" already committed this record to GitHub.
    let mut remote = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
    remote.completed = events
        .iter()
        .map(|event| CompletedEntry {
            idem_key: idem_key("demo", event, &pointers),
            event: event.clone(),
            at: "2026-06-10T00:00:00Z".to_string(),
        })
        .collect();
    remote.max_fencing_token = 1;
    remote.issue_number = Some(99);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(contents_body(&remote, "sha-remote")),
        )
        .mount(&server)
        .await;

    // Session B: the redo on this pod, holding a HIGHER fencing token.
    let mut journaler = Journaler::start(
        ctx("22222222-2222-4222-8222-222222222222", 2, "fp"),
        github_cfg(&server.uri()),
    )
    .await
    .expect("start");
    let skip = journaler.load_skip_set().await.expect("bootstrap");
    assert_eq!(skip.len(), 3);
    for event in &events {
        assert!(skip.contains(&idem_key("demo", event, &pointers)));
    }

    // Re-emitting every bootstrapped event buffers ZERO new completions.
    for event in events {
        journaler
            .record(ProgressSignal::Raised { event_json: event })
            .await
            .expect("re-emit");
    }
    assert_eq!(journaler.buffered(), 0, "idempotent redo");
}

#[tokio::test]
async fn unreachable_github_at_bootstrap_fails_open() {
    let mut journaler = Journaler::start(
        ctx("33333333-3333-4333-8333-333333333333", 1, "fp"),
        github_cfg("http://127.0.0.1:1"),
    )
    .await
    .expect("start");

    let skip = journaler.load_skip_set().await.expect("fail-open");
    assert!(skip.is_empty(), "unreachable github => EMPTY skip-set");

    // Recording still works (buffered in RAM); the session proceeds.
    journaler
        .record(ProgressSignal::Raised {
            event_json: json!({"department":"d","source":"s","name":"e","corr":"c"}),
        })
        .await
        .expect("record");
    assert_eq!(journaler.buffered(), 1);
}

#[tokio::test]
async fn flush_commits_the_record_and_clears_the_buffer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/repos/owner/name/contents/.fkst-hosted/journal/"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "committed" } })),
        )
        .mount(&server)
        .await;
    // Catch-all PUT (the journal path embeds the run_key hash).
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({ "content": { "sha": "committed" } })),
        )
        .mount(&server)
        .await;

    let mut journaler = Journaler::start(
        ctx("44444444-4444-4444-8444-444444444444", 1, "fp"),
        github_cfg(&server.uri()),
    )
    .await
    .expect("start");
    journaler
        .record(ProgressSignal::Raised {
            event_json: json!({"department":"d","source":"s","name":"e","corr":"c"}),
        })
        .await
        .expect("record");
    let outcome = journaler.flush(true).await.expect("flush");
    assert_eq!(outcome.committed, 1);
    assert_eq!(outcome.commit_sha.as_deref(), Some("committed"));
    assert_eq!(journaler.buffered(), 0);
    // The in-file sentinel is the canonical pre-verified sha.
    assert_eq!(UNVERIFIED_SHA, "unverified");
}
