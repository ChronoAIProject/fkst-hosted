//! wiremock tests for the package-ref reachability pre-flight: all-reachable is
//! `Ok`, an unreachable (404) ref is named in the `Err`, and a mix reports only
//! the bad refs.

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

/// Mount a `GET /repos/{owner}/{repo}/contents/{path}/fkst.toml?ref=..` that
/// answers `status` for one ref.
async fn mount(server: &MockServer, r: &PackageRef, status: u16) {
    let p = format!(
        "/repos/{}/{}/contents/{}/fkst.toml",
        r.owner, r.repo, r.path
    );
    Mock::given(method("GET"))
        .and(path(p))
        .and(query_param("ref", r.git_ref.as_str()))
        .respond_with(ResponseTemplate::new(status))
        .mount(server)
        .await;
}

#[tokio::test]
async fn all_reachable_is_ok() {
    let server = MockServer::start().await;
    let a = pkg("acme", "pkgs", "dev", "packages/a");
    let b = pkg("acme", "pkgs", "main", "packages/b");
    mount(&server, &a, 200).await;
    mount(&server, &b, 200).await;

    let refs = vec![a, b];
    check_reachable(&refs, &client(), &server.uri())
        .await
        .expect("all reachable");
}

#[tokio::test]
async fn a_404_ref_is_named_in_the_error() {
    let server = MockServer::start().await;
    let bad = pkg("acme", "gone", "dev", "packages/x");
    mount(&server, &bad, 404).await;

    let refs = vec![bad.clone()];
    let failures = check_reachable(&refs, &client(), &server.uri())
        .await
        .expect_err("must fail");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, render_ref(&bad), "names the bad ref");
    assert!(
        failures[0].1.contains("not reachable"),
        "reason: {}",
        failures[0].1
    );
}

#[tokio::test]
async fn mixed_reports_only_the_unreachable() {
    let server = MockServer::start().await;
    let good = pkg("acme", "pkgs", "dev", "packages/good");
    let bad = pkg("acme", "pkgs", "dev", "packages/bad");
    mount(&server, &good, 200).await;
    mount(&server, &bad, 404).await;

    let refs = vec![good, bad.clone()];
    let failures = check_reachable(&refs, &client(), &server.uri())
        .await
        .expect_err("one is unreachable");
    assert_eq!(failures.len(), 1, "only the bad ref is reported");
    assert_eq!(failures[0].0, render_ref(&bad));
}

#[tokio::test]
async fn a_non_404_status_is_reported_verbatim() {
    let server = MockServer::start().await;
    let r = pkg("acme", "pkgs", "dev", "packages/a");
    mount(&server, &r, 500).await;

    let refs = vec![r];
    let failures = check_reachable(&refs, &client(), &server.uri())
        .await
        .expect_err("500 is a failure");
    assert!(
        failures[0].1.contains("unexpected status"),
        "reason: {}",
        failures[0].1
    );
}
