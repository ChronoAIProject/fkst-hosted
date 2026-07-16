//! Tests for work-label auto-discovery: the `[github].work_labels` union across a
//! package set and its transitive `[event_deps]` closure, over a mocked GitHub
//! contents API.

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::resolve_work_labels;
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
