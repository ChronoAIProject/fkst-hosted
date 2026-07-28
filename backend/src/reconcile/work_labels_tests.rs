//! Tests for work-label auto-discovery: the `[github].work_labels` union across a
//! package set and its transitive `[event_deps]` closure, over a mocked GitHub
//! contents API.

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{
    apply_work_label_namespace, provider_session_issue_title, recover_work_label_scope,
    resolve_work_labels, validate_work_label_namespace, GITHUB_LABEL_NAME_MAX_CHARS,
};
use crate::goals::trigger_parse::PackageRef;

fn pkg(owner: &str, repo: &str, git_ref: &str, path: &str) -> PackageRef {
    PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    }
}

fn tok() -> SecretString {
    SecretString::from("t".to_string())
}

#[test]
fn provider_session_title_expands_the_namespace_for_humans() {
    assert_eq!(
        provider_session_issue_title("chronoai-fkst-cloud", "Default FKST Substrate"),
        "🔔[CHRONOAI FKST CLOUD SESSION] Default FKST Substrate"
    );
}

/// Mount a `fkst.toml` body at `contents/{path}/fkst.toml`.
async fn mount_manifest(server: &MockServer, repo_path: &str, git_ref: &str, body: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/{repo_path}/fkst.toml")))
        .and(query_param("ref", git_ref))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.to_string()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn unions_declared_labels_across_packages() {
    let server = MockServer::start().await;
    mount_manifest(
        &server,
        "o/r/contents/packages/workflow-security",
        "main",
        "[github]\nwork_labels = [\"fkst-security\"]\n",
    )
    .await;
    mount_manifest(
        &server,
        "o/r/contents/packages/workflow-writer",
        "main",
        "[github]\nwork_labels = [\"fkst-workflow\"]\n",
    )
    .await;

    let labels = resolve_work_labels(
        &reqwest::Client::new(),
        &server.uri(),
        &tok(),
        &[
            pkg("o", "r", "main", "packages/workflow-security"),
            pkg("o", "r", "main", "packages/workflow-writer"),
        ],
    )
    .await;

    assert!(labels.contains("fkst-security"));
    assert!(labels.contains("fkst-workflow"));
    assert_eq!(labels.len(), 2);
}

#[tokio::test]
async fn resolves_event_deps_transitively() {
    let server = MockServer::start().await;
    // Root package declares nothing itself but pulls in two sibling packages.
    mount_manifest(
        &server,
        "o/r/contents/packages/workflow-dev",
        "dev",
        "[event_deps]\npackages = [\"github-proxy\", \"security-adapter\"]\n",
    )
    .await;
    mount_manifest(
        &server,
        "o/r/contents/packages/github-proxy",
        "dev",
        "[github]\nwork_labels = [\"fkst-dev\"]\n[event_deps]\npackages = [\"security-adapter\"]\n",
    )
    .await;
    mount_manifest(
        &server,
        "o/r/contents/packages/security-adapter",
        "dev",
        "[github]\nwork_labels = [\"fkst-security\"]\n",
    )
    .await;

    let labels = resolve_work_labels(
        &reqwest::Client::new(),
        &server.uri(),
        &tok(),
        &[pkg("o", "r", "dev", "packages/workflow-dev")],
    )
    .await;

    // Diamond (both root and github-proxy depend on security-adapter) is fetched
    // once and both transitive labels surface.
    assert!(labels.contains("fkst-dev"));
    assert!(labels.contains("fkst-security"));
    assert_eq!(labels.len(), 2);
}

#[tokio::test]
async fn missing_or_sectionless_manifests_contribute_nothing() {
    let server = MockServer::start().await;
    // A package whose manifest has no [github] section.
    mount_manifest(
        &server,
        "o/r/contents/packages/plain",
        "main",
        "kind = \"package.flat\"\nname = \"plain\"\n[code]\nroot = \".\"\n",
    )
    .await;
    // The other package path is not mounted → 404, contributes nothing (no panic).

    let labels = resolve_work_labels(
        &reqwest::Client::new(),
        &server.uri(),
        &tok(),
        &[
            pkg("o", "r", "main", "packages/plain"),
            pkg("o", "r", "main", "packages/missing"),
        ],
    )
    .await;

    assert!(labels.is_empty());
}

#[test]
fn provider_namespace_maps_multiple_labels_deterministically() {
    let labels = apply_work_label_namespace(
        &[
            "fkst-security".to_string(),
            "fkst-dev".to_string(),
            "fkst-dev".to_string(),
        ],
        Some("chronoai-fkst"),
    )
    .expect("valid namespace");

    assert_eq!(
        labels.logical,
        vec!["fkst-dev".to_string(), "fkst-security".to_string()]
    );
    assert_eq!(
        labels.effective,
        vec![
            "fkst-dev-chronoai-fkst".to_string(),
            "fkst-security-chronoai-fkst".to_string(),
        ]
    );
    assert_eq!(
        labels.map_json().as_deref(),
        Some(
            r#"{"fkst-dev":"fkst-dev-chronoai-fkst","fkst-security":"fkst-security-chronoai-fkst"}"#
        )
    );
}

#[test]
fn absent_namespace_is_identity_and_omits_mapping_env() {
    let labels = apply_work_label_namespace(&["fkst-dev".to_string()], None)
        .expect("identity mapping is valid");
    assert_eq!(labels.logical, labels.effective);
    assert_eq!(labels.map_json(), None);
}

#[test]
fn exact_family_scope_rejects_plain_foreign_dual_and_lifecycle_only_issues() {
    let scope = apply_work_label_namespace(
        &["fkst-dev".to_string(), "fkst-security".to_string()],
        Some("chronoai-fkst-cloud-test"),
    )
    .unwrap();
    let dev = "fkst-dev-chronoai-fkst-cloud-test";
    let security = "fkst-security-chronoai-fkst-cloud-test";

    assert!(scope.matches_issue_labels(&[
        dev.to_string(),
        format!("{dev}:claimed"),
        "bug".to_string(),
    ]));
    assert!(scope.matches_issue_labels(&[security.to_string(), dev.to_string(),]));
    for labels in [
        vec!["fkst-dev".to_string()],
        vec![dev.to_string(), "fkst-dev".to_string()],
        vec![dev.to_string(), "fkst-dev-other-provider".to_string()],
        vec![format!("{dev}:claimed")],
        vec![security.to_string(), "fkst-security:blocked".to_string()],
    ] {
        assert!(!scope.matches_issue_labels(&labels), "labels={labels:?}");
    }
}

#[test]
fn retirement_scope_round_trips_namespace_and_rejects_foreign_runtime_labels() {
    let effective = vec![
        "fkst-dev-chronoai-fkst-cloud-test".to_string(),
        "fkst-security-chronoai-fkst-cloud-test".to_string(),
    ];
    let recovered = recover_work_label_scope(&effective, Some("chronoai-fkst-cloud-test")).unwrap();
    assert_eq!(recovered.effective, effective);
    assert_eq!(
        recovered.logical,
        vec!["fkst-dev".to_string(), "fkst-security".to_string()]
    );

    let error = recover_work_label_scope(
        &["fkst-dev-another-provider".to_string()],
        Some("chronoai-fkst-cloud-test"),
    )
    .expect_err("foreign runtime metadata must fail closed");
    assert!(error.to_string().contains("does not belong"));
}

#[test]
fn namespace_and_derived_label_validation_fail_closed() {
    for invalid in [
        "ChronoAI-fkst",
        "chronoai_fkst",
        "-chronoai",
        "chronoai-",
        "chronoai--fkst",
        "provider namespace",
    ] {
        assert!(
            validate_work_label_namespace(invalid).is_err(),
            "{invalid} must be rejected"
        );
    }
    assert!(validate_work_label_namespace("chronoai-fkst").is_ok());

    let empty = apply_work_label_namespace(&[String::new()], Some("cloud"))
        .expect_err("an empty logical label cannot become valid through suffixing");
    assert!(empty.to_string().contains("must be non-empty"));

    let logical = "x".repeat(GITHUB_LABEL_NAME_MAX_CHARS);
    let error = apply_work_label_namespace(&[logical], Some("cloud"))
        .expect_err("derived label exceeds GitHub's limit");
    assert!(error.to_string().contains("at most 50"));
}

#[test]
fn case_insensitive_effective_collisions_are_rejected() {
    let error = apply_work_label_namespace(
        &["FKST-DEV".to_string(), "fkst-dev".to_string()],
        Some("cloud"),
    )
    .expect_err("GitHub label identity is case-insensitive");
    assert!(error.to_string().contains("collide"));
}
