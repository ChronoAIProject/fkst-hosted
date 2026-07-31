//! Installation-lifecycle event tests: payload parsing, the cache-bust dispatch
//! table over the [`CacheBust`] seam (a recording fake — no `AppState`, no
//! cluster), install-time seeding, and the Model B reconcile nudge.

use std::sync::{Arc, Mutex};

use super::*;
use crate::config::Config;
use crate::routes::github_app_webhook::test_support::{key, state_with_reconciler};

/// Records every cache-bust side effect so a test can assert exactly which
/// repos/owners were evicted + failed. `evict_repo` represents the in-memory
/// eviction AND the cross-worker broadcast (the production impl's
/// `GithubAppTokens::evict_repo` fans the eviction out), so its recorded
/// calls are the "broadcast hook invoked once per affected repo" assertion.
#[derive(Default)]
struct FakeCacheBust {
    evicted_repos: Mutex<Vec<String>>,
    evicted_owners: Mutex<Vec<String>>,
    failed_repos: Mutex<Vec<(String, String)>>,
    failed_owners: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl CacheBust for FakeCacheBust {
    async fn evict_repo(&self, owner: &str, name: &str) {
        self.evicted_repos
            .lock()
            .unwrap()
            .push(format!("{owner}/{name}"));
    }
    async fn evict_owner(&self, owner: &str) {
        self.evicted_owners.lock().unwrap().push(owner.to_string());
    }
    async fn fail_repo(&self, owner: &str, name: &str, reason: &str) {
        self.failed_repos
            .lock()
            .unwrap()
            .push((format!("{owner}/{name}"), reason.to_string()));
    }
    async fn fail_owner(&self, owner: &str, reason: &str) {
        self.failed_owners
            .lock()
            .unwrap()
            .push((owner.to_string(), reason.to_string()));
    }
}

// ---- payload parse --------------------------------------------------------

#[test]
fn installation_created_parses_selected_repos() {
    let body = br#"{
        "action": "created",
        "installation": {
            "id": 99,
            "account": { "login": "Acme", "type": "Organization" }
        },
        "sender": { "login": "installing-user", "id": 1234 },
        "repositories": [{ "full_name": "Acme/Site" }]
    }"#;
    let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
    assert_eq!(event.action, "created");
    assert_eq!(event.installation.id, 99);
    assert_eq!(event.installation.account.login, "Acme");
    assert_eq!(
        event.sender.as_ref().map(|sender| sender.login.as_str()),
        Some("installing-user")
    );
    let repos: Vec<String> = event
        .repositories
        .iter()
        .map(|r| canonical(&r.full_name))
        .collect();
    assert_eq!(repos, vec!["acme/site".to_string()]);
}

#[tokio::test]
async fn installation_created_all_selection_uses_owner_wide_eviction() {
    // An `all` install (no enumerated `repositories`) on a `deleted` event
    // selects the owner-wide eviction path, NOT a per-repo one.
    let body = br#"{
        "action": "deleted",
        "installation": {
            "id": 1,
            "account": { "login": "Octocat" }
        }
    }"#;
    let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
    let fake = FakeCacheBust::default();
    let handled = dispatch_installation(&fake, &event)
        .await
        .expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted");
    // No concrete repos => account-wide eviction by lowercased login.
    assert_eq!(*fake.evicted_owners.lock().unwrap(), vec!["octocat"]);
    assert!(
        fake.evicted_repos.lock().unwrap().is_empty(),
        "no per-repo eviction when nothing is enumerated"
    );
    assert_eq!(fake.failed_owners.lock().unwrap().len(), 1);
}

#[test]
fn installation_repositories_parses_added_removed() {
    let body = br#"{
        "action": "removed",
        "installation": { "id": 5, "account": { "login": "acme" } },
        "sender": { "login": "repo-manager" },
        "repositories_added": [],
        "repositories_removed": [{ "full_name": "acme/old" }]
    }"#;
    let event: InstallationReposEvent = serde_json::from_slice(body).expect("parse");
    assert_eq!(event.action, "removed");
    assert_eq!(
        event.sender.as_ref().map(|sender| sender.login.as_str()),
        Some("repo-manager")
    );
    assert_eq!(event.repositories_removed.len(), 1);
    assert_eq!(
        canonical(&event.repositories_removed[0].full_name),
        "acme/old"
    );
}

// ---- cache-bust dispatch (#141) ------------------------------------------

#[tokio::test]
async fn installation_deleted_evicts_and_fails_without_persistence() {
    // A `deleted` event that enumerates concrete repos evicts + fails each
    // of them; the broadcast hook (the fake's `evict_repo`) is invoked once
    // per affected repo. No Mongo is touched (there is none).
    let body = br#"{
        "action": "deleted",
        "installation": {
            "id": 7,
            "account": { "login": "Acme" }
        },
        "repositories": [
            { "full_name": "Acme/Site" },
            { "full_name": "Acme/Docs" }
        ]
    }"#;
    let event: InstallationEvent = serde_json::from_slice(body).expect("parse");
    let fake = FakeCacheBust::default();
    let handled = dispatch_installation(&fake, &event)
        .await
        .expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted");

    // evict_repo (= local eviction + cross-worker broadcast) once per repo.
    assert_eq!(
        *fake.evicted_repos.lock().unwrap(),
        vec!["acme/site".to_string(), "acme/docs".to_string()]
    );
    // fail_for_uninstalled_repo called per repo with the uninstall reason.
    let failed = fake.failed_repos.lock().unwrap();
    assert_eq!(failed.len(), 2);
    assert!(failed[0].1.starts_with(UNINSTALL_REASON_PREFIX));
    assert!(failed[0].1.contains("acme/site"));
    // Owner-wide path was NOT taken (concrete repos were enumerated).
    assert!(fake.evicted_owners.lock().unwrap().is_empty());
    assert!(fake.failed_owners.lock().unwrap().is_empty());
}

#[tokio::test]
async fn installation_repositories_removed_evicts_removed_only() {
    // Only `repositories_removed` is evicted + failed; `repositories_added`
    // is left alone (the next on-demand resolve picks it up).
    let body = br#"{
        "action": "removed",
        "installation": { "id": 5, "account": { "login": "acme" } },
        "repositories_added": [{ "full_name": "acme/fresh" }],
        "repositories_removed": [{ "full_name": "acme/old" }]
    }"#;
    let event: InstallationReposEvent = serde_json::from_slice(body).expect("parse");
    let fake = FakeCacheBust::default();
    let handled = dispatch_installation_repositories(&fake, &event)
        .await
        .expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted");

    assert_eq!(
        *fake.evicted_repos.lock().unwrap(),
        vec!["acme/old".to_string()],
        "only removed repos are evicted"
    );
    let failed = fake.failed_repos.lock().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "acme/old");
    // The added repo must NOT have been touched.
    assert!(!fake
        .evicted_repos
        .lock()
        .unwrap()
        .contains(&"acme/fresh".to_string()));
}

#[tokio::test]
async fn created_and_unsuspend_are_no_op_cache_busts() {
    // (re)install / unsuspend have nothing to bust: the next on-demand
    // resolve picks the coverage up. The handler never mints.
    for action in ["created", "unsuspend"] {
        let body = format!(
            r#"{{
                "action": "{action}",
                "installation": {{ "id": 3, "account": {{ "login": "acme" }} }},
                "repositories": [{{ "full_name": "acme/site" }}]
            }}"#
        );
        let event: InstallationEvent = serde_json::from_slice(body.as_bytes()).expect("parse");
        let fake = FakeCacheBust::default();
        let handled = dispatch_installation(&fake, &event)
            .await
            .expect("dispatch");
        assert_eq!(handled.as_str(), "ignored", "{action} must be a no-op");
        assert!(fake.evicted_repos.lock().unwrap().is_empty());
        assert!(fake.evicted_owners.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn malformed_installation_body_is_an_error_not_a_panic() {
    // The webhook maps a parse error to 202 (logged); the handler helper
    // surfaces it as `Err` and must not panic. The fake AppState path is
    // not exercised here — `handle_installation` builds the event itself —
    // so we drive the JSON parse boundary directly.
    let bad = br#"{ "action": "deleted", "installation": "not-an-object" }"#;
    let parsed: Result<InstallationEvent, _> = serde_json::from_slice(bad);
    assert!(parsed.is_err(), "malformed body must fail to parse");
}

#[derive(Clone)]
struct BufWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
    type Writer = BufWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn missing_sender_skips_seeding_warns_and_still_returns_success() {
    use crate::routes::canvas::test_support::{test_app, test_state};
    use wiremock::MockServer;

    let server = MockServer::start().await;
    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    assert!(state.config.reconcile.seed_trigger_issue_on_install);
    let body = br#"{
        "action": "created",
        "installation": { "id": 99, "account": { "login": "acme" } },
        "repositories": [{ "full_name": "acme/site" }]
    }"#;

    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufWriter(buf.clone()))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let handled = {
        let _guard = tracing::subscriber::set_default(subscriber);
        handle_installation(&state, body)
            .await
            .expect("2xx dispatch")
    };

    assert_eq!(handled.as_str(), "ignored");
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .is_empty(),
        "an unattributable installation must not make any seed API call"
    );
    let logs = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8 logs");
    assert!(
        logs.contains(
            "seed: no sender on installation event; skipping seeding (unattributable trigger)"
        ),
        "the skip must be visible to operators: {logs}"
    );
}

// ---- Model B reconcile nudge (PR6) ---------------------------------------

#[tokio::test]
async fn installation_event_cache_busts_and_nudges_each_enumerated_repo() {
    // A `deleted` event that names concrete repos still busts caches AND now
    // enqueues each repo so the reconciler tears its session down.
    let body = br#"{
        "action": "deleted",
        "installation": { "id": 42, "account": { "login": "acme" } },
        "repositories": [{ "full_name": "acme/site" }, { "full_name": "acme/docs" }]
    }"#;
    let (state, mut rx) = state_with_reconciler();
    let handled = handle_installation(&state, body).await.expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted", "cache-bust is preserved");

    let mut got = vec![
        rx.try_recv().expect("first"),
        rx.try_recv().expect("second"),
    ];
    got.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    assert_eq!(got, vec![key(42, "acme", "docs"), key(42, "acme", "site")]);
    assert!(
        rx.try_recv().is_err(),
        "exactly the two named repos enqueued"
    );
}

#[tokio::test]
async fn installation_repositories_event_nudges_added_and_removed() {
    let body = br#"{
        "action": "added",
        "installation": { "id": 7, "account": { "login": "acme" } },
        "repositories_added": [{ "full_name": "acme/fresh" }],
        "repositories_removed": [{ "full_name": "acme/old" }]
    }"#;
    let (state, mut rx) = state_with_reconciler();
    let handled = handle_installation_repositories(&state, body)
        .await
        .expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted");

    let mut got = vec![
        rx.try_recv().expect("first"),
        rx.try_recv().expect("second"),
    ];
    got.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    // Both the added AND the removed repo are nudged (order: added then removed
    // by the handler, sorted here for a stable assertion).
    assert_eq!(got, vec![key(7, "acme", "fresh"), key(7, "acme", "old")]);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn installation_event_without_a_reconciler_still_cache_busts() {
    // The enqueue is additive + guarded on `Some(reconciler)`: with `None` the
    // handler is a pure cache-bust (the existing behaviour), no panic.
    let body = br#"{
        "action": "deleted",
        "installation": { "id": 1, "account": { "login": "acme" } },
        "repositories": [{ "full_name": "acme/site" }]
    }"#;
    let state = AppState {
        config: Config::default(),
        recovery: Default::default(),
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        session_access: Default::default(),
        log_bundle_cache: Default::default(),
        disposable_environments: Default::default(),
        self_router: crate::state::empty_self_router(),
        chat: None,
        audit: Default::default(),
    };
    let handled = handle_installation(&state, body).await.expect("dispatch");
    assert_eq!(handled.as_str(), "cache_busted");
}
