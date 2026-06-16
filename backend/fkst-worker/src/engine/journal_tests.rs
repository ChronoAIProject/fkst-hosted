//! Suite for the worker's journaling glue (issue #151, increment 6c). Proves the
//! WIRING — the gating of [`start_session_journaler`] (a plan yields a journaler;
//! `None` / an empty package set yields `None`) and a full lifecycle + RAISED
//! round-trip against a wiremock GitHub (GET contents 404 → fresh, PUT contents
//! 200 → committed). The journaler's own flush/CAS mechanics are covered by the
//! `fkst-journal` crate's tests; this file proves the worker drives them. No
//! secret value is ever asserted or printed.

use std::collections::BTreeMap;

use base64::Engine as _;
use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use fkst_journal::Transition;
use fkst_shared::models::RepoRef;
use fkst_shared::protocol::{CloneSpec, DispatchGoal, JournalPlan, ResolvedDispatch};

use super::*;

/// A `JournalPlan` pointing its GitHub API base at `api_base`, with a journal
/// repo + token. Field values mirror the process-level journal config the
/// controller ships (a small batch + a tight interval so a `force=true`
/// lifecycle flush commits immediately in the round-trip test).
fn journal_plan(api_base: &str) -> JournalPlan {
    JournalPlan {
        flush_interval_ms: 50,
        flush_max_batch: 1,
        issue_comments: false,
        activity_comment_enabled: false,
        cas_max_retries: 3,
        bootstrap_read_retries: 1,
        github_branch: "main".into(),
        github_repo: "acme/journal".into(),
        github_api_base: api_base.to_string(),
        identity_pointers: vec!["/type".into(), "/id".into()],
        max_line_bytes: 1_048_576,
        github_token: SecretString::from("ghp_journal_token".to_string()),
    }
}

/// A dispatch carrying the given (optional) journal plan. Everything else is the
/// minimal valid shape (a single package root so the journaler has a name).
fn dispatch_with_journal(plan: Option<JournalPlan>) -> ResolvedDispatch {
    ResolvedDispatch {
        session_id: "11111111-1111-1111-1111-111111111111".into(),
        worker_id: "w1".into(),
        fencing_id: 7,
        goal: DispatchGoal {
            goal_id: "22222222-2222-2222-2222-222222222222".into(),
            title: "Build it".into(),
            description: SecretString::from("SECRET-PROMPT".to_string()),
            repo: RepoRef {
                owner: "acme".into(),
                name: "site".into(),
            },
        },
        clone_spec: CloneSpec {
            repo: RepoRef {
                owner: "acme".into(),
                name: "site".into(),
            },
            git_ref: "main".into(),
            package_roots: vec!["demo".into()],
        },
        github_token: SecretString::from("ghs_test_token".to_string()),
        github_token_expires_at_unix_ms: 1_700_000_000_000,
        env_profile: BTreeMap::new(),
        codex_config_toml: None,
        ornn: None,
        journal: plan,
        mint_nonce: SecretString::from("controller-nonce-abc".to_string()),
    }
}

/// Mount the GitHub contents endpoints a flush touches: every GET is a 404 (the
/// journal file does not exist yet → a fresh create), every PUT is a 200 with a
/// commit sha. Returns nothing; the server records the requests.
async fn mount_github_contents(server: &MockServer) {
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "content": { "sha": "sha-1" } })),
        )
        .mount(server)
        .await;
}

/// Count the PUT (commit) requests the server received.
async fn put_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::PUT)
        .count()
}

/// A dispatch WITH a plan (pointed at a wiremock GitHub) yields `Some(journaler)`.
#[tokio::test]
async fn start_session_journaler_with_a_plan_yields_some() {
    let server = MockServer::start().await;
    // The bootstrap `load_skip_set` GETs the file; 404 => a fresh run.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let dispatch = dispatch_with_journal(Some(journal_plan(&server.uri())));
    let journaler = start_session_journaler(&dispatch, None).await;
    assert!(
        journaler.is_some(),
        "a configured plan must yield a journaler"
    );
}

/// A dispatch with `journal: None` yields `None` (journaling off).
#[tokio::test]
async fn start_session_journaler_without_a_plan_yields_none() {
    let dispatch = dispatch_with_journal(None);
    let journaler = start_session_journaler(&dispatch, None).await;
    assert!(
        journaler.is_none(),
        "no plan must mean no journaler (journaling off)"
    );
}

/// A dispatch whose `clone_spec.package_roots` is EMPTY yields `None` — the
/// journaler rejects an empty package name, so the worker must skip it.
#[tokio::test]
async fn start_session_journaler_with_no_package_roots_yields_none() {
    let mut dispatch = dispatch_with_journal(Some(journal_plan("http://127.0.0.1:1")));
    dispatch.clone_spec.package_roots.clear();
    let journaler = start_session_journaler(&dispatch, None).await;
    assert!(
        journaler.is_none(),
        "an empty package set has no valid journaler name; must skip"
    );
}

/// Full round-trip: start the journaler, journal Validating, feed a real
/// `RAISED:<b64-json>` line, then finish Stopped — and assert at least one PUT
/// committed the run to GitHub (the lifecycle force-flushes immediately).
#[tokio::test]
async fn journals_lifecycle_and_raised_line_to_github() {
    let server = MockServer::start().await;
    mount_github_contents(&server).await;

    let dispatch = dispatch_with_journal(Some(journal_plan(&server.uri())));
    let mut journaler = start_session_journaler(&dispatch, None).await;
    assert!(journaler.is_some(), "the plan must yield a journaler");

    // Validating force-flushes immediately => at least one PUT lands.
    journal_lifecycle(&mut journaler, Transition::Validating).await;

    // A real RAISED line (base64-encoded JSON event).
    let event = json!({ "type": "tick", "id": "e1" });
    let b64 = base64::engine::general_purpose::STANDARD.encode(event.to_string());
    let line = format!("RAISED:{b64}");
    journal_stdout_line(&mut journaler, line.as_bytes()).await;

    // Terminal: finish Stopped (a final forced flush).
    journal_finish(&mut journaler, Transition::Stopped { exit_code: Some(0) }).await;

    assert!(
        put_count(&server).await >= 1,
        "the completed run must have been committed to GitHub at least once"
    );
}

/// A MALFORMED RAISED line (invalid base64 payload) bumps `malformed_raised_total`
/// and is journaled as an anomaly rather than dropped — the wiring records it.
#[tokio::test]
async fn malformed_raised_line_bumps_the_counter() {
    let server = MockServer::start().await;
    mount_github_contents(&server).await;

    let dispatch = dispatch_with_journal(Some(journal_plan(&server.uri())));
    let mut journaler = start_session_journaler(&dispatch, None).await;
    assert!(journaler.is_some(), "the plan must yield a journaler");
    assert_eq!(
        journaler.as_ref().unwrap().malformed_raised_total,
        0,
        "counter starts at zero"
    );

    // `@@@` is not valid base64 in any accepted alphabet => Malformed.
    journal_stdout_line(&mut journaler, b"RAISED:@@@").await;

    assert_eq!(
        journaler.as_ref().unwrap().malformed_raised_total,
        1,
        "a malformed RAISED line must bump the counter"
    );
}
