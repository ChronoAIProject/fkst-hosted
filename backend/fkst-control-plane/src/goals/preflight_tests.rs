//! Unit tests for [`crate::goals::preflight`] (#179). Split into its own file
//! (referenced via `#[path]`) so `preflight.rs` stays under 500 lines —
//! mirroring the `ornn/client_tests.rs` split convention.
//!
//! Every test injects a FAKE [`ContentsReader`] and/or a fake [`OrnnTransport`]
//! so the checks are exercised without a live GitHub/NyxID/Ornn or a network.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use http_body_util::BodyExt;
use secrecy::SecretString;

use super::*;
use crate::github_app::{ContentsEntry, ContentsListing, ContentsReader, GithubAppError};
use crate::nyxid::ProxyResponse;
use crate::ornn::types::OrnnPinKind;
use crate::ornn::{OrnnClient, OrnnTransport};

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

/// A scripted Contents reader: keyed by repo-relative path, each entry is either
/// a directory listing, a single file, or a typed error to return.
struct FakeContents {
    /// `path -> outcome`. A path not in the map yields `NotFound`.
    replies: HashMap<String, ContentsOutcome>,
}

#[derive(Clone)]
enum ContentsOutcome {
    Dir(Vec<(&'static str, &'static str)>), // (name, type)
    File,
    NotInstalled(Option<String>),
}

impl FakeContents {
    fn new() -> Self {
        Self {
            replies: HashMap::new(),
        }
    }

    fn with(mut self, path: &str, outcome: ContentsOutcome) -> Self {
        self.replies.insert(path.to_string(), outcome);
        self
    }

    /// Lay down a fully-valid package layout for `name`.
    fn with_valid_package(self, name: &str) -> Self {
        self.with(
            &format!(".fkst/packages/{name}"),
            ContentsOutcome::Dir(vec![("departments", "dir")]),
        )
        .with(
            &format!(".fkst/packages/{name}/departments/{name}/main.lua"),
            ContentsOutcome::File,
        )
    }
}

#[async_trait]
impl ContentsReader for FakeContents {
    async fn get_contents(
        &self,
        _owner_repo: &str,
        path: &str,
    ) -> Result<ContentsListing, GithubAppError> {
        match self.replies.get(path) {
            Some(ContentsOutcome::Dir(children)) => Ok(ContentsListing {
                entries: children
                    .iter()
                    .map(|(n, t)| ContentsEntry {
                        name: n.to_string(),
                        path: format!("{path}/{n}"),
                        kind: t.to_string(),
                    })
                    .collect(),
                is_file: false,
            }),
            Some(ContentsOutcome::File) => Ok(ContentsListing {
                entries: vec![ContentsEntry {
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    path: path.to_string(),
                    kind: "file".to_string(),
                }],
                is_file: true,
            }),
            Some(ContentsOutcome::NotInstalled(url)) => Err(GithubAppError::NotInstalled {
                owner_repo: "acme/site".to_string(),
                install_url: url.clone(),
            }),
            None => Err(GithubAppError::NotFound {
                owner_repo: "acme/site".to_string(),
                path: path.to_string(),
            }),
        }
    }
}

/// Scripted Ornn transport (mirrors the `ornn` module fakes): FIFO replies keyed
/// by a path substring.
struct FakeOrnn {
    proxy: Mutex<Vec<(String, u16, serde_json::Value)>>,
}

impl FakeOrnn {
    fn new() -> Self {
        Self {
            proxy: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, needle: &str, status: u16, body: serde_json::Value) {
        self.proxy
            .lock()
            .unwrap()
            .push((needle.to_string(), status, body));
    }
}

#[async_trait]
impl OrnnTransport for FakeOrnn {
    async fn proxy_get(
        &self,
        path: &str,
        _query: &[(&str, &str)],
        _user_token: &SecretString,
    ) -> Result<ProxyResponse, crate::error::AppError> {
        let mut queue = self.proxy.lock().unwrap();
        let idx = queue
            .iter()
            .position(|(needle, _, _)| path.contains(needle.as_str()))
            .unwrap_or_else(|| panic!("no fake ornn reply for {path}"));
        let (_, status, body) = queue.remove(idx);
        Ok(ProxyResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers: reqwest::header::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn download_direct(&self, _url: &str) -> Result<Vec<u8>, crate::error::AppError> {
        unreachable!("preflight never downloads packages")
    }
}

fn skill(name: &str, version: &str) -> OrnnSkillPin {
    OrnnSkillPin {
        kind: OrnnPinKind::Skill,
        name: name.to_string(),
        version: version.to_string(),
    }
}

fn token() -> SecretString {
    SecretString::from("user_tok".to_string())
}

// ---------------------------------------------------------------------------
// SubmissionErrors / 422 body (commit 2)
// ---------------------------------------------------------------------------

#[test]
fn submission_errors_is_empty_only_when_all_classes_empty() {
    assert!(SubmissionErrors::default().is_empty());
    let mut e = SubmissionErrors::default();
    e.packages.push(PackageError {
        name: "p".into(),
        reason: "r".into(),
    });
    assert!(!e.is_empty());
}

#[tokio::test]
async fn populated_submission_errors_render_422_with_every_entry() {
    use axum::response::IntoResponse;
    let errors = SubmissionErrors {
        issue_format: vec![FieldError {
            field: "goal".into(),
            message: "missing".into(),
        }],
        packages: vec![PackageError {
            name: "alpha".into(),
            reason: "missing .fkst/packages/alpha".into(),
        }],
        ornn: vec![PinError {
            kind: "skill".into(),
            name: "ghost".into(),
            version: "1.0".into(),
            reason: "not found".into(),
        }],
    };
    let response = errors.into_response();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(body["error"], "submission_invalid");
    // Every class/entry is enumerated in the body.
    assert_eq!(body["issue_format"][0]["field"], "goal");
    assert_eq!(body["packages"][0]["name"], "alpha");
    assert_eq!(body["ornn"][0]["name"], "ghost");
    assert!(body["message"].as_str().unwrap().contains("3 error"));
}

// ---------------------------------------------------------------------------
// Package correctness (commit 3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn package_missing_dir_pushes_error() {
    // No replies → the dir read 404s.
    let reader = FakeContents::new();
    let errors = check_packages(&reader, "acme/site", &["alpha".to_string()])
        .await
        .expect("no abort");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].name, "alpha");
    assert!(errors[0].reason.contains("missing .fkst/packages/alpha"));
}

#[tokio::test]
async fn package_dir_without_main_lua_pushes_error() {
    // The dir exists but the entry-file read 404s.
    let reader = FakeContents::new().with(
        ".fkst/packages/alpha",
        ContentsOutcome::Dir(vec![("README.md", "file")]),
    );
    let errors = check_packages(&reader, "acme/site", &["alpha".to_string()])
        .await
        .expect("no abort");
    assert_eq!(errors.len(), 1);
    assert!(errors[0]
        .reason
        .contains("missing entry file departments/alpha/main.lua"));
}

#[tokio::test]
async fn package_dir_with_main_lua_as_dir_pushes_error() {
    // The entry path resolves but is a DIRECTORY, not a file.
    let reader = FakeContents::new()
        .with(
            ".fkst/packages/alpha",
            ContentsOutcome::Dir(vec![("departments", "dir")]),
        )
        .with(
            ".fkst/packages/alpha/departments/alpha/main.lua",
            ContentsOutcome::Dir(vec![]),
        );
    let errors = check_packages(&reader, "acme/site", &["alpha".to_string()])
        .await
        .expect("no abort");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].reason.contains("missing entry file"));
}

#[tokio::test]
async fn fully_valid_package_passes() {
    let reader = FakeContents::new().with_valid_package("alpha");
    let errors = check_packages(&reader, "acme/site", &["alpha".to_string()])
        .await
        .expect("no abort");
    assert!(errors.is_empty(), "valid package must pass: {errors:?}");
}

#[tokio::test]
async fn invalid_and_reserved_names_are_rejected_without_a_read() {
    let reader = FakeContents::new();
    let errors = check_packages(
        &reader,
        "acme/site",
        &["bad/name".to_string(), "host".to_string()],
    )
    .await
    .expect("no abort");
    assert_eq!(errors.len(), 2);
    assert!(errors
        .iter()
        .any(|e| e.reason.contains("invalid package name")));
    assert!(errors.iter().any(|e| e.reason.contains("reserved")));
}

#[tokio::test]
async fn app_not_installed_short_circuits_to_install_blocked() {
    let reader = FakeContents::new().with(
        ".fkst/packages/alpha",
        ContentsOutcome::NotInstalled(Some(
            "https://github.com/apps/fkst-test/installations/new".to_string(),
        )),
    );
    let abort = check_packages(&reader, "acme/site", &["alpha".to_string()])
        .await
        .expect_err("install-blocked must abort");
    match abort {
        PackagePreflightAbort::InstallBlocked(reason) => {
            assert!(reason.contains("not installed"), "reason: {reason}");
            assert!(
                reason.contains("fkst-test"),
                "must reuse install URL: {reason}"
            );
        }
    }
}

#[tokio::test]
async fn multiple_bad_packages_all_reported_never_first_fail() {
    // Two bad packages + one good → BOTH bad ones reported (no first-fail).
    let reader = FakeContents::new().with_valid_package("good").with(
        ".fkst/packages/half",
        ContentsOutcome::Dir(vec![("x", "file")]),
    );
    // `missing` has no replies → missing dir. `half` → missing main.lua.
    let errors = check_packages(
        &reader,
        "acme/site",
        &[
            "good".to_string(),
            "missing".to_string(),
            "half".to_string(),
        ],
    )
    .await
    .expect("no abort");
    assert_eq!(errors.len(), 2, "both faulty packages reported: {errors:?}");
    let names: Vec<&str> = errors.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"missing"));
    assert!(names.contains(&"half"));
}

// (The Ornn-availability cases live with the code they exercise, in
// `preflight_ornn_tests.rs`; here we cover the SubmissionErrors/422 shape, the
// package-correctness check, and the cross-class `validate_submission`
// aggregation.)

fn ornn_client(fake: FakeOrnn) -> OrnnClient {
    OrnnClient::new(std::sync::Arc::new(fake))
}

// ---------------------------------------------------------------------------
// validate_submission aggregation (commit 5)
// ---------------------------------------------------------------------------

fn repo() -> fkst_shared::models::RepoRef {
    fkst_shared::models::RepoRef {
        owner: "acme".into(),
        name: "site".into(),
    }
}

/// A bad package AND a bad pin AND a seeded issue-format error all surface in ONE
/// `SubmissionErrors` — proving the validator never first-fails across classes.
/// Drives the injectable core with a fake `ContentsReader` + fake Ornn client.
#[tokio::test]
async fn validate_submission_aggregates_all_classes_no_first_fail() {
    // Package class: `alpha` has no `.fkst/packages/alpha` (missing dir).
    let reader = FakeContents::new();
    // Ornn class: `ghost` does not exist in the catalog.
    let fake = FakeOrnn::new();
    fake.push("/skills/ghost/versions", 404, serde_json::json!({}));
    let client = ornn_client(fake);

    let issue_format = vec![FieldError {
        field: "goal".into(),
        message: "the `### Goal` section is required".into(),
    }];

    let err = validate_submission_with(
        &reader,
        Some(&token()),
        &repo(),
        &["alpha".to_string()],
        &[skill("ghost", "1.0")],
        issue_format,
        &client,
    )
    .await
    .expect_err("a bad package + bad pin + issue error must fail");

    // All three classes are present in the SINGLE aggregated result.
    assert_eq!(err.issue_format.len(), 1, "issue-format seeded");
    assert_eq!(err.packages.len(), 1, "package fault reported");
    assert_eq!(err.ornn.len(), 1, "ornn fault reported");
    assert_eq!(err.packages[0].name, "alpha");
    assert_eq!(err.ornn[0].name, "ghost");
}

#[tokio::test]
async fn validate_submission_passes_when_everything_resolves() {
    let reader = FakeContents::new().with_valid_package("alpha");
    let fake = FakeOrnn::new();
    fake.push(
        "/skills/fmt/versions",
        200,
        serde_json::json!({ "data": { "items": [ { "version": "2.0" } ] } }),
    );
    let client = ornn_client(fake);

    validate_submission_with(
        &reader,
        Some(&token()),
        &repo(),
        &["alpha".to_string()],
        &[skill("fmt", "2.0")],
        Vec::new(),
        &client,
    )
    .await
    .expect("a fully-valid submission must pass");
}
