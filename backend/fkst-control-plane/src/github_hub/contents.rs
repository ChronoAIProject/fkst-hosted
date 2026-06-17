//! GitHub Git Data API WRITER (#181): commit a set of files atomically onto a
//! repo's DEFAULT branch using a GitHub **App installation token**.
//!
//! WHY the App token (not [`crate::github_hub::GithubProxy`]): the seeded user
//! GitHub provider reached through NyxID is read-only; only the App installation
//! token holds `contents:write` (#110, `default_permissions`). So this writer
//! mints via [`GithubAppTokens::token_for_repo`] (the `None` perms select the
//! default set, which includes `contents:write`), exactly like the Contents READ
//! helper (`github_app::contents`) and `HttpIssueApi`.
//!
//! WHY the Git Data API (blobs → tree → commit → ref) and not N×
//! `PUT /contents/{path}`: the whole scaffold lands as ONE reviewable commit with
//! ONE ref update; there is never a half-written `.fkst/`, and we do not have to
//! thread the prior-file sha through a per-file PUT. The base tree is reused
//! (`base_tree`) so existing repo content is preserved — only the supplied paths
//! are added/overwritten.
//!
//! Security: the installation token is a [`SecretString`]; it is exposed ONLY at
//! the `Authorization: Bearer` header and NEVER logged. File contents are never
//! logged either — only the repo, the file COUNT, and per-file byte SIZES.

use base64::Engine as _;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::github_app::{GithubAppError, GithubAppTokens};

/// Request timeout for every Git Data API call (mirrors the App transport).
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The git blob mode for a regular, non-executable file.
const BLOB_MODE_FILE: &str = "100644";

/// A file to write into the repo, addressed by its repo-relative path.
///
/// `contents` is raw bytes — the writer base64-encodes them for the blob POST so
/// arbitrary (including binary) content round-trips losslessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldFile {
    pub path: String,
    pub contents: Vec<u8>,
}

/// Result of a successful multi-file commit: the new commit sha and the branch it
/// landed on (the repo's resolved default branch — never assumed `"main"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    pub commit_sha: String,
    pub default_branch: String,
}

/// Typed failures of [`commit_files`]. Credential-free: no token or file content
/// ever reaches a variant's payload.
#[derive(Debug, thiserror::Error)]
pub enum ContentsWriteError {
    /// The repo does not exist (or the App cannot see it): the repo GET 404'd.
    #[error("repository not found: {0}")]
    RepoNotFound(String),
    /// The App is not installed on the repo (the token mint surfaced
    /// `InstallationGone`/`NotInstalled`). Carries the install URL when known so
    /// the caller can render an actionable hint.
    #[error("github app not installed on {owner_repo}")]
    NotInstalled {
        owner_repo: String,
        install_url: Option<String>,
    },
    /// The ref update was rejected (409/422): a concurrent push moved the branch,
    /// or the ref was in an unexpected state. The scaffold was NOT applied.
    #[error("ref update conflict on {0}")]
    Conflict(String),
    /// Any other upstream failure (transport, auth, unexpected status/body). The
    /// string is client-safe — it carries no token and no file content.
    #[error("github upstream error: {0}")]
    Upstream(String),
}

/// Minimal shape of `GET /repos/{owner}/{repo}` — only the default branch matters.
#[derive(Debug, Deserialize)]
struct RepoMeta {
    default_branch: String,
}

/// Minimal shape of `GET /git/ref/heads/{branch}`: the object the ref points at.
#[derive(Debug, Deserialize)]
struct RefObject {
    object: GitObject,
}

#[derive(Debug, Deserialize)]
struct GitObject {
    sha: String,
}

/// Minimal shape of `GET /git/commits/{sha}`: its tree.
#[derive(Debug, Deserialize)]
struct CommitMeta {
    tree: TreeRef,
}

#[derive(Debug, Deserialize)]
struct TreeRef {
    sha: String,
}

/// Minimal shape of any create response that returns a `sha` (blob/tree/commit).
#[derive(Debug, Deserialize)]
struct ShaOnly {
    sha: String,
}

/// Commit `files` atomically onto the repo's DEFAULT branch using the GitHub App
/// installation token.
///
/// Resolves the default branch first (never assumes `"main"`), then walks the
/// Git Data API: base ref → base commit → base tree → one blob per file → a new
/// tree on the base tree → a new commit → a non-forced ref update. Returns the
/// new commit sha and the branch it landed on. See the module docs for the
/// rationale of the App-token + Git-Data-API design.
pub async fn commit_files(
    app: &GithubAppTokens,
    owner: &str,
    repo: &str,
    message: &str,
    files: &[ScaffoldFile],
) -> Result<CommitResult, ContentsWriteError> {
    let owner_repo = format!("{owner}/{repo}");

    // Mint ONCE via the existing installation-token path. `None` selects
    // `default_permissions()` (incl. `contents:write`). Install-lifecycle errors
    // map to the typed `NotInstalled`; everything else is upstream.
    let token = app
        .token_for_repo(&owner_repo, None)
        .await
        .map_err(|e| map_mint_error(&owner_repo, e))?;

    let client = build_client()?;
    let api_base = app.api_base();
    let writer = GitDataWriter {
        client,
        api_base,
        owner: owner.to_string(),
        repo: repo.to_string(),
        token,
    };

    let total_bytes: usize = files.iter().map(|f| f.contents.len()).sum();
    tracing::info!(
        owner_repo = %owner_repo,
        file_count = files.len(),
        total_bytes,
        "committing fkst scaffold via Git Data API"
    );

    writer.commit(message, files).await
}

/// Map a `token_for_repo` mint failure onto the writer's typed error. The
/// install-lifecycle variants become `NotInstalled` (carrying the install URL);
/// everything else is `Upstream` with the credential-free Display string.
fn map_mint_error(owner_repo: &str, err: GithubAppError) -> ContentsWriteError {
    match err {
        GithubAppError::NotInstalled { install_url, .. } => ContentsWriteError::NotInstalled {
            owner_repo: owner_repo.to_string(),
            install_url,
        },
        GithubAppError::InstallationGone { .. } => ContentsWriteError::NotInstalled {
            owner_repo: owner_repo.to_string(),
            install_url: None,
        },
        other => ContentsWriteError::Upstream(other.to_string()),
    }
}

/// Build the direct-`reqwest` transport, mirroring `HttpGithubApi`'s shape
/// (20s timeout, `fkst-hosted-api` UA).
fn build_client() -> Result<reqwest::Client, ContentsWriteError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("fkst-hosted-api")
        .build()
        .map_err(|e| ContentsWriteError::Upstream(format!("client build: {e}")))
}

/// The Git Data API writer bound to one repo + token. Built per call; cheap
/// relative to the network round-trips.
struct GitDataWriter {
    client: reqwest::Client,
    api_base: String,
    owner: String,
    repo: String,
    token: SecretString,
}

impl GitDataWriter {
    /// Run the full blobs → tree → commit → ref sequence.
    async fn commit(
        &self,
        message: &str,
        files: &[ScaffoldFile],
    ) -> Result<CommitResult, ContentsWriteError> {
        // 1. Resolve the default branch (NEVER assume "main").
        let default_branch = self.repo_default_branch().await?;
        // 2. Base commit sha from the branch ref.
        let base_sha = self.base_commit_sha(&default_branch).await?;
        // 3. Base tree sha from the base commit.
        let base_tree_sha = self.base_tree_sha(&base_sha).await?;
        // 4. One blob per file.
        let mut tree_entries = Vec::with_capacity(files.len());
        for file in files {
            let blob_sha = self.create_blob(&file.contents).await?;
            tree_entries.push(serde_json::json!({
                "path": file.path,
                "mode": BLOB_MODE_FILE,
                "type": "blob",
                "sha": blob_sha,
            }));
        }
        // 5. A new tree layered on the base tree (preserves existing content).
        let tree_sha = self.create_tree(&base_tree_sha, &tree_entries).await?;
        // 6. A new commit parented on the base commit.
        let commit_sha = self.create_commit(message, &tree_sha, &base_sha).await?;
        // 7. Move the branch ref to the new commit (no force).
        self.update_ref(&default_branch, &commit_sha).await?;

        tracing::info!(
            owner_repo = %format!("{}/{}", self.owner, self.repo),
            default_branch = %default_branch,
            "fkst scaffold committed"
        );

        Ok(CommitResult {
            commit_sha,
            default_branch,
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!(
            "{}/repos/{}/{}/{suffix}",
            self.api_base, self.owner, self.repo
        )
    }

    fn owner_repo(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// `GET /repos/{owner}/{repo}` → `default_branch`. A 404 is `RepoNotFound`.
    async fn repo_default_branch(&self) -> Result<String, ContentsWriteError> {
        let url = self.url("");
        // Trim the trailing slash the empty suffix leaves.
        let url = url.trim_end_matches('/').to_string();
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(|e| ContentsWriteError::Upstream(format!("repo get: {e}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ContentsWriteError::RepoNotFound(self.owner_repo()));
        }
        let meta: RepoMeta = self.decode(response, "repo get").await?;
        Ok(meta.default_branch)
    }

    /// `GET /git/ref/heads/{branch}` → base commit sha.
    async fn base_commit_sha(&self, branch: &str) -> Result<String, ContentsWriteError> {
        let response = self
            .authed_get(&self.url(&format!("git/ref/heads/{branch}")))
            .await?;
        let body: RefObject = self.decode(response, "ref get").await?;
        Ok(body.object.sha)
    }

    /// `GET /git/commits/{sha}` → base tree sha.
    async fn base_tree_sha(&self, commit_sha: &str) -> Result<String, ContentsWriteError> {
        let response = self
            .authed_get(&self.url(&format!("git/commits/{commit_sha}")))
            .await?;
        let body: CommitMeta = self.decode(response, "commit get").await?;
        Ok(body.tree.sha)
    }

    /// `POST /git/blobs` with `{content: base64(bytes), encoding: "base64"}`.
    async fn create_blob(&self, contents: &[u8]) -> Result<String, ContentsWriteError> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(contents);
        let body = serde_json::json!({ "content": encoded, "encoding": "base64" });
        let response = self.authed_post(&self.url("git/blobs"), &body).await?;
        let body: ShaOnly = self.decode(response, "blob create").await?;
        Ok(body.sha)
    }

    /// `POST /git/trees` with `base_tree` + the entries.
    async fn create_tree(
        &self,
        base_tree: &str,
        entries: &[serde_json::Value],
    ) -> Result<String, ContentsWriteError> {
        let body = serde_json::json!({ "base_tree": base_tree, "tree": entries });
        let response = self.authed_post(&self.url("git/trees"), &body).await?;
        let body: ShaOnly = self.decode(response, "tree create").await?;
        Ok(body.sha)
    }

    /// `POST /git/commits` with `{message, tree, parents:[base_sha]}`.
    async fn create_commit(
        &self,
        message: &str,
        tree: &str,
        base_sha: &str,
    ) -> Result<String, ContentsWriteError> {
        let body = serde_json::json!({
            "message": message,
            "tree": tree,
            "parents": [base_sha],
        });
        let response = self.authed_post(&self.url("git/commits"), &body).await?;
        let body: ShaOnly = self.decode(response, "commit create").await?;
        Ok(body.sha)
    }

    /// `PATCH /git/refs/heads/{branch}` with `{sha}` (no `force`). A 409/422 is a
    /// `Conflict` (the branch moved underneath us).
    async fn update_ref(&self, branch: &str, sha: &str) -> Result<(), ContentsWriteError> {
        let url = self.url(&format!("git/refs/heads/{branch}"));
        let body = serde_json::json!({ "sha": sha });
        let response = self
            .client
            .patch(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(self.token.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(|e| ContentsWriteError::Upstream(format!("ref patch: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT
            || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
        {
            return Err(ContentsWriteError::Conflict(format!(
                "{}@{branch}",
                self.owner_repo()
            )));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ContentsWriteError::Upstream(format!(
                "ref patch status {status}: {body}"
            )));
        }
        Ok(())
    }

    /// Authenticated GET that maps a non-success status to `Upstream` (callers
    /// that need 404 semantics — only the repo GET — handle it themselves first).
    async fn authed_get(&self, url: &str) -> Result<reqwest::Response, ContentsWriteError> {
        self.client
            .get(url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(|e| ContentsWriteError::Upstream(format!("get {url}: {e}")))
    }

    /// Authenticated JSON POST.
    async fn authed_post(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, ContentsWriteError> {
        self.client
            .post(url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(self.token.expose_secret())
            .json(body)
            .send()
            .await
            .map_err(|e| ContentsWriteError::Upstream(format!("post {url}: {e}")))
    }

    /// Status-before-body discipline: reject any non-success status as `Upstream`
    /// (the body may carry an upstream error message, never a token), then decode.
    async fn decode<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
        what: &str,
    ) -> Result<T, ContentsWriteError> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ContentsWriteError::Upstream(format!(
                "{what} status {status}: {body}"
            )));
        }
        response
            .json::<T>()
            .await
            .map_err(|e| ContentsWriteError::Upstream(format!("{what} body: {e}")))
    }
}

#[cfg(test)]
#[path = "contents_tests.rs"]
mod tests;
