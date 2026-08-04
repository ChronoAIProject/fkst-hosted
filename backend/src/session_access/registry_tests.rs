//! Unit tests for the session-access projection: per-repository replacement,
//! atomic generation publication, readiness, and bounded diagnostics.

use std::sync::Arc;
use std::thread;

use super::*;
use crate::session_access::test_support::{context_in, repo};

/// One `(session_id, context)` pair for installation 1 / `acme/<repo_name>`.
fn entry(session: &str, repo_name: &str, creator_id: i64) -> (String, SessionAccessContext) {
    (
        session.to_string(),
        context_in(
            1,
            repo_name,
            Some(creator_id),
            "alice",
            &["bob"],
            &["carol"],
        ),
    )
}

#[test]
fn a_dispatch_disabled_deployment_is_authoritatively_empty() {
    let registry = SessionAccessRegistry::new(false);
    assert!(registry.is_ready());
    assert_eq!(registry.lookup("nope"), ContextLookup::Unknown);
}

#[test]
fn a_cold_registry_is_unavailable_not_empty() {
    let registry = SessionAccessRegistry::new(true);
    assert!(!registry.is_ready());
    assert_eq!(
        registry.lookup("sess-1"),
        ContextLookup::Unavailable,
        "cold state must never look like a confident empty answer"
    );
    assert_eq!(registry.snapshot().state, RegistryState::Cold);
}

#[test]
fn default_fails_closed() {
    assert_eq!(
        SessionAccessRegistry::default().snapshot().state,
        RegistryState::Cold
    );
}

#[test]
fn a_staged_generation_is_invisible_until_the_last_repo_lands() {
    let registry = SessionAccessRegistry::new(true);
    let expected: HashSet<RepoKey> = [(1, repo("site")), (1, repo("web"))].into_iter().collect();
    registry.begin_generation(expected);
    assert_eq!(registry.snapshot().state, RegistryState::Recovering);

    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert_eq!(
        registry.lookup("sess-site"),
        ContextLookup::Unavailable,
        "a half-built generation must not be observable"
    );
    assert!(registry.is_empty(), "nothing is published yet");
    assert_eq!(registry.snapshot().pending_repositories, 1);

    registry.replace_repo(1, &repo("web"), vec![entry("sess-web", "web", 43)]);
    assert!(registry.is_ready());
    assert_eq!(registry.len(), 2);
    assert!(matches!(
        registry.lookup("sess-site"),
        ContextLookup::Found(_)
    ));
    assert_eq!(registry.snapshot().pending_repositories, 0);
}

#[test]
fn an_empty_enumeration_publishes_an_authoritative_empty_generation() {
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation(HashSet::new());
    assert!(registry.is_ready());
    assert_eq!(registry.lookup("sess-1"), ContextLookup::Unknown);
}

#[test]
fn abandoning_a_cold_generation_stays_fail_closed_then_recovers() {
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation([(1, repo("site"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.is_ready());

    // A later discovery attempt that cannot complete must not publish and must
    // not downgrade the already-complete generation.
    let cold = SessionAccessRegistry::new(true);
    cold.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    cold.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    cold.abandon_generation();
    assert_eq!(cold.snapshot().state, RegistryState::Cold);
    assert_eq!(cold.lookup("sess-site"), ContextLookup::Unavailable);

    // The next complete generation recovers it.
    cold.begin_generation([(1, repo("site"))].into_iter().collect());
    cold.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(cold.is_ready());
}

#[test]
fn abandoning_a_refresh_keeps_the_published_generation_ready() {
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation([(1, repo("site"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.is_ready());

    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    assert!(
        registry.is_ready(),
        "a refresh must not flap a healthy deployment into 503"
    );
    registry.abandon_generation();
    assert!(registry.is_ready());
    assert!(matches!(
        registry.lookup("sess-site"),
        ContextLookup::Found(_)
    ));
}

#[test]
fn a_repo_that_never_reports_does_not_wedge_the_live_projection() {
    // The wedge this guards: `web` fails every pass (Issues disabled), so the
    // generation's pending set never empties. Without degradation every later
    // `replace_repo` would be swallowed by the staging buffer and the live map —
    // the one the shipped log/observe routes read — would freeze forever.
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.get("sess-site").is_none(), "still staged");

    registry.record_repo_failure(1, &repo("web"));
    assert_eq!(
        registry.snapshot().pending_repositories,
        0,
        "the doomed generation is no longer holding writes"
    );
    assert!(
        registry.get("sess-site").is_some(),
        "the collected set is folded into the live map, not thrown away"
    );
    assert_eq!(
        registry.snapshot().state,
        RegistryState::Cold,
        "a projection that was never complete keeps failing closed"
    );
    assert_eq!(registry.lookup("sess-site"), ContextLookup::Unavailable);

    // The live map is maintained again: a newly registered session is immediately
    // visible to the routes that read it without readiness.
    registry.replace_repo(
        1,
        &repo("site"),
        vec![
            entry("sess-site", "site", 42),
            entry("sess-new", "site", 43),
        ],
    );
    assert!(registry.get("sess-new").is_some());

    // And a later complete generation still recovers readiness.
    registry.begin_generation([(1, repo("site"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.is_ready());
    assert_eq!(
        registry.lookup("sess-new"),
        ContextLookup::Unknown,
        "the complete generation is authoritative again"
    );
}

#[test]
fn a_repo_failure_outside_the_staged_set_leaves_the_generation_alone() {
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation([(1, repo("site"))].into_iter().collect());
    // A webhook-driven repository that is not part of this generation failing says
    // nothing about the generation's completability.
    registry.record_repo_failure(1, &repo("other"));
    assert_eq!(registry.snapshot().pending_repositories, 1);
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.is_ready(), "the generation still published");
}

#[test]
fn degrading_a_refresh_keeps_a_ready_projection_ready_and_fresh() {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &repo("site"),
        vec![entry("sess-old", "site", 1), entry("sess-keep", "site", 2)],
    );
    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    // `site` reports a set in which `sess-old` retired; `web` then fails forever.
    registry.replace_repo(1, &repo("site"), vec![entry("sess-keep", "site", 2)]);
    registry.record_repo_failure(1, &repo("web"));

    assert!(
        registry.is_ready(),
        "a failed refresh must never flap a healthy deployment into 503"
    );
    assert!(matches!(
        registry.lookup("sess-keep"),
        ContextLookup::Found(_)
    ));
    assert_eq!(
        registry.lookup("sess-old"),
        ContextLookup::Unknown,
        "the folded per-repository set is a replacement, so a retired grant still dies"
    );
}

#[test]
fn a_repo_reporting_an_empty_set_still_retires_its_grants_when_degraded() {
    // "Reported nothing" is the opposite instruction to "said nothing": the folded
    // generation must drop the repository's live entries even though it
    // contributed no context of its own.
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(1, &repo("site"), vec![entry("sess-gone", "site", 1)]);
    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![]);
    registry.record_repo_failure(1, &repo("web"));
    assert_eq!(registry.lookup("sess-gone"), ContextLookup::Unknown);
}

#[test]
fn a_superseding_generation_folds_the_one_that_never_completed() {
    // The backstop for a repository that vanished from the queue WITHOUT failing
    // (a dropped enqueue): no error path reports it, so the next full resync's
    // `begin_generation` is what unfreezes the live map.
    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    assert!(registry.get("sess-site").is_none(), "still staged");

    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());
    assert!(
        registry.get("sess-site").is_some(),
        "the abandoned generation's work survives in the live map"
    );
}

#[test]
fn a_successful_repo_sweep_removes_that_repos_retired_sessions_only() {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &repo("site"),
        vec![entry("sess-a", "site", 1), entry("sess-b", "site", 2)],
    );
    registry.replace_repo(1, &repo("web"), vec![entry("sess-w", "web", 3)]);
    assert_eq!(registry.len(), 3);

    // `sess-b` retired: the repo's next complete sweep no longer lists it.
    registry.replace_repo(1, &repo("site"), vec![entry("sess-a", "site", 1)]);
    assert!(matches!(registry.lookup("sess-a"), ContextLookup::Found(_)));
    assert_eq!(
        registry.lookup("sess-b"),
        ContextLookup::Unknown,
        "a retired registration must not survive as a stale grant"
    );
    assert!(
        matches!(registry.lookup("sess-w"), ContextLookup::Found(_)),
        "another repository's entries are untouched"
    );
}

#[test]
fn replacement_is_scoped_by_installation_as_well_as_repository() {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(1, &repo("site"), vec![entry("sess-one", "site", 1)]);
    registry.replace_repo(
        2,
        &repo("site"),
        vec![(
            "sess-two".to_string(),
            context_in(2, "site", Some(2), "bob", &[], &[]),
        )],
    );
    // Installation 1 sweeps `acme/site` empty; installation 2's same-named
    // repository must be unaffected.
    registry.replace_repo(1, &repo("site"), vec![]);
    assert_eq!(registry.lookup("sess-one"), ContextLookup::Unknown);
    assert!(matches!(
        registry.lookup("sess-two"),
        ContextLookup::Found(_)
    ));
}

#[test]
fn get_ignores_readiness_so_legacy_session_routes_keep_their_semantics() {
    let registry = SessionAccessRegistry::new(true);
    assert!(
        registry.get("sess-a").is_none(),
        "a cold registry has always answered 404 on the log route"
    );
    registry.replace_repo(1, &repo("site"), vec![entry("sess-a", "site", 42)]);
    assert!(
        registry.get("sess-a").is_some(),
        "an incremental write is readable by the legacy path even before readiness"
    );
}

#[test]
fn unknown_and_malformed_session_ids_never_resolve() {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(1, &repo("site"), vec![entry("sess-a", "site", 42)]);
    for probe in ["", "   ", "sess-a ", "SESS-A", "../sess-a", "sess-a\0"] {
        assert_eq!(
            registry.lookup(probe),
            ContextLookup::Unknown,
            "probe {probe:?} must not resolve"
        );
    }
}

#[test]
fn concurrent_readers_never_observe_a_mixed_generation() {
    let registry = Arc::new(SessionAccessRegistry::new(true));
    registry.begin_generation([(1, repo("site")), (1, repo("web"))].into_iter().collect());

    let reader = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let mut mixed = 0usize;
            for _ in 0..2_000 {
                let snapshot = registry.snapshot();
                let site = registry.lookup("sess-site");
                let web = registry.lookup("sess-web");
                if snapshot.state.is_ready() {
                    // A published generation is complete by construction: both
                    // repositories' entries are present together.
                    if matches!(site, ContextLookup::Unknown)
                        || matches!(web, ContextLookup::Unknown)
                    {
                        mixed += 1;
                    }
                } else if !matches!(site, ContextLookup::Unavailable)
                    || !matches!(web, ContextLookup::Unavailable)
                {
                    mixed += 1;
                }
            }
            mixed
        })
    };

    registry.replace_repo(1, &repo("site"), vec![entry("sess-site", "site", 42)]);
    registry.replace_repo(1, &repo("web"), vec![entry("sess-web", "web", 43)]);

    let mixed = reader.join().expect("reader thread");
    assert_eq!(
        mixed, 0,
        "a reader observed a partially published generation"
    );
}

#[test]
fn a_clone_shares_one_backing_store() {
    let registry = SessionAccessRegistry::new(false);
    let handle = registry.clone();
    handle.replace_repo(1, &repo("site"), vec![entry("sess-a", "site", 42)]);
    assert!(registry.get("sess-a").is_some());
}

#[test]
fn debug_and_snapshot_expose_only_bounded_counts() {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &repo("site"),
        vec![(
            "sess-secret-id".to_string(),
            context_in(1, "site", Some(42), "alice", &["bob"], &["carol"]),
        )],
    );
    let rendered = format!("{registry:?}");
    assert!(rendered.contains("sessions"), "{rendered}");
    assert!(rendered.contains("ready"), "{rendered}");
    for leak in ["sess-secret-id", "alice", "bob", "carol", "acme"] {
        assert!(!rendered.contains(leak), "{leak} leaked: {rendered}");
    }
}
