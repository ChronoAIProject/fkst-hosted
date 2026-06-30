//! GitHub App Contents READ helper (#179): a minimal `GET /repos/{o}/{r}/
//! contents/{path}` over the App-installation token path.
//!
//! WHY this lives with the App layer: a per-repo Contents READ on an arbitrary
//! user repo must resolve the per-repo App installation and present an
//! INSTALLATION token (the App holds `contents:write` ⊇ read via
//! `default_permissions`), surfacing the typed
//! [`GithubAppError::NotInstalled { install_url }`] when the App is absent.
//!
//! Read-only: this never PUTs/DELETEs contents (write capability belongs to a
//! sibling scaffold issue). It mints via the existing
//! [`GithubAppTokens::token_for_repo`] path and mirrors the direct-`reqwest`
//! request/classify shape of `goals/repo_create.rs::create_repo` and the App
//! `api` module.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::api::reset_seconds;
use super::{GithubAppError, GithubAppTokens};

/// One entry from a `GET .../contents/{path}` response. GitHub returns a JSON
/// ARRAY of these for a directory and a single such OBJECT for a file; both are
/// normalized into [`ContentsListing`].
///
/// Tolerant: only the three fields the pre-flight needs are typed (`name`,
/// `path`, `type`); the rest of GitHub's content object (size, sha, git/html
/// URLs, base64 content) is ignored — the check is a pure existence/kind probe.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ContentsEntry {
    pub name: String,
    pub path: String,
    /// `"file"`, `"dir"`, `"symlink"`, or `"submodule"` (GitHub's `type`).
    #[serde(rename = "type")]
    pub kind: String,
}

impl ContentsEntry {
    /// True when this entry is a regular file (`type == "file"`).
    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }
}

/// Normalized result of a Contents READ: the entries the path resolves to.
///
/// - A DIRECTORY path yields its children (one [`ContentsEntry`] per child).
/// - A FILE path yields exactly one entry (the file itself).
///
/// A missing path is NOT represented here — it surfaces as
/// [`GithubAppError::NotFound`] from [`GithubAppTokens::get_contents`] so the
/// caller can distinguish "absent" from "present but empty".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsListing {
    /// Directory children, or the single-element file listing.
    pub entries: Vec<ContentsEntry>,
    /// True when the requested path itself resolved to a single FILE object
    /// (GitHub returned an object, not an array).
    pub is_file: bool,
}

impl ContentsListing {
    /// True when the requested path resolved to a single regular file.
    pub fn is_single_file(&self) -> bool {
        self.is_file && self.entries.first().is_some_and(ContentsEntry::is_file)
    }
}

/// Abstraction for "read repo contents via the App installation", injected so the
/// pre-flight package check (#179) is unit-testable against a fake without a live
/// GitHub. [`GithubAppTokens`] is the production implementation.
#[async_trait]
pub trait ContentsReader: Send + Sync {
    /// `GET /repos/{owner}/{repo}/contents/{path}` as the App installation.
    ///
    /// `owner_repo` is `"owner/name"`; `path` is the repo-relative path with NO
    /// leading slash. A 404 maps to [`GithubAppError::NotFound`]; the typed
    /// install-lifecycle errors ([`GithubAppError::NotInstalled`] /
    /// [`GithubAppError::InstallationGone`]) propagate UNCHANGED.
    async fn get_contents(
        &self,
        owner_repo: &str,
        path: &str,
    ) -> Result<ContentsListing, GithubAppError>;
}

#[async_trait]
impl ContentsReader for GithubAppTokens {
    async fn get_contents(
        &self,
        owner_repo: &str,
        path: &str,
    ) -> Result<ContentsListing, GithubAppError> {
        GithubAppTokens::get_contents(self, owner_repo, path).await
    }
}

impl GithubAppTokens {
    /// Read a repo path's contents via the App installation token.
    ///
    /// Mints (or reuses the cached) per-repo installation token through the
    /// existing [`Self::token_for_repo`] path, then `GET`s the Contents API.
    /// GitHub returns a JSON ARRAY for a directory and a JSON OBJECT for a file;
    /// both are normalized into [`ContentsListing`]. The typed
    /// [`GithubAppError::NotInstalled { install_url }`] /
    /// [`GithubAppError::InstallationGone`] from the mint propagate UNCHANGED so
    /// the caller can short-circuit to an install hint; a missing path is a
    /// [`GithubAppError::NotFound`].
    pub async fn get_contents(
        &self,
        owner_repo: &str,
        path: &str,
    ) -> Result<ContentsListing, GithubAppError> {
        // Mint via the existing installation-token path. `NotInstalled` /
        // `InstallationGone` propagate unchanged (never swallowed).
        let token = self.token_for_repo(owner_repo, None).await?;
        self.contents_client()?
            .fetch_contents(owner_repo, path, &token)
            .await
    }
}

/// Direct-`reqwest` Contents transport, mirroring `HttpGithubApi`'s shape
/// (injected `api_base`, 20s timeout, `fkst-hosted-api` UA). Built per call from
/// the service's configured API base; cheap relative to the network round-trip.
pub(super) struct ContentsHttp {
    api_base: String,
    client: reqwest::Client,
}

impl ContentsHttp {
    fn new(api_base: &str) -> Result<Self, GithubAppError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| GithubAppError::Http(format!("contents client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Perform the `GET .../contents/{path}` and classify the response, mirroring
    /// `repo_create.rs::create_repo`'s status-before-body discipline.
    async fn fetch_contents(
        &self,
        owner_repo: &str,
        path: &str,
        token: &SecretString,
    ) -> Result<ContentsListing, GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        // The path is repo-relative with no leading slash; it is composed by the
        // caller from validated package names, so it carries no traversal.
        let url = format!(
            "{}/repos/{owner}/{repo}/contents/{}",
            self.api_base,
            path.trim_start_matches('/')
        );

        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_contents: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::NotFound {
                owner_repo: owner_repo.to_string(),
                path: path.to_string(),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            // A 403 with rate-limit headers is a rate limit; otherwise an auth
            // failure (mirror the `api` module's disambiguation).
            if super::api::is_rate_limited(response.headers()) {
                return Err(GithubAppError::RateLimited(reset_seconds(
                    response.headers(),
                )));
            }
            return Err(GithubAppError::AppAuth);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "get_contents status {status}: {body}"
            )));
        }

        // A directory is a JSON array; a file is a JSON object. Decode the raw
        // value first, then normalize both shapes.
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_contents body: {e}")))?;
        normalize_contents(value)
    }
}

impl GithubAppTokens {
    /// Build the Contents transport against this service's configured API base
    /// (the same base the `api` token transport uses).
    fn contents_client(&self) -> Result<ContentsHttp, GithubAppError> {
        ContentsHttp::new(&self.api_base())
    }
}

/// Normalize a Contents API JSON value (array for a dir, object for a file) into
/// a [`ContentsListing`]. Any other shape is a malformed response.
fn normalize_contents(value: serde_json::Value) -> Result<ContentsListing, GithubAppError> {
    match value {
        serde_json::Value::Array(items) => {
            let entries: Vec<ContentsEntry> =
                serde_json::from_value(serde_json::Value::Array(items))
                    .map_err(|e| GithubAppError::Http(format!("get_contents dir decode: {e}")))?;
            Ok(ContentsListing {
                entries,
                is_file: false,
            })
        }
        obj @ serde_json::Value::Object(_) => {
            let entry: ContentsEntry = serde_json::from_value(obj)
                .map_err(|e| GithubAppError::Http(format!("get_contents file decode: {e}")))?;
            Ok(ContentsListing {
                entries: vec![entry],
                is_file: true,
            })
        }
        _ => Err(GithubAppError::Http(
            "get_contents: unexpected response shape (not array or object)".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_app::config::GithubAppConfig;
    use crate::github_app::GithubAppTokens;
    use secrecy::SecretString;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A config whose `api_base` points at the wiremock server, with a valid
    /// generated RSA key so the JWT mint succeeds.
    fn test_config(api_base: &str) -> GithubAppConfig {
        use rand::rngs::OsRng;
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        use rsa::RsaPrivateKey;
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
        let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
        GithubAppConfig {
            app_id: 42,
            private_key_pem: SecretString::from(pem.to_string()),
            app_slug: Some("fkst-test".to_string()),
            webhook_secret: None,
            api_base: api_base.to_string(),
        }
    }

    /// Stand up the installation-resolution + token-mint mocks the
    /// `token_for_repo` path needs, so a Contents test can exercise the GET.
    async fn mount_token_mint(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path_matcher("/repos/acme/site/installation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 7 })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path_matcher("/app/installations/7/access_tokens"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_contents_token",
                "expires_at": "2999-01-01T00:00:00Z"
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn directory_array_is_normalized_to_entries() {
        let server = MockServer::start().await;
        mount_token_mint(&server).await;
        Mock::given(method("GET"))
            .and(path_matcher(
                "/repos/acme/site/contents/.fkst/packages/demo",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "name": "departments", "path": ".fkst/packages/demo/departments", "type": "dir" },
                { "name": "README.md", "path": ".fkst/packages/demo/README.md", "type": "file" }
            ])))
            .mount(&server)
            .await;
        let svc = GithubAppTokens::new(&test_config(&server.uri())).expect("svc");

        let listing = svc
            .get_contents("acme/site", ".fkst/packages/demo")
            .await
            .expect("dir listing");
        assert!(
            !listing.is_file,
            "a directory must not be flagged as a file"
        );
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.entries[0].name, "departments");
        assert_eq!(listing.entries[0].kind, "dir");
        assert!(listing.entries[1].is_file());
    }

    #[tokio::test]
    async fn file_object_is_normalized_to_single_entry() {
        let server = MockServer::start().await;
        mount_token_mint(&server).await;
        Mock::given(method("GET"))
            .and(path_matcher(
                "/repos/acme/site/contents/.fkst/packages/demo/departments/demo/main.lua",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "name": "main.lua",
                "path": ".fkst/packages/demo/departments/demo/main.lua",
                "type": "file",
                "size": 42
            })))
            .mount(&server)
            .await;
        let svc = GithubAppTokens::new(&test_config(&server.uri())).expect("svc");

        let listing = svc
            .get_contents("acme/site", ".fkst/packages/demo/departments/demo/main.lua")
            .await
            .expect("file listing");
        assert!(listing.is_file, "a file object must be flagged as a file");
        assert!(listing.is_single_file());
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "main.lua");
    }

    #[tokio::test]
    async fn missing_path_is_not_found() {
        let server = MockServer::start().await;
        mount_token_mint(&server).await;
        Mock::given(method("GET"))
            .and(path_matcher(
                "/repos/acme/site/contents/.fkst/packages/ghost",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let svc = GithubAppTokens::new(&test_config(&server.uri())).expect("svc");

        let err = svc
            .get_contents("acme/site", ".fkst/packages/ghost")
            .await
            .expect_err("missing path");
        assert!(
            matches!(err, GithubAppError::NotFound { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn not_installed_propagates_unchanged() {
        // The installation lookup 404s → the mint surfaces NotInstalled, which
        // get_contents must propagate UNCHANGED (carrying the install URL).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/repos/acme/missing/installation"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let svc = GithubAppTokens::new(&test_config(&server.uri())).expect("svc");

        let err = svc
            .get_contents("acme/missing", ".fkst/packages/demo")
            .await
            .expect_err("not installed");
        match err {
            GithubAppError::NotInstalled { install_url, .. } => {
                assert!(
                    install_url.is_some_and(|u| u.contains("fkst-test")),
                    "install URL must be preserved"
                );
            }
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }

    #[test]
    fn normalize_rejects_a_scalar_shape() {
        let err = normalize_contents(serde_json::json!("not-an-object")).expect_err("scalar");
        assert!(matches!(err, GithubAppError::Http(_)), "got {err:?}");
    }
}
