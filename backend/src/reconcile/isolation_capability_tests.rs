//! wiremock tests for the session-isolation capability pre-flight: a tree carrying
//! the rule passes, a tree shipping `devloop` WITHOUT it is refused, a tree with no
//! `devloop` at all passes, and a non-404 error is reported rather than swallowed.

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn pkg(owner: &str, repo: &str, git_ref: &str, path_: &str) -> PackageRef {
    PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path_.to_string(),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().expect("client")
}

/// Mount `GET /repos/{owner}/{repo}/contents/libraries/devloop/claims.lua?ref=..`
/// answering `status` with `body` for one tree.
async fn mount_tree(
    server: &MockServer,
    owner: &str,
    repo: &str,
    git_ref: &str,
    status: u16,
    body: &str,
) {
    let p = format!("/repos/{owner}/{repo}/contents/libraries/devloop/claims.lua");
    Mock::given(method("GET"))
        .and(path(p))
        .and(query_param("ref", git_ref))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
        .mount(server)
        .await;
}

const WITH_RULE: &str = "function C.issue_owned_by_session(assignees, exec)\n  return true\nend\n";
const WITHOUT_RULE: &str = "function C.is_routed_to_session(a, c)\n  return false\nend\n";

#[tokio::test]
async fn tree_carrying_the_rule_passes() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "r", "main", 200, WITH_RULE).await;
    let refs = vec![pkg("o", "r", "main", "packages/p")];
    assert!(
        check_isolation_capability(&refs, &client(), &server.uri(), None)
            .await
            .is_ok()
    );
}

/// The prod failure: an older tree ships `devloop` without the gate, so its session
/// would act on issues assigned to other creators.
#[tokio::test]
async fn tree_shipping_devloop_without_the_rule_is_refused() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "old", "legacy", 200, WITHOUT_RULE).await;
    let refs = vec![pkg("o", "old", "legacy", "packages/p")];
    let err = check_isolation_capability(&refs, &client(), &server.uri(), None)
        .await
        .expect_err("must refuse");
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].0, "o/old@legacy");
    assert!(err[0].1.contains("issue_owned_by_session"), "{}", err[0].1);
}

/// A tree with no devloop library contributes no ungated entry point.
#[tokio::test]
async fn tree_without_a_devloop_library_passes() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "r", "main", 404, "").await;
    let refs = vec![pkg("o", "r", "main", "packages/p")];
    assert!(
        check_isolation_capability(&refs, &client(), &server.uri(), None)
            .await
            .is_ok()
    );
}

/// A transient GitHub error must stay distinguishable from a genuine miss — it may
/// not silently pass.
#[tokio::test]
async fn unexpected_status_is_reported_not_swallowed() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "r", "main", 500, "").await;
    let refs = vec![pkg("o", "r", "main", "packages/p")];
    let err = check_isolation_capability(&refs, &client(), &server.uri(), None)
        .await
        .expect_err("must report");
    assert!(err[0].1.contains("unexpected status"), "{}", err[0].1);
}

/// Libraries are repo-level: many packages from one tree cost ONE probe, and the
/// tree is named once rather than once per package.
#[tokio::test]
async fn packages_from_one_tree_are_probed_once() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "old", "legacy", 200, WITHOUT_RULE).await;
    let refs = vec![
        pkg("o", "old", "legacy", "packages/a"),
        pkg("o", "old", "legacy", "packages/b"),
        pkg("o", "old", "legacy", "packages/c"),
    ];
    let err = check_isolation_capability(&refs, &client(), &server.uri(), None)
        .await
        .expect_err("must refuse");
    assert_eq!(err.len(), 1, "one finding per tree, not per package");
    assert_eq!(server.received_requests().await.expect("requests").len(), 1);
}

/// Every offending tree is reported, not just the first.
#[tokio::test]
async fn all_offending_trees_are_collected() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "good", "main", 200, WITH_RULE).await;
    mount_tree(&server, "o", "bad1", "main", 200, WITHOUT_RULE).await;
    mount_tree(&server, "o", "bad2", "main", 200, WITHOUT_RULE).await;
    let refs = vec![
        pkg("o", "good", "main", "packages/p"),
        pkg("o", "bad1", "main", "packages/p"),
        pkg("o", "bad2", "main", "packages/p"),
    ];
    let err = check_isolation_capability(&refs, &client(), &server.uri(), None)
        .await
        .expect_err("must refuse");
    let names: Vec<&str> = err.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(names, vec!["o/bad1@main", "o/bad2@main"]);
}

/// The same repo at two refs is two trees: one may carry the rule and the other not.
#[tokio::test]
async fn distinct_refs_of_one_repo_are_probed_separately() {
    let server = MockServer::start().await;
    mount_tree(&server, "o", "r", "new", 200, WITH_RULE).await;
    mount_tree(&server, "o", "r", "old", 200, WITHOUT_RULE).await;
    let refs = vec![
        pkg("o", "r", "new", "packages/a"),
        pkg("o", "r", "old", "packages/b"),
    ];
    let err = check_isolation_capability(&refs, &client(), &server.uri(), None)
        .await
        .expect_err("must refuse");
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].0, "o/r@old");
}
