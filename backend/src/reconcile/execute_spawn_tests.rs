//! Tests for the branch topology a spawn provisions before any runtime exists.
//!
//! The invariants are narrow and load-bearing: a missing target is created at the
//! current source head, an EXISTING target is never reset (that would discard
//! merged work), and any lookup failure skips the spawn rather than guessing.

use std::sync::Arc;

use super::*;
use crate::reconcile::execute_test_support::*;

fn target_branch_ctx(api: Arc<RecordingApi>) -> ReconcileCtx {
    let backend = Arc::new(FakeSessionBackend::default());
    let mut ctx = test_ctx(backend);
    ctx.github = tokens(api);
    ctx
}

#[tokio::test]
async fn missing_target_is_created_at_the_current_source_head() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    let ctx = target_branch_ctx(api.clone());

    let topology = ensure_branch_topology(&registration(), &ctx)
        .await
        .expect("branch topology resolves");
    assert_eq!(
        topology,
        ResolvedBranchTopology {
            upstream: "main".to_string(),
            integration: "fkst-hosted-default".to_string(),
        }
    );
    assert_eq!(
        *api.create_refs.lock().unwrap(),
        [("fkst-hosted-default".to_string(), "source-head".to_string())]
    );
}

#[tokio::test]
async fn target_create_lost_race_is_successful() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    *api.create_ref_error.lock().unwrap() = Some(GithubAppError::RefExists);
    let ctx = target_branch_ctx(api);

    assert!(ensure_branch_topology(&registration(), &ctx)
        .await
        .is_some());
}

#[tokio::test]
async fn existing_target_is_never_reset_or_recreated() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "fkst-hosted-default".to_string(),
        "existing-head".to_string(),
    )]));
    let ctx = target_branch_ctx(api.clone());

    let topology = ensure_branch_topology(&registration(), &ctx)
        .await
        .expect("existing target still resolves its upstream");
    assert_eq!(topology.upstream, "main");
    assert_eq!(topology.integration, "fkst-hosted-default");
    assert!(api.create_refs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_source_remains_upstream_after_target_exists() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "integration/release".to_string(),
        "existing-head".to_string(),
    )]));
    let ctx = target_branch_ctx(api.clone());
    let mut reg = registration();
    reg.def.source_branch = Some("release/v2".to_string());
    reg.def.target_branch = Some("integration/release".to_string());

    let topology = ensure_branch_topology(&reg, &ctx)
        .await
        .expect("explicit topology resolves");

    assert_eq!(topology.upstream, "release/v2");
    assert_eq!(topology.integration, "integration/release");
    assert!(api.create_refs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn target_create_failure_skips_spawn_without_issue_feedback() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    *api.create_ref_error.lock().unwrap() = Some(GithubAppError::Http(
        "branch protection denied creation".to_string(),
    ));
    let ctx = target_branch_ctx(api.clone());

    assert!(ensure_branch_topology(&registration(), &ctx)
        .await
        .is_none());
    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
}

// ---- pure argument assembly -------------------------------------------------
