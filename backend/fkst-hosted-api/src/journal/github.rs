//! Thin GitHub Contents-API client for the per-run progress record, plus the
//! (dormant) issue-comment mirror.
//!
//! Design rules:
//! - The token lives in a [`secrecy::SecretString`]: it is exposed only at
//!   request-build time, never captured into `Debug`/`Display` of the repo,
//!   any error variant, or any log line (asserted by tests below).
//! - All write concurrency is optimistic: `get_record` captures the blob
//!   `sha`, `put_record` supplies it; a mismatch surfaces as
//!   [`JournalError::CasConflict`] for the caller's CAS loop.
//! - 403 is disambiguated: rate-limit headers mean "respect the reset and
//!   retry" ([`JournalError::GithubRateLimited`]); otherwise it is an auth
//!   failure ([`JournalError::GithubAuth`]), like 401.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};

use crate::journal::model::{ProgressRecord, PROGRESS_RECORD_SCHEMA};
use crate::journal::JournalError;

/// Default GitHub REST API base (overridable for tests / GHE).
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// Request timeout for every GitHub call.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// A Contents-API blob sha used for optimistic concurrency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSha(pub String);

/// Outcome of reading the remote progress record.
#[derive(Debug, Clone)]
pub enum RemoteRecord {
    /// Parsed, schema-supported record.
    Valid {
        record: ProgressRecord,
        sha: FileSha,
    },
    /// Present but unparseable / structurally wrong: never overwrite blindly.
    Corrupt { sha: FileSha },
    /// Present with a schema other than ours (forward-compat guard): refuse
    /// to write.
    NewerSchema { schema: String, sha: FileSha },
}

/// GitHub Contents-API client bound to one `owner/name` repo + branch.
pub struct ProgressRepo {
    api_base: String,
    repo: String,
    branch: String,
    token: Option<SecretString>,
    client: reqwest::Client,
}

// Hand-written: the token must never appear in any Debug rendering.
impl fmt::Debug for ProgressRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressRepo")
            .field("api_base", &self.api_base)
            .field("repo", &self.repo)
            .field("branch", &self.branch)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Reduce a reqwest error to a credential-free string (reqwest never embeds
/// request headers in its messages; this keeps that invariant explicit).
fn http_err(context: &str, err: reqwest::Error) -> JournalError {
    JournalError::Http(format!("{context}: {err}"))
}

/// Seconds until the rate-limit reset, from `retry-after` (delta seconds) or
/// `x-ratelimit-reset` (epoch seconds). Defaults to 60s when unparseable.
///
/// Crate-visible so the github-hub upstream classifier reuses the same
/// header parsing as the journal client.
pub(crate) fn reset_seconds(headers: &HeaderMap) -> u64 {
    if let Some(retry_after) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return retry_after;
    }
    if let Some(reset_epoch) = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return reset_epoch.saturating_sub(now);
    }
    60
}

/// True when a 403 carries rate-limit evidence (exhausted quota or an
/// explicit retry hint) rather than an auth refusal.
///
/// Crate-visible so the github-hub upstream classifier reuses the same
/// rate-limit detection as the journal client.
pub(crate) fn is_rate_limited(headers: &HeaderMap) -> bool {
    let remaining_zero = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "0")
        .unwrap_or(false);
    remaining_zero || headers.contains_key("retry-after")
}

/// Map auth/rate-limit statuses to their dedicated variants; `None` for
/// everything else.
fn classify_status(status: StatusCode, headers: &HeaderMap) -> Option<JournalError> {
    match status {
        StatusCode::UNAUTHORIZED => Some(JournalError::GithubAuth),
        StatusCode::FORBIDDEN => {
            if is_rate_limited(headers) {
                Some(JournalError::GithubRateLimited(reset_seconds(headers)))
            } else {
                Some(JournalError::GithubAuth)
            }
        }
        _ => None,
    }
}

impl ProgressRepo {
    /// Build a client for `repo` (`owner/name`) on `branch`, talking to
    /// `api_base` (default [`DEFAULT_API_BASE`]; overridable for tests).
    /// `token` is optional only so read paths can degrade; write paths
    /// against the real API require it.
    pub fn new(
        api_base: &str,
        repo: &str,
        branch: &str,
        token: Option<SecretString>,
    ) -> Result<Self, JournalError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| http_err("client build", e))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            token,
            client,
        })
    }

    /// The configured branch.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// The configured `owner/name`.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    fn contents_url(&self, path: &str) -> String {
        format!("{}/repos/{}/contents/{}", self.api_base, self.repo, path)
    }

    /// Apply shared headers (Accept + optional bearer token) to a request.
    fn decorate(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("accept", "application/vnd.github+json");
        match &self.token {
            Some(token) => request.bearer_auth(token.expose_secret()),
            None => request,
        }
    }

    /// GET the progress record at `path` on the configured branch.
    /// `Ok(None)` on 404 (no record yet — a fresh logical run).
    pub async fn get_record(&self, path: &str) -> Result<Option<RemoteRecord>, JournalError> {
        let response = self
            .decorate(self.client.get(self.contents_url(path)))
            .query(&[("ref", self.branch.as_str())])
            .send()
            .await
            .map_err(|e| http_err("contents GET", e))?;

        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if let Some(err) = classify_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            return Err(JournalError::Http(format!("contents GET status {status}")));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| http_err("contents GET body", e))?;
        let sha = FileSha(
            body.get("sha")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JournalError::Http("contents GET: missing sha".to_string()))?
                .to_string(),
        );
        // The Contents API base64-encodes with embedded newlines.
        let encoded: String = body
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let Ok(raw) = STANDARD.decode(&encoded) else {
            return Ok(Some(RemoteRecord::Corrupt { sha }));
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
            return Ok(Some(RemoteRecord::Corrupt { sha }));
        };
        let schema = value
            .get("schema")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if schema != PROGRESS_RECORD_SCHEMA {
            return Ok(Some(RemoteRecord::NewerSchema { schema, sha }));
        }
        match serde_json::from_value::<ProgressRecord>(value) {
            Ok(record) => Ok(Some(RemoteRecord::Valid { record, sha })),
            Err(_) => Ok(Some(RemoteRecord::Corrupt { sha })),
        }
    }

    /// PUT the record at `path`, supplying the prior blob `sha` for CAS
    /// (`None` creates the file). Returns the new blob sha.
    ///
    /// CAS losses (409, and 422 from a concurrent create / sha mismatch)
    /// surface as [`JournalError::CasConflict`]; a 404 on the update path
    /// (file deleted mid-run) surfaces as [`JournalError::RemoteMissing`]
    /// so the caller can fall back to create.
    pub async fn put_record(
        &self,
        path: &str,
        record: &ProgressRecord,
        prev: Option<&FileSha>,
        message: &str,
    ) -> Result<FileSha, JournalError> {
        let content = serde_json::to_vec_pretty(record)
            .map_err(|e| JournalError::Http(format!("record serialize: {e}")))?;
        let mut body = serde_json::json!({
            "message": message,
            "content": STANDARD.encode(content),
            "branch": self.branch,
        });
        if let Some(FileSha(sha)) = prev {
            body["sha"] = serde_json::Value::String(sha.clone());
        }

        let response = self
            .decorate(self.client.put(self.contents_url(path)))
            .json(&body)
            .send()
            .await
            .map_err(|e| http_err("contents PUT", e))?;

        let status = response.status();
        match status {
            StatusCode::OK | StatusCode::CREATED => {
                let body: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| http_err("contents PUT body", e))?;
                let sha = body
                    .pointer("/content/sha")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        JournalError::Http("contents PUT: missing content.sha".to_string())
                    })?;
                Ok(FileSha(sha.to_string()))
            }
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY => {
                Err(JournalError::CasConflict)
            }
            StatusCode::NOT_FOUND if prev.is_some() => Err(JournalError::RemoteMissing),
            _ => {
                if let Some(err) = classify_status(status, response.headers()) {
                    return Err(err);
                }
                Err(JournalError::Http(format!("contents PUT status {status}")))
            }
        }
    }

    /// Create or update the human-readable summary comment on `issue`.
    /// Returns the comment id. Dormant in v1: callers gate this on
    /// `FKST_JOURNAL_ISSUE_COMMENTS` and a known issue number.
    pub async fn upsert_issue_comment(
        &self,
        issue: u64,
        comment_id: Option<u64>,
        body: &str,
    ) -> Result<u64, JournalError> {
        if let Some(id) = comment_id {
            let url = format!("{}/repos/{}/issues/comments/{id}", self.api_base, self.repo);
            let response = self
                .decorate(self.client.patch(url))
                .json(&serde_json::json!({ "body": body }))
                .send()
                .await
                .map_err(|e| http_err("comment PATCH", e))?;
            let status = response.status();
            if status.is_success() {
                return Ok(id);
            }
            if status != StatusCode::NOT_FOUND {
                if let Some(err) = classify_status(status, response.headers()) {
                    return Err(err);
                }
                return Err(JournalError::Http(format!("comment PATCH status {status}")));
            }
            // 404: the comment vanished — fall through to create.
        }

        let url = format!(
            "{}/repos/{}/issues/{issue}/comments",
            self.api_base, self.repo
        );
        let response = self
            .decorate(self.client.post(url))
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| http_err("comment POST", e))?;
        let status = response.status();
        if let Some(err) = classify_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            return Err(JournalError::Http(format!("comment POST status {status}")));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| http_err("comment POST body", e))?;
        body.get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| JournalError::Http("comment POST: missing id".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::journal::model::UNVERIFIED_SHA;

    const TOKEN: &str = "ghp_supersecret_token_value_1234567890";

    fn repo(server_uri: &str, with_token: bool) -> ProgressRepo {
        let token = with_token.then(|| SecretString::from(TOKEN.to_string()));
        ProgressRepo::new(server_uri, "owner/name", "main", token).expect("client")
    }

    fn sample_record() -> ProgressRecord {
        ProgressRecord::new("rk", "demo", "fp", "2026-06-10T00:00:00Z".to_string())
    }

    /// Contents-API GET body for a record (base64 with embedded newlines,
    /// exactly as GitHub serves it).
    fn contents_body(record: &ProgressRecord, sha: &str) -> serde_json::Value {
        let encoded = STANDARD.encode(serde_json::to_vec(record).expect("json"));
        let wrapped: String = encoded
            .as_bytes()
            .chunks(60)
            .map(|chunk| format!("{}\n", String::from_utf8_lossy(chunk)))
            .collect();
        serde_json::json!({ "content": wrapped, "sha": sha, "encoding": "base64" })
    }

    // ---- get_record ---------------------------------------------------------

    #[tokio::test]
    async fn get_record_returns_none_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/repos/owner/name/contents/.fkst-hosted/journal/rk.json",
            ))
            .and(query_param("ref", "main"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let result = repo(&server.uri(), true)
            .get_record(".fkst-hosted/journal/rk.json")
            .await
            .expect("404 is Ok(None)");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_record_parses_a_valid_record_and_captures_the_sha() {
        let server = MockServer::start().await;
        let record = sample_record();
        Mock::given(method("GET"))
            .and(path("/repos/owner/name/contents/j.json"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&record, "abc")))
            .mount(&server)
            .await;
        match repo(&server.uri(), true)
            .get_record("j.json")
            .await
            .expect("ok")
        {
            Some(RemoteRecord::Valid {
                record: parsed,
                sha,
            }) => {
                assert_eq!(parsed, record);
                assert_eq!(sha, FileSha("abc".to_string()));
            }
            other => panic!("expected Valid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_record_flags_corrupt_and_newer_schema_remotes() {
        let server = MockServer::start().await;
        let corrupt = serde_json::json!({
            "content": STANDARD.encode(b"not json"), "sha": "c1"
        });
        Mock::given(method("GET"))
            .and(path("/repos/owner/name/contents/corrupt.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(corrupt))
            .mount(&server)
            .await;
        let mut newer = sample_record();
        newer.schema = "fkst-hosted/progress-record@2".to_string();
        Mock::given(method("GET"))
            .and(path("/repos/owner/name/contents/newer.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(contents_body(&newer, "c2")))
            .mount(&server)
            .await;

        let repo = repo(&server.uri(), true);
        assert!(matches!(
            repo.get_record("corrupt.json").await.expect("ok"),
            Some(RemoteRecord::Corrupt { .. })
        ));
        match repo.get_record("newer.json").await.expect("ok") {
            Some(RemoteRecord::NewerSchema { schema, .. }) => {
                assert_eq!(schema, "fkst-hosted/progress-record@2");
            }
            other => panic!("expected NewerSchema, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_record_network_failure_is_an_http_error() {
        // A closed port: connection refused.
        let repo = ProgressRepo::new(
            "http://127.0.0.1:1",
            "owner/name",
            "main",
            Some(SecretString::from(TOKEN.to_string())),
        )
        .expect("client");
        let err = repo.get_record("j.json").await.expect_err("must fail");
        assert!(matches!(err, JournalError::Http(_)), "got {err:?}");
    }

    // ---- put_record ------------------------------------------------------------

    #[tokio::test]
    async fn put_record_sends_the_prior_sha_and_returns_the_new_sha() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/owner/name/contents/j.json"))
            .and(body_partial_json(
                serde_json::json!({ "sha": "prev", "branch": "main" }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "content": { "sha": "next" } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let sha = repo(&server.uri(), true)
            .put_record(
                "j.json",
                &sample_record(),
                Some(&FileSha("prev".to_string())),
                "journal",
            )
            .await
            .expect("put ok");
        assert_eq!(sha, FileSha("next".to_string()));
    }

    #[tokio::test]
    async fn put_record_create_path_omits_the_sha() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/owner/name/contents/j.json"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(serde_json::json!({ "content": { "sha": "created" } })),
            )
            .mount(&server)
            .await;
        let sha = repo(&server.uri(), true)
            .put_record("j.json", &sample_record(), None, "journal")
            .await
            .expect("create ok");
        assert_eq!(sha, FileSha("created".to_string()));
    }

    #[tokio::test]
    async fn put_record_conflict_and_422_surface_as_cas_conflict() {
        for status in [409, 422] {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let err = repo(&server.uri(), true)
                .put_record(
                    "j.json",
                    &sample_record(),
                    Some(&FileSha("prev".to_string())),
                    "journal",
                )
                .await
                .expect_err("conflict must fail");
            assert!(
                matches!(err, JournalError::CasConflict),
                "status {status}: got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn put_record_404_on_update_is_remote_missing() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err = repo(&server.uri(), true)
            .put_record(
                "j.json",
                &sample_record(),
                Some(&FileSha("prev".to_string())),
                "journal",
            )
            .await
            .expect_err("404 must fail");
        assert!(matches!(err, JournalError::RemoteMissing), "got {err:?}");
    }

    // ---- 401/403 disambiguation -----------------------------------------------

    #[tokio::test]
    async fn forbidden_with_rate_headers_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "30"),
            )
            .mount(&server)
            .await;
        let err = repo(&server.uri(), true)
            .get_record("j.json")
            .await
            .expect_err("403 must fail");
        match err {
            JournalError::GithubRateLimited(secs) => assert_eq!(secs, 30),
            other => panic!("expected GithubRateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forbidden_without_rate_headers_and_401_are_auth_failures() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let err = repo(&server.uri(), true)
                .put_record("j.json", &sample_record(), None, "journal")
                .await
                .expect_err("auth must fail");
            assert!(
                matches!(err, JournalError::GithubAuth),
                "status {status}: got {err:?}"
            );
        }
    }

    // ---- issue comments ----------------------------------------------------------

    #[tokio::test]
    async fn upsert_comment_creates_then_updates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/name/issues/12/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": 77})))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/owner/name/issues/comments/77"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 77})))
            .mount(&server)
            .await;

        let repo = repo(&server.uri(), true);
        let created = repo
            .upsert_issue_comment(12, None, "summary")
            .await
            .expect("create");
        assert_eq!(created, 77);
        let updated = repo
            .upsert_issue_comment(12, Some(77), "summary v2")
            .await
            .expect("update");
        assert_eq!(updated, 77);
    }

    // ---- secret hygiene -------------------------------------------------------------

    #[tokio::test]
    async fn no_error_variant_or_debug_ever_contains_the_token() {
        // Drive a real failing request so reqwest-derived errors are covered.
        let unreachable = ProgressRepo::new(
            "http://127.0.0.1:1",
            "owner/name",
            "main",
            Some(SecretString::from(TOKEN.to_string())),
        )
        .expect("client");
        let live_err = unreachable
            .get_record("j.json")
            .await
            .expect_err("unreachable");

        let errors: Vec<JournalError> = vec![
            live_err,
            JournalError::CasExhausted(5),
            JournalError::CasConflict,
            JournalError::RemoteMissing,
            JournalError::Fenced { got: 1, known: 2 },
            JournalError::GithubAuth,
            JournalError::GithubRateLimited(30),
            JournalError::UnsupportedSchema("x@2".to_string()),
            JournalError::Http("contents PUT status 500".to_string()),
            JournalError::Other(anyhow::anyhow!("wrapped context")),
        ];
        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            assert!(!display.contains(TOKEN), "Display leaked: {display}");
            assert!(!debug.contains(TOKEN), "Debug leaked: {debug}");
        }

        let repo_debug = format!("{unreachable:?}");
        assert!(!repo_debug.contains(TOKEN), "repo Debug leaked");
        assert!(repo_debug.contains("<redacted>"));
    }

    #[test]
    fn unverified_sentinel_is_stable() {
        // The sentinel is part of the cross-module contract (bootstrap sets
        // it; flush replaces it); pin its exact value.
        assert_eq!(UNVERIFIED_SHA, "unverified");
    }
}
