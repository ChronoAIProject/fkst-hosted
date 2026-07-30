//! Tests for fkst-manifest expansion: fetch + fail-closed validation of a manifest
//! JSON into its concrete [`PackageRef`] list, over a mocked GitHub contents API.

use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{expand_manifest, ManifestError};
use crate::goals::trigger_parse::PackageRef;

/// A distinctive token so the leak-free tests can assert it never surfaces in an
/// error's `Display`/`Debug`.
const SECRET_TOKEN: &str = "ghs_manifest_secret_deadbeef";

fn pkg(owner: &str, repo: &str, git_ref: &str, path: &str) -> PackageRef {
    PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    }
}

fn tok() -> SecretString {
    SecretString::from(SECRET_TOKEN.to_string())
}

/// The manifest file reference every test fetches (`owner/repo@ref:path`, where the
/// path is the JSON file itself).
fn manifest_ref() -> PackageRef {
    pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        "manifests/default-workflows.json",
    )
}

/// The 14 package names the authored `default-workflows.json` bundles.
const PACKAGE_NAMES: [&str; 14] = [
    "workflow-dev",
    "workflow-security",
    "workflow-writer",
    "workflow-reviewer",
    "workflow-planner",
    "workflow-tester",
    "workflow-docs",
    "workflow-release",
    "github-proxy",
    "security-adapter",
    "codex-triage",
    "issue-intake",
    "pr-merger",
    "log-streamer",
];

/// The 14 package reference strings, spelled exactly as a manifest authors them.
fn fourteen_ref_strings() -> Vec<String> {
    PACKAGE_NAMES
        .iter()
        .map(|name| format!("ChronoAIProject/fkst-hosted@packages:packages/{name}"))
        .collect()
}

/// The expected parsed [`PackageRef`] for one bundled package name.
fn expected_ref(name: &str) -> PackageRef {
    pkg(
        "ChronoAIProject",
        "fkst-hosted",
        "packages",
        &format!("packages/{name}"),
    )
}

/// Serialize a manifest body with the given schema version + package strings.
fn manifest_body(schema_version: i64, packages: &[String]) -> String {
    json!({
        "schemaVersion": schema_version,
        "name": "default-workflows",
        "description": "Curated default workflow package set",
        "packages": packages,
    })
    .to_string()
}

/// Mount the manifest JSON at the exact contents path `manifest_ref()` fetches.
async fn mount_body(server: &MockServer, body: String) {
    let m = manifest_ref();
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/{}/{}/contents/{}",
            m.owner, m.repo, m.path
        )))
        .and(query_param("ref", m.git_ref.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// Mount a non-2xx status at the manifest path.
async fn mount_status(server: &MockServer, status: u16) {
    let m = manifest_ref();
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/{}/{}/contents/{}",
            m.owner, m.repo, m.path
        )))
        .and(query_param("ref", m.git_ref.as_str()))
        .respond_with(ResponseTemplate::new(status))
        .mount(server)
        .await;
}

/// The package list only — every pre-existing test asserts on packages, so this
/// keeps them expressing exactly what they did before `packageEnv` existed.
async fn expand(server: &MockServer) -> Result<Vec<PackageRef>, ManifestError> {
    expand_full(server).await.map(|expanded| expanded.packages)
}

/// The whole expansion, for the `packageEnv` tests.
async fn expand_full(
    server: &MockServer,
) -> Result<crate::reconcile::manifest_expand::ExpandedManifest, ManifestError> {
    expand_manifest(
        &reqwest::Client::new(),
        &server.uri(),
        &tok(),
        &manifest_ref(),
    )
    .await
}

#[tokio::test]
async fn valid_manifest_expands_all_fourteen_refs() {
    let server = MockServer::start().await;
    mount_body(&server, manifest_body(1, &fourteen_ref_strings())).await;

    let refs = expand(&server).await.expect("valid manifest expands");

    let expected: Vec<PackageRef> = PACKAGE_NAMES.iter().map(|n| expected_ref(n)).collect();
    assert_eq!(refs, expected);
    // Spot-check the parse landed on the manifest's owner/repo/ref, not the file path.
    assert_eq!(refs.len(), 14);
    assert_eq!(refs[0].owner, "ChronoAIProject");
    assert_eq!(refs[0].repo, "fkst-hosted");
    assert_eq!(refs[0].git_ref, "packages");
    assert_eq!(refs[0].path, "packages/workflow-dev");
    assert_eq!(refs[13].path, "packages/log-streamer");
}

#[tokio::test]
async fn authenticated_forbidden_falls_back_to_public_manifest_read() {
    let server = MockServer::start().await;
    let m = manifest_ref();
    let manifest_path = format!("/repos/{}/{}/contents/{}", m.owner, m.repo, m.path);

    Mock::given(method("GET"))
        .and(path(manifest_path.as_str()))
        .and(query_param("ref", m.git_ref.as_str()))
        .and(header(
            "authorization",
            format!("Bearer {SECRET_TOKEN}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(manifest_path))
        .and(query_param("ref", m.git_ref.as_str()))
        .and(|request: &Request| !request.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_body(
            1,
            &["ChronoAIProject/fkst-hosted@packages:packages/workflow-dev".to_string()],
        )))
        .expect(1)
        .mount(&server)
        .await;

    let refs = expand(&server)
        .await
        .expect("public manifest should be retried without auth");

    assert_eq!(refs, vec![expected_ref("workflow-dev")]);
}

#[tokio::test]
async fn unsupported_schema_version_is_rejected() {
    let server = MockServer::start().await;
    mount_body(&server, manifest_body(2, &fourteen_ref_strings())).await;

    let err = expand(&server)
        .await
        .expect_err("schemaVersion 2 must fail");
    assert!(
        matches!(err, ManifestError::BadSchemaVersion(2)),
        "expected BadSchemaVersion(2), got {err:?}"
    );
}

#[tokio::test]
async fn empty_package_list_is_rejected() {
    let server = MockServer::start().await;
    mount_body(&server, manifest_body(1, &[])).await;

    let err = expand(&server).await.expect_err("empty packages must fail");
    assert!(
        matches!(err, ManifestError::Empty),
        "expected Empty, got {err:?}"
    );
}

#[tokio::test]
async fn over_the_cap_is_rejected() {
    let server = MockServer::start().await;
    // 65 valid refs — one past the 64 ceiling.
    let packages: Vec<String> = (0..65)
        .map(|i| format!("ChronoAIProject/fkst-hosted@packages:packages/p{i}"))
        .collect();
    mount_body(&server, manifest_body(1, &packages)).await;

    let err = expand(&server).await.expect_err("65 packages must fail");
    assert!(
        matches!(err, ManifestError::TooMany { count: 65, max: 64 }),
        "expected TooMany {{ 65, 64 }}, got {err:?}"
    );
}

#[tokio::test]
async fn malformed_ref_names_its_index() {
    let server = MockServer::start().await;
    // A bad entry (no `@`) sits at index 1, between two valid refs.
    let packages = vec![
        "ChronoAIProject/fkst-hosted@packages:packages/workflow-dev".to_string(),
        "not-a-valid-package-reference".to_string(),
        "ChronoAIProject/fkst-hosted@packages:packages/workflow-writer".to_string(),
    ];
    mount_body(&server, manifest_body(1, &packages)).await;

    let err = expand(&server).await.expect_err("malformed ref must fail");
    match err {
        ManifestError::BadRef { index, detail } => {
            assert_eq!(index, 1, "must name the offending index");
            assert!(
                detail.contains("not-a-valid-package-reference"),
                "detail should echo the offending value: {detail}"
            );
        }
        other => panic!("expected BadRef, got {other:?}"),
    }
}

#[tokio::test]
async fn non_json_body_is_a_parse_error() {
    let server = MockServer::start().await;
    mount_body(&server, "this is not json at all".to_string()).await;

    let err = expand(&server).await.expect_err("non-JSON must fail");
    assert!(
        matches!(err, ManifestError::Parse),
        "expected Parse, got {err:?}"
    );
}

#[tokio::test]
async fn missing_manifest_is_not_found() {
    let server = MockServer::start().await;
    mount_status(&server, 404).await;

    let err = expand(&server).await.expect_err("404 must fail");
    assert!(
        matches!(err, ManifestError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[tokio::test]
async fn other_http_error_is_a_fetch_error() {
    let server = MockServer::start().await;
    mount_status(&server, 500).await;

    let err = expand(&server).await.expect_err("500 must fail");
    assert!(
        matches!(err, ManifestError::Fetch(_)),
        "expected Fetch, got {err:?}"
    );
}

#[tokio::test]
async fn unknown_extra_field_still_parses() {
    let server = MockServer::start().await;
    // Forward-compat: a future manifest key must not break today's expander.
    let body = json!({
        "schemaVersion": 1,
        "name": "default-workflows",
        "description": "…",
        "packages": ["ChronoAIProject/fkst-hosted@packages:packages/workflow-dev"],
        "futureField": { "nested": true },
    })
    .to_string();
    mount_body(&server, body).await;

    let refs = expand(&server).await.expect("unknown field is ignored");
    assert_eq!(refs, vec![expected_ref("workflow-dev")]);
}

#[tokio::test]
async fn errors_never_leak_the_token_or_url() {
    let server = MockServer::start().await;
    mount_status(&server, 500).await;
    let uri = server.uri();

    // A fetch failure carries only a curated reason.
    let fetch_err = expand(&server).await.expect_err("500 must fail");
    for rendered in [format!("{fetch_err}"), format!("{fetch_err:?}")] {
        assert!(!rendered.contains(SECRET_TOKEN), "leaked token: {rendered}");
        assert!(!rendered.contains(&uri), "leaked url: {rendered}");
    }

    // The NotFound path likewise stays free of the token/URL.
    let nf_server = MockServer::start().await;
    mount_status(&nf_server, 404).await;
    let nf_uri = nf_server.uri();
    let nf_err = expand(&nf_server).await.expect_err("404 must fail");
    for rendered in [format!("{nf_err}"), format!("{nf_err:?}")] {
        assert!(!rendered.contains(SECRET_TOKEN), "leaked token: {rendered}");
        assert!(!rendered.contains(&nf_uri), "leaked url: {rendered}");
    }
}

/// A manifest may supply per-package configuration for every session that uses it,
/// so a fleet-wide package setting is written once instead of in every trigger.
#[tokio::test]
async fn package_env_is_expanded() {
    let server = MockServer::start().await;
    mount_body(
        &server,
        serde_json::json!({
            "schemaVersion": 1,
            "packages": ["acme/tools@main:pkg/a"],
            "packageEnv": {
                "github-devloop": { "FKST_DEVLOOP_AUTO_REFINE_MAX": "2" }
            }
        })
        .to_string(),
    )
    .await;

    let expanded = expand_full(&server).await.expect("valid manifest");
    assert_eq!(
        expanded.package_env["github-devloop"]["FKST_DEVLOOP_AUTO_REFINE_MAX"],
        "2"
    );
}

/// Every manifest written before this key existed must expand byte-identically.
#[tokio::test]
async fn a_manifest_without_package_env_expands_to_an_empty_map() {
    let server = MockServer::start().await;
    mount_body(
        &server,
        manifest_body(1, &["acme/tools@main:pkg/a".to_string()]),
    )
    .await;

    let expanded = expand_full(&server).await.expect("valid manifest");
    assert!(expanded.package_env.is_empty());
}

/// The manifest and the trigger share ONE validator, so a manifest can never ship
/// configuration a trigger author would be refused. Each case here is rejected by
/// the same rule that rejects it in `### Package Env`.
#[tokio::test]
async fn a_malformed_package_env_fails_closed() {
    for (label, block) in [
        (
            "bad key",
            serde_json::json!({ "pkg": { "lowercase": "x" } }),
        ),
        (
            "platform-owned key",
            serde_json::json!({ "pkg": { "FKST_GITHUB_BOT_LOGIN": "x" } }),
        ),
        (
            "cross-block conflict",
            serde_json::json!({ "a": { "FKST_SHARED": "1" }, "b": { "FKST_SHARED": "2" } }),
        ),
        (
            "bad package name",
            serde_json::json!({ "has spaces": { "FKST_A": "1" } }),
        ),
    ] {
        let server = MockServer::start().await;
        mount_body(
            &server,
            serde_json::json!({
                "schemaVersion": 1,
                "packages": ["acme/tools@main:pkg/a"],
                "packageEnv": block
            })
            .to_string(),
        )
        .await;

        match expand_full(&server).await {
            Err(ManifestError::BadPackageEnv { .. }) => {}
            other => panic!("{label}: expected BadPackageEnv, got {other:?}"),
        }
    }
}
