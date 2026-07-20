//! Tests for the install-time trigger-issue seeder: the seed body round-trips
//! through the real trigger parser, and the seeder honors the idempotency probe.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::goals::trigger_parse::parse_trigger_issue_body;
use crate::reconcile::execute_test_support::test_config;

/// A recorded `create_issue` call: `(owner, repo, title, body, labels)`.
type CreatedIssue = (String, String, String, String, Vec<String>);

/// A minimal GitHub transport for the seeder: token mint is stubbed; the
/// idempotency probe returns `existing`; `create_issue` is recorded.
#[derive(Default)]
struct SeedFake {
    existing: Vec<u64>,
    create_calls: AtomicUsize,
    created: Mutex<Vec<CreatedIssue>>,
}

#[async_trait]
impl GithubApi for SeedFake {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(1))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        Ok(InstallationToken {
            token: SecretString::from("ghs_fake".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }

    async fn open_issues_with_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _label: &str,
    ) -> Result<Vec<u64>, GithubAppError> {
        Ok(self.existing.clone())
    }

    async fn create_issue(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, GithubAppError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        self.created.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            title.to_string(),
            body.to_string(),
            labels.to_vec(),
        ));
        Ok(4242)
    }
}

fn tokens(api: std::sync::Arc<SeedFake>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

/// The default fkst-manifest ref an I9 seed carries (matches
/// `reconcile_config::DEFAULT_MANIFEST_REF`).
const DEFAULT_MANIFEST: &str =
    "ChronoAIProject/fkst-packages@fkst-hosted:manifests/default-workflows.json";

#[test]
fn manifest_seed_body_round_trips_with_manifest_and_no_packages_or_work_label() {
    // The I9 default: a `### Manifest` reference supplies the packages and the wake
    // labels auto-discover, so the body carries NEITHER `### Packages` NOR
    // `### Work Label`. It must still parse as a valid trigger issue.
    let body = build_seed_body(&[], Some(DEFAULT_MANIFEST), "octo-owner");
    // Shape assertions: manifest present, packages/work-label absent.
    assert!(body.contains("### Manifest"), "carries a manifest section");
    assert!(
        !body.contains("### Packages"),
        "manifest seed omits ### Packages"
    );
    assert!(
        !body.contains("### Work Label"),
        "manifest seed omits ### Work Label (labels auto-detect)"
    );

    let spec =
        parse_trigger_issue_body(&body).expect("manifest seed body is a valid trigger issue");
    assert_eq!(spec.name, "default-workflows");
    assert!(
        spec.work_label.is_none(),
        "no explicit work label — the wake labels auto-discover from the manifest's packages"
    );
    assert!(spec.packages.is_empty(), "no explicit ### Packages");
    assert_eq!(
        spec.manifest_refs.len(),
        1,
        "exactly the default manifest ref"
    );
    let m = &spec.manifest_refs[0];
    assert_eq!(m.owner, "ChronoAIProject");
    assert_eq!(m.repo, "fkst-packages");
    assert_eq!(m.git_ref, "fkst-hosted");
    assert_eq!(m.path, "manifests/default-workflows.json");
    assert!(spec.auto_merge, "seed sets auto-merge on");
    assert_eq!(spec.log_access, vec!["octo-owner".to_string()]);
    assert!(spec.environment.is_none());
}

#[test]
fn legacy_seed_body_round_trips_through_the_real_trigger_parser() {
    // With NO default manifest configured, the seeder falls back to the legacy
    // explicit `### Packages` + `### Work Label` body — this pins that shape to the
    // parser so the two can never drift.
    let default_pkgs =
        vec!["ChronoAIProject/fkst-packages@dev:packages/github-devloop-workflow".to_string()];
    let spec = parse_trigger_issue_body(&build_seed_body(&default_pkgs, None, "octo-owner"))
        .expect("legacy seed body is a valid trigger issue");
    assert_eq!(spec.name, "evolve");
    assert_eq!(spec.work_label.as_deref(), Some("fkst-evolve"));
    assert!(
        spec.manifest_refs.is_empty(),
        "legacy body has no ### Manifest"
    );
    assert!(spec.auto_merge, "seed sets auto-merge on");
    assert_eq!(spec.log_access, vec!["octo-owner".to_string()]);
    assert_eq!(spec.packages.len(), 1);
    let p = &spec.packages[0];
    assert_eq!(p.owner, "ChronoAIProject");
    assert_eq!(p.repo, "fkst-packages");
    assert_eq!(p.git_ref, "dev");
    assert_eq!(p.path, "packages/github-devloop-workflow");
    assert!(spec.environment.is_none());
}

#[tokio::test]
async fn seeds_a_repo_with_no_open_trigger_issue() {
    // The default (manifest-driven) path: verify the created issue's title, labels,
    // and that the recorded body is the manifest-driven trigger body.
    let api = std::sync::Arc::new(SeedFake::default()); // existing = empty
    let github = tokens(api.clone());
    seed_trigger_issues(
        &github,
        "fkst-substrate-trigger",
        &[],
        Some(DEFAULT_MANIFEST),
        "octo-owner",
        &["octo-owner/repo-a".to_string()],
    )
    .await;

    assert_eq!(api.create_calls.load(Ordering::SeqCst), 1);
    let created = api.created.lock().unwrap();
    let (owner, repo, title, body, labels) = &created[0];
    assert_eq!(owner, "octo-owner");
    assert_eq!(repo, "repo-a");
    assert_eq!(title, "[session] default-workflows (auto-seeded)");
    assert_eq!(labels, &vec!["fkst-substrate-trigger".to_string()]);
    // The recorded body is the manifest-driven trigger body (parses; carries a
    // manifest ref; no explicit work label).
    let spec = parse_trigger_issue_body(body).expect("recorded body parses");
    assert_eq!(spec.manifest_refs.len(), 1);
    assert!(spec.work_label.is_none());
}

#[tokio::test]
async fn skips_a_repo_that_already_has_an_open_trigger_issue() {
    let api = std::sync::Arc::new(SeedFake {
        existing: vec![7], // an open trigger issue already exists
        ..SeedFake::default()
    });
    let github = tokens(api.clone());
    seed_trigger_issues(
        &github,
        "fkst-substrate-trigger",
        &[],
        Some(DEFAULT_MANIFEST),
        "octo-owner",
        &["octo-owner/repo-a".to_string()],
    )
    .await;
    assert_eq!(
        api.create_calls.load(Ordering::SeqCst),
        0,
        "must NOT create a second trigger issue"
    );
}

#[tokio::test]
async fn legacy_seed_used_when_no_default_manifest_is_configured() {
    // `default_manifest = None` (a blank FKST_DEFAULT_MANIFEST) → the legacy body,
    // titled for the `evolve` session, carrying the configured packages.
    let api = std::sync::Arc::new(SeedFake::default());
    let github = tokens(api.clone());
    seed_trigger_issues(
        &github,
        "fkst-substrate-trigger",
        &["ChronoAIProject/fkst-packages@dev:packages/github-devloop-workflow".to_string()],
        None,
        "octo-owner",
        &["octo-owner/repo-a".to_string()],
    )
    .await;

    assert_eq!(api.create_calls.load(Ordering::SeqCst), 1);
    let created = api.created.lock().unwrap();
    let (_owner, _repo, title, body, _labels) = &created[0];
    assert_eq!(title, "[session] evolve (auto-seeded)");
    let spec = parse_trigger_issue_body(body).expect("recorded body parses");
    assert_eq!(spec.work_label.as_deref(), Some("fkst-evolve"));
    assert_eq!(spec.packages.len(), 1);
    assert!(spec.manifest_refs.is_empty());
}

#[test]
fn legacy_seed_body_with_multiple_packages_round_trips_in_order() {
    // A configured multi-package list renders one `### Packages` line each and
    // parses back to the same ordered refs — pins FKST_SEED_PACKAGES support for the
    // legacy (no-manifest) body.
    let pkgs = vec![
        "chronoai-shining/fkst-packages@feat/workflow-engine:packages/workflow-dev".to_string(),
        "chronoai-shining/fkst-packages@feat/workflow-engine:packages/workflow-security"
            .to_string(),
        "chronoai-shining/fkst-packages@feat/workflow-engine:packages/workflow-writer".to_string(),
    ];
    let spec = parse_trigger_issue_body(&build_seed_body(&pkgs, None, "octo-owner"))
        .expect("multi-package seed body is valid");
    assert_eq!(spec.packages.len(), 3);
    let paths: Vec<&str> = spec.packages.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "packages/workflow-dev",
            "packages/workflow-security",
            "packages/workflow-writer"
        ]
    );
    for p in &spec.packages {
        assert_eq!(p.owner, "chronoai-shining");
        assert_eq!(p.repo, "fkst-packages");
        assert_eq!(p.git_ref, "feat/workflow-engine");
    }
}
