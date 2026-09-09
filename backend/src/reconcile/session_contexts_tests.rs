//! Unit tests for session-access context publication.
//!
//! These assert the AUTHORIZATION contract of the reconciler's write path: what
//! the projection ends up holding, and that a completed sweep removes what it no
//! longer lists.

use super::*;

use crate::reconcile::execute_test_support::{test_ctx, FakeSessionBackend};

/// A registration carrying the three authorization facts the projection stores.
fn access_registration(session: &str, creator_id: Option<i64>) -> SessionRegistration {
    use crate::goals::package_env::PackageEnv;
    use crate::goals::trigger_parse::PackageRef;
    use crate::reconcile::desired::SessionDef;

    SessionRegistration {
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue: 7,
        // Deliberately different from the creator: the trigger author must never
        // be substituted for a missing creator id.
        trigger_author_id: 9000,
        trigger_author_login: "fkst-app[bot]".to_string(),
        creator_login: "alice".to_string(),
        creator_id,
        def: SessionDef {
            name: session.to_string(),
            packages: Vec::<PackageRef>::new(),
            manifest_refs: Vec::<PackageRef>::new(),
            work_label: Some("fkst-run".to_string()),
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
            source_branch: None,
            target_branch: None,
            package_env: PackageEnv::new(),
        },
        effective_packages: Vec::new(),
        session_id: session.to_string(),
        config_hash: "hash".to_string(),
        auto_merge: false,
        log_access: vec!["carol".to_string()],
        collaborators: vec!["bob".to_string()],
        effective_package_env: PackageEnv::new(),
    }
}

#[tokio::test]
async fn session_registrations_round_trip_creator_collaborators_and_log_access() {
    let backend = std::sync::Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend);
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };

    record_session_contexts(&ctx, 1, &repo, &[access_registration("sess-a", Some(101))]);
    let context = ctx
        .session_access
        .get("sess-a")
        .expect("published after a successful sweep");
    assert_eq!(context.installation_id, 1);
    assert_eq!(context.trigger_issue, 7);
    assert_eq!(context.creator.id, Some(101));
    assert_eq!(context.creator.login, "alice");
    assert_eq!(context.collaborators, vec!["bob".to_string()]);
    assert_eq!(context.log_access, vec!["carol".to_string()]);
}

#[tokio::test]
async fn a_missing_creator_id_is_preserved_rather_than_backfilled() {
    let backend = std::sync::Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend);
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };

    record_session_contexts(&ctx, 1, &repo, &[access_registration("sess-a", None)]);
    let context = ctx.session_access.get("sess-a").expect("published");
    assert_eq!(
        context.creator.id, None,
        "the trigger author's id must never stand in for a missing creator id"
    );
    assert_eq!(context.creator.login, "alice");
}

#[tokio::test]
async fn a_successful_sweep_replaces_the_repositorys_whole_set() {
    let backend = std::sync::Arc::new(FakeSessionBackend::default());
    let ctx = test_ctx(backend);
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };

    record_session_contexts(
        &ctx,
        1,
        &repo,
        &[
            access_registration("sess-a", Some(101)),
            access_registration("sess-b", Some(102)),
        ],
    );
    assert_eq!(ctx.session_access.len(), 2);

    // `sess-b`'s trigger closed: the next complete sweep no longer lists it, so
    // its grant must be gone rather than surviving the session it described.
    record_session_contexts(&ctx, 1, &repo, &[access_registration("sess-a", Some(101))]);
    assert!(ctx.session_access.get("sess-a").is_some());
    assert!(
        ctx.session_access.get("sess-b").is_none(),
        "a retired registration must not survive as a stale grant"
    );
}
