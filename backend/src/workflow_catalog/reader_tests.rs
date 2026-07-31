use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use secrecy::SecretString;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::github_app::api::HttpGithubApi;
use crate::models::RepoRef;

use super::blueprint::StepKind;
use super::reader::read_repo_catalog;

#[tokio::test]
async fn reads_one_repo_local_blueprint_through_the_http_transport() {
    let server = MockServer::start().await;
    let token = SecretString::from("ghs_catalog_test".to_string());
    let document = r#"{
      "schema": "fkst.workflow.v1",
      "id": "release-hardening",
      "version": "1.0.0",
      "summary": "Check a release candidate before publication.",
      "applies_when": "A release candidate needs final review.",
      "selector": {
        "labels_any": ["release"],
        "title_contains_any": ["release"]
      },
      "steps": [{
        "id": "verify",
        "title": "Verify the release candidate",
        "content": {
          "kind": "static",
          "intent": "Inspect the release candidate and report blocking defects."
        }
      }]
    }"#;

    Mock::given(method("GET"))
        .and(path("/repos/acme/widgets/contents/.fkst/packages"))
        .and(query_param("ref", "feature/catalog"))
        .and(header("accept", "application/vnd.github+json"))
        .and(header("authorization", "Bearer ghs_catalog_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
            "name": "release-hardening.json",
            "path": ".fkst/packages/release-hardening.json",
            "sha": "directory-entry-sha",
            "size": 312,
            "type": "file",
            "url": "https://api.github.test/repos/acme/widgets/contents/.fkst/packages/release-hardening.json"
        }])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repos/acme/widgets/contents/.fkst/packages/release-hardening.json",
        ))
        .and(query_param("ref", "feature/catalog"))
        .and(header("accept", "application/vnd.github+json"))
        .and(header("authorization", "Bearer ghs_catalog_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "sha": "blueprint-blob-sha",
            "content": STANDARD.encode(document.as_bytes())
        })))
        .expect(1)
        .mount(&server)
        .await;

    let api = HttpGithubApi::new(&server.uri()).expect("HTTP transport");
    let view = read_repo_catalog(
        &api,
        &token,
        &RepoRef {
            owner: "acme".to_string(),
            name: "widgets".to_string(),
        },
        "feature/catalog",
    )
    .await
    .expect("catalog read");

    assert_eq!(view.workflows.len(), 1);
    let workflow = &view.workflows[0];
    assert_eq!(workflow.id, "release-hardening");
    assert_eq!(workflow.version, "1.0.0");
    assert_eq!(
        workflow.source_path,
        ".fkst/packages/release-hardening.json"
    );
    assert_eq!(workflow.steps.len(), 1);
    assert_eq!(workflow.steps[0].id, "verify");
    assert_eq!(workflow.steps[0].title, "Verify the release candidate");
    assert_eq!(workflow.steps[0].kind, StepKind::Static);
    assert!(view.rejected.is_empty());
    assert!(view.disqualified_ids.is_empty());
    assert!(!format!("{view:?}").contains("Inspect the release candidate"));
}
