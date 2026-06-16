//! The (dormant) issue-comment mirror for the per-run progress summary.
//!
//! Split out of [`crate::github`] to keep that file focused on the
//! Contents-API record path; it remains an inherent method on
//! [`ProgressRepo`] via a second `impl` block (same crate, same struct).

use reqwest::StatusCode;

use crate::github::ProgressRepo;
use crate::github_http::{classify_status, http_err};
use crate::JournalError;

impl ProgressRepo {
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
            let url = format!(
                "{}/repos/{}/issues/comments/{id}",
                self.api_base(),
                self.repo()
            );
            let response = self
                .decorate(self.client().patch(url))
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
            self.api_base(),
            self.repo()
        );
        let response = self
            .decorate(self.client().post(url))
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
    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::github::ProgressRepo;

    const TOKEN: &str = "ghp_supersecret_token_value_1234567890";

    fn repo(server_uri: &str, with_token: bool) -> ProgressRepo {
        let token = with_token.then(|| SecretString::from(TOKEN.to_string()));
        ProgressRepo::new(server_uri, "owner/name", "main", token).expect("client")
    }

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
}
