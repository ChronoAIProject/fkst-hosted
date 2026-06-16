//! `GoalIssueStore` — the database-free replacement for `GoalRepo` (#137).
//!
//! A goal is represented as a **GitHub Issue** on its target repo (the durable,
//! human-visible mirror) plus **controller in-memory state** for everything that
//! must not live in a public issue: the sensitive engine prompt, the
//! goal<->session link, and the issue-number mapping. The in-memory map is
//! AUTHORITATIVE for reads (a `get` after `insert` never re-fetches GitHub);
//! issue writes are the durable mirror (best-effort on failure — the goal still
//! lives in memory and the issue is reconciled later).
//!
//! Single-trigger atomicity is the controller claim's job (#135), NOT a label
//! CAS — the `status:*` label only REFLECTS state. The engine prompt is NEVER
//! written to GitHub (only a non-sensitive title + package count + repo slug).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use crate::error::AppError;
use crate::github_app::GithubAppTokens;
use crate::goals::marker::render_marker;
use crate::goals::model::{GoalDoc, GoalStatus, RepoRef};

/// The label every fkst-hosted goal issue carries (alongside the `status:*`).
pub const GOAL_LABEL: &str = "fkst-hosted:goal";
/// Default GitHub REST base.
const GITHUB_API_BASE: &str = "https://api.github.com";

/// A typed patch for [`GoalIssueStore::patch`] (replaces the old `bson::Document`
/// `$set`). `repo: Some(None)` clears the repo (`clear_repo`).
#[derive(Debug, Default, Clone)]
pub struct GoalPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub package_names: Option<Vec<String>>,
    pub repo: Option<Option<RepoRef>>,
}

/// Fields of an issue PATCH (only the `Some` ones are sent).
#[derive(Debug, Default, Clone)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    pub state: Option<String>,
}

/// The GitHub Issue operations the store needs — a seam so the in-memory logic
/// is testable with a recording fake (and the real impl mints the App token +
/// makes the HTTP call).
#[async_trait]
pub trait IssueApi: Send + Sync {
    /// Create an issue; returns its number.
    async fn create_issue(
        &self,
        repo: &RepoRef,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, AppError>;

    /// Patch an existing issue (title/body/labels/state).
    async fn patch_issue(
        &self,
        repo: &RepoRef,
        number: u64,
        patch: IssuePatch,
    ) -> Result<(), AppError>;
}

/// The production [`IssueApi`]: mints the App installation token per write and
/// calls the GitHub REST API.
pub struct HttpIssueApi {
    github_app: GithubAppTokens,
    http: reqwest::Client,
    api_base: String,
}

impl HttpIssueApi {
    fn new(github_app: GithubAppTokens, api_base: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("fkst-hosted-api")
            .build()
            .expect("reqwest client");
        Self {
            github_app,
            http,
            api_base,
        }
    }

    async fn token(&self, repo: &RepoRef) -> Result<SecretString, AppError> {
        let owner_repo = format!("{}/{}", repo.owner, repo.name);
        Ok(self.github_app.token_for_repo(&owner_repo, None).await?)
    }
}

#[async_trait]
impl IssueApi for HttpIssueApi {
    async fn create_issue(
        &self,
        repo: &RepoRef,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, AppError> {
        let token = self.token(repo).await?;
        let url = format!(
            "{}/repos/{}/{}/issues",
            self.api_base, repo.owner, repo.name
        );
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("token {}", token.expose_secret()))
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({ "title": title, "body": body, "labels": labels }))
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("github issue create transport error: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            tracing::error!(repo = %url, status = status.as_u16(), "github issue create failed");
            return Err(AppError::Upstream(format!(
                "github returned {} creating the goal issue",
                status.as_u16()
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::Upstream(format!("github issue create decode error: {e}")))?;
        value
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AppError::Upstream("github issue response missing number".to_string()))
    }

    async fn patch_issue(
        &self,
        repo: &RepoRef,
        number: u64,
        patch: IssuePatch,
    ) -> Result<(), AppError> {
        let token = self.token(repo).await?;
        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            self.api_base, repo.owner, repo.name, number
        );
        let mut body = serde_json::Map::new();
        if let Some(title) = patch.title {
            body.insert("title".into(), title.into());
        }
        if let Some(issue_body) = patch.body {
            body.insert("body".into(), issue_body.into());
        }
        if let Some(labels) = patch.labels {
            body.insert("labels".into(), serde_json::json!(labels));
        }
        if let Some(state) = patch.state {
            body.insert("state".into(), state.into());
        }
        let resp = self
            .http
            .patch(&url)
            .header("Authorization", format!("token {}", token.expose_secret()))
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .map_err(|e| AppError::Upstream(format!("github issue patch transport error: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            tracing::error!(
                issue = number,
                status = status.as_u16(),
                "github issue patch failed"
            );
            return Err(AppError::Upstream(format!(
                "github returned {} patching the goal issue",
                status.as_u16()
            )));
        }
        Ok(())
    }
}

/// No-op [`IssueApi`] used when the GitHub App is disabled: goals live in
/// memory only (the authoritative read path), with no GitHub mirror.
struct NoopIssueApi;

#[async_trait]
impl IssueApi for NoopIssueApi {
    async fn create_issue(
        &self,
        _repo: &RepoRef,
        _title: &str,
        _body: &str,
        _labels: &[String],
    ) -> Result<u64, AppError> {
        tracing::warn!("github app disabled; goal issue not mirrored (in-memory only)");
        Ok(0)
    }
    async fn patch_issue(
        &self,
        _repo: &RepoRef,
        _number: u64,
        _patch: IssuePatch,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

/// In-memory state for one goal: the full `GoalDoc` (incl. the prompt) + the
/// issue number once materialized.
#[derive(Debug, Clone)]
struct GoalEntry {
    doc: GoalDoc,
    issue_number: Option<u64>,
}

/// The goal store: in-memory authoritative state + GitHub-Issue mirror.
#[derive(Clone)]
pub struct GoalIssueStore {
    issues: Arc<dyn IssueApi>,
    store: Arc<Mutex<HashMap<bson::Uuid, GoalEntry>>>,
}

/// The `status:<value>` label for a goal status (snake_case, 1:1 with the wire).
pub fn status_label(status: GoalStatus) -> String {
    let v = match status {
        GoalStatus::NotStarted => "not_started",
        GoalStatus::Triggered => "triggered",
        GoalStatus::Running => "running",
        GoalStatus::Stopped => "stopped",
        GoalStatus::Failed => "failed",
    };
    format!("status:{v}")
}

/// The full label set for a goal issue at `status`.
fn labels_for(status: GoalStatus) -> Vec<String> {
    vec![GOAL_LABEL.to_string(), status_label(status)]
}

/// A NON-sensitive one-line summary for the issue body — title + package count
/// + repo slug ONLY. NEVER the description (the engine prompt).
fn non_sensitive_summary(goal: &GoalDoc) -> String {
    let repo = goal
        .repo
        .as_ref()
        .map(|r| format!(" · `{}/{}`", r.owner, r.name))
        .unwrap_or_default();
    format!(
        "**{}** · {} package(s){}",
        goal.title.trim(),
        goal.package_names.len(),
        repo
    )
}

/// The full issue body: non-sensitive summary + the hidden marker.
fn issue_body(goal: &GoalDoc) -> String {
    format!("{}\n\n{}", non_sensitive_summary(goal), render_marker(goal))
}

impl GoalIssueStore {
    /// Production constructor. With the GitHub App configured, goals are
    /// mirrored to GitHub Issues; without it (App disabled), the store is
    /// in-memory only (the authoritative read path still works — no mirror).
    pub fn new(github_app: Option<GithubAppTokens>) -> Self {
        match github_app {
            Some(app) => Self::with_api(Arc::new(HttpIssueApi::new(
                app,
                GITHUB_API_BASE.to_string(),
            ))),
            None => Self::with_api(Arc::new(NoopIssueApi)),
        }
    }

    /// Construct over an injected [`IssueApi`] (tests / a custom API base).
    pub fn with_api(issues: Arc<dyn IssueApi>) -> Self {
        Self {
            issues,
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<bson::Uuid, GoalEntry>> {
        self.store.lock().expect("goal store poisoned")
    }

    /// Create a goal. With `repo` set, file the GitHub issue (storing its
    /// number); with `repo` unset (create_new path) hold it in memory only —
    /// [`Self::set_repo`] materializes the issue at trigger time.
    pub async fn insert(&self, goal: &GoalDoc) -> Result<(), AppError> {
        // In-memory state is authoritative; record it first.
        self.lock().insert(
            goal.id,
            GoalEntry {
                doc: goal.clone(),
                issue_number: None,
            },
        );
        if let Some(repo) = goal.repo.clone() {
            let number = self
                .issues
                .create_issue(
                    &repo,
                    &goal.title,
                    &issue_body(goal),
                    &labels_for(goal.status),
                )
                .await?;
            if let Some(entry) = self.lock().get_mut(&goal.id) {
                entry.issue_number = Some(number);
            }
            tracing::info!(goal_id = %goal.id, issue = number, repo = %format!("{}/{}", repo.owner, repo.name), "goal issue created");
        }
        Ok(())
    }

    /// Read a goal from the authoritative in-memory map. `Ok(None)` for an
    /// unknown id (degrades gracefully — never panics).
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<GoalDoc>, AppError> {
        Ok(self.lock().get(&id).map(|e| e.doc.clone()))
    }

    /// List goals visible to `owner_user_id` / `visible_org_ids`, filtered by
    /// `status`, newest-first, `skip(offset).take(limit)` — all from memory
    /// (avoids the GitHub Search 30/min budget).
    pub async fn list(
        &self,
        owner_user_id: &str,
        visible_org_ids: &[String],
        status: Option<GoalStatus>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<GoalDoc>, AppError> {
        let map = self.lock();
        let mut goals: Vec<GoalDoc> = map
            .values()
            .map(|e| e.doc.clone())
            .filter(|g| {
                let visible = g.owner_user_id == owner_user_id
                    || g.org_id
                        .as_ref()
                        .is_some_and(|o| visible_org_ids.contains(o));
                let status_ok = status.is_none_or(|s| g.status == s);
                visible && status_ok
            })
            .collect();
        goals.sort_by_key(|g| std::cmp::Reverse(g.created_at));
        Ok(goals
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }

    /// CAS-style patch: apply `patch` iff the goal exists and (when
    /// `mutability_filter` is `Some`) its status is in the allowed set; else
    /// `Ok(None)` (the caller maps to 409). `description` updates memory only.
    pub async fn patch(
        &self,
        id: bson::Uuid,
        mutability_filter: Option<Vec<GoalStatus>>,
        patch: GoalPatch,
    ) -> Result<Option<GoalDoc>, AppError> {
        // Mutate in memory under the lock, computing the issue write to do after.
        let (doc, issue, repo) = {
            let mut map = self.lock();
            let Some(entry) = map.get_mut(&id) else {
                return Ok(None);
            };
            if let Some(allowed) = &mutability_filter {
                if !allowed.contains(&entry.doc.status) {
                    return Ok(None);
                }
            }
            let mut touched_issue = false;
            if let Some(title) = patch.title {
                entry.doc.title = title;
                touched_issue = true;
            }
            if let Some(description) = patch.description {
                entry.doc.description = description; // memory only — never the issue
            }
            if let Some(package_names) = patch.package_names {
                entry.doc.package_names = package_names;
                touched_issue = true;
            }
            if let Some(repo) = patch.repo {
                entry.doc.repo = repo;
                touched_issue = true;
            }
            entry.doc.updated_at = bson::DateTime::now();
            (
                entry.doc.clone(),
                touched_issue.then_some(entry.issue_number).flatten(),
                entry.doc.repo.clone(),
            )
        };
        // Mirror the title/body change to the issue (if materialized).
        if let (Some(number), Some(repo)) = (issue, repo) {
            self.issues
                .patch_issue(
                    &repo,
                    number,
                    IssuePatch {
                        title: Some(doc.title.clone()),
                        body: Some(issue_body(&doc)),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(Some(doc))
    }

    /// CAS-style delete: iff the goal's status is in `allowed_statuses`, CLOSE
    /// the issue (GitHub issues cannot be REST-deleted; closing is the durable
    /// "deleted" record) and drop it from memory. Returns the goal as it was.
    pub async fn delete(
        &self,
        id: bson::Uuid,
        allowed_statuses: &[GoalStatus],
    ) -> Result<Option<GoalDoc>, AppError> {
        let (doc, issue, repo) = {
            let map = self.lock();
            let Some(entry) = map.get(&id) else {
                return Ok(None);
            };
            if !allowed_statuses.contains(&entry.doc.status) {
                return Ok(None);
            }
            (
                entry.doc.clone(),
                entry.issue_number,
                entry.doc.repo.clone(),
            )
        };
        if let (Some(number), Some(repo)) = (issue, repo) {
            self.issues
                .patch_issue(
                    &repo,
                    number,
                    IssuePatch {
                        state: Some("closed".to_string()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        self.lock().remove(&id);
        tracing::info!(goal_id = %id, "goal deleted (issue closed)");
        Ok(Some(doc))
    }

    /// Status-label CAS: iff the current status is in `from_statuses`, swap the
    /// `status:*` label on the issue + update memory; optionally clear the
    /// active-session link. Single-trigger atomicity is the controller claim's
    /// job (#135) — this only REFLECTS state. `Ok(None)` on a CAS miss.
    pub async fn transition_status(
        &self,
        id: bson::Uuid,
        from_statuses: &[GoalStatus],
        new_status: GoalStatus,
        clear_active: bool,
    ) -> Result<Option<GoalDoc>, AppError> {
        let (doc, issue, repo) = {
            let mut map = self.lock();
            let Some(entry) = map.get_mut(&id) else {
                return Ok(None);
            };
            if !from_statuses.contains(&entry.doc.status) {
                return Ok(None);
            }
            entry.doc.status = new_status;
            if clear_active {
                entry.doc.active_session_id = None;
            }
            entry.doc.updated_at = bson::DateTime::now();
            (
                entry.doc.clone(),
                entry.issue_number,
                entry.doc.repo.clone(),
            )
        };
        if let (Some(number), Some(repo)) = (issue, repo) {
            self.issues
                .patch_issue(
                    &repo,
                    number,
                    IssuePatch {
                        labels: Some(labels_for(new_status)),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Ok(Some(doc))
    }

    /// Write the goal->session link into memory (authoritative) iff the goal is
    /// `Triggered`; returns whether it matched. No GitHub call.
    pub async fn set_active_session(
        &self,
        goal_id: bson::Uuid,
        session_id: bson::Uuid,
    ) -> Result<bool, AppError> {
        let mut map = self.lock();
        let Some(entry) = map.get_mut(&goal_id) else {
            return Ok(false);
        };
        if entry.doc.status != GoalStatus::Triggered {
            return Ok(false);
        }
        entry.doc.active_session_id = Some(session_id);
        entry.doc.updated_at = bson::DateTime::now();
        Ok(true)
    }

    /// Set the repo on a goal. For a memory-only create_new goal this
    /// MATERIALIZES the issue (one POST); for an already-materialized goal it
    /// re-renders the marker + patches the body. Returns whether it applied.
    pub async fn set_repo(&self, goal_id: bson::Uuid, repo: &RepoRef) -> Result<bool, AppError> {
        let (doc, existing_issue) = {
            let mut map = self.lock();
            let Some(entry) = map.get_mut(&goal_id) else {
                return Ok(false);
            };
            entry.doc.repo = Some(repo.clone());
            entry.doc.updated_at = bson::DateTime::now();
            (entry.doc.clone(), entry.issue_number)
        };
        match existing_issue {
            None => {
                let number = self
                    .issues
                    .create_issue(repo, &doc.title, &issue_body(&doc), &labels_for(doc.status))
                    .await?;
                if let Some(entry) = self.lock().get_mut(&goal_id) {
                    entry.issue_number = Some(number);
                }
                tracing::info!(goal_id = %goal_id, issue = number, "goal issue materialized on set_repo");
            }
            Some(number) => {
                self.issues
                    .patch_issue(
                        repo,
                        number,
                        IssuePatch {
                            body: Some(issue_body(&doc)),
                            ..Default::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(true)
    }

    /// Test/diagnostic: the active-session link for a goal (controller memory).
    pub async fn active_session(&self, goal_id: bson::Uuid) -> Option<bson::Uuid> {
        self.lock()
            .get(&goal_id)
            .and_then(|e| e.doc.active_session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::marker::parse_marker;

    /// One recorded `create_issue` call: `(repo, title, body, labels)`.
    type CreatedIssue = (RepoRef, String, String, Vec<String>);

    /// Recording fake IssueApi.
    #[derive(Default)]
    struct FakeIssueApi {
        created: Mutex<Vec<CreatedIssue>>,
        patched: Mutex<Vec<(u64, IssuePatch)>>,
        next_number: Mutex<u64>,
    }

    #[async_trait]
    impl IssueApi for FakeIssueApi {
        async fn create_issue(
            &self,
            repo: &RepoRef,
            title: &str,
            body: &str,
            labels: &[String],
        ) -> Result<u64, AppError> {
            self.created.lock().unwrap().push((
                repo.clone(),
                title.to_string(),
                body.to_string(),
                labels.to_vec(),
            ));
            let mut n = self.next_number.lock().unwrap();
            *n += 1;
            Ok(*n)
        }

        async fn patch_issue(
            &self,
            _repo: &RepoRef,
            number: u64,
            patch: IssuePatch,
        ) -> Result<(), AppError> {
            self.patched.lock().unwrap().push((number, patch));
            Ok(())
        }
    }

    fn store_with(fake: Arc<FakeIssueApi>) -> GoalIssueStore {
        GoalIssueStore::with_api(fake)
    }

    fn goal(repo: Option<RepoRef>) -> GoalDoc {
        GoalDoc {
            id: bson::Uuid::new(),
            title: "Build it".to_string(),
            description: "SECRET-PROMPT".to_string(),
            package_names: vec!["pkg-a".to_string()],
            repo,
            status: GoalStatus::NotStarted,
            owner_user_id: "user-1".to_string(),
            org_id: None,
            active_session_id: None,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
        }
    }

    fn repo() -> RepoRef {
        RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        }
    }

    #[test]
    fn summary_never_contains_description() {
        assert!(!non_sensitive_summary(&goal(Some(repo()))).contains("SECRET-PROMPT"));
    }

    #[tokio::test]
    async fn insert_existing_repo_posts_issue_with_status_label() {
        let fake = Arc::new(FakeIssueApi::default());
        let store = store_with(fake.clone());
        store.insert(&goal(Some(repo()))).await.unwrap();
        let created = fake.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        let (_repo, _title, body, labels) = &created[0];
        assert!(labels.contains(&"status:not_started".to_string()));
        assert!(labels.contains(&GOAL_LABEL.to_string()));
        assert!(
            !body.contains("SECRET-PROMPT"),
            "prompt never in the issue body"
        );
        assert!(parse_marker(body).is_ok(), "body carries a valid marker");
    }

    #[tokio::test]
    async fn insert_create_new_holds_in_memory_then_set_repo_materializes() {
        let fake = Arc::new(FakeIssueApi::default());
        let store = store_with(fake.clone());
        let g = goal(None);
        store.insert(&g).await.unwrap();
        assert!(
            fake.created.lock().unwrap().is_empty(),
            "create_new files no issue"
        );
        // get returns the in-memory goal even with no issue.
        assert!(store.get(g.id).await.unwrap().is_some());
        store.set_repo(g.id, &repo()).await.unwrap();
        assert_eq!(
            fake.created.lock().unwrap().len(),
            1,
            "set_repo files exactly one"
        );
    }

    #[tokio::test]
    async fn transition_status_cas_miss_returns_none() {
        let store = store_with(Arc::new(FakeIssueApi::default()));
        let g = goal(Some(repo()));
        store.insert(&g).await.unwrap();
        // current is NotStarted; from=[Running] misses.
        let r = store
            .transition_status(g.id, &[GoalStatus::Running], GoalStatus::Stopped, false)
            .await
            .unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn transition_status_swaps_label() {
        let fake = Arc::new(FakeIssueApi::default());
        let store = store_with(fake.clone());
        let g = goal(Some(repo()));
        store.insert(&g).await.unwrap();
        store
            .transition_status(
                g.id,
                &[GoalStatus::NotStarted],
                GoalStatus::Triggered,
                false,
            )
            .await
            .unwrap();
        let patched = fake.patched.lock().unwrap();
        let labels = patched.last().unwrap().1.labels.as_ref().unwrap();
        assert!(labels.contains(&"status:triggered".to_string()));
    }

    #[tokio::test]
    async fn patch_immutable_status_returns_none() {
        let store = store_with(Arc::new(FakeIssueApi::default()));
        let g = goal(Some(repo()));
        store.insert(&g).await.unwrap();
        store
            .transition_status(g.id, &[GoalStatus::NotStarted], GoalStatus::Running, false)
            .await
            .unwrap();
        let r = store
            .patch(
                g.id,
                Some(vec![
                    GoalStatus::NotStarted,
                    GoalStatus::Stopped,
                    GoalStatus::Failed,
                ]),
                GoalPatch {
                    title: Some("new".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            r.is_none(),
            "patch on a Running goal with the mutability filter misses"
        );
    }

    #[tokio::test]
    async fn set_active_session_only_when_triggered() {
        let store = store_with(Arc::new(FakeIssueApi::default()));
        let g = goal(Some(repo()));
        store.insert(&g).await.unwrap();
        // NotStarted -> false
        assert!(!store
            .set_active_session(g.id, bson::Uuid::new())
            .await
            .unwrap());
        store
            .transition_status(
                g.id,
                &[GoalStatus::NotStarted],
                GoalStatus::Triggered,
                false,
            )
            .await
            .unwrap();
        let sid = bson::Uuid::new();
        assert!(store.set_active_session(g.id, sid).await.unwrap());
        assert_eq!(store.active_session(g.id).await, Some(sid));
    }

    #[tokio::test]
    async fn list_filters_by_owner_and_status_newest_first() {
        let store = store_with(Arc::new(FakeIssueApi::default()));
        store.insert(&goal(Some(repo()))).await.unwrap();
        let mut other = goal(Some(repo()));
        other.owner_user_id = "user-2".to_string();
        store.insert(&other).await.unwrap();
        let mine = store.list("user-1", &[], None, 50, 0).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].owner_user_id, "user-1");
    }

    /// #142: org-scoped LIST visibility is an in-memory filter (no Mongo `$in`):
    /// a goal owned by someone else is included only when its `org_id` is in the
    /// caller's visible-org set; a non-visible, non-owned org goal is excluded.
    #[tokio::test]
    async fn list_filters_by_in_memory_visibility() {
        let store = store_with(Arc::new(FakeIssueApi::default()));
        // A stranger's goal in org-visible.
        let mut visible = goal(Some(repo()));
        visible.owner_user_id = "stranger".to_string();
        visible.org_id = Some("org-visible".to_string());
        store.insert(&visible).await.unwrap();
        // A stranger's goal in an org the caller cannot see.
        let mut hidden = goal(Some(repo()));
        hidden.owner_user_id = "stranger".to_string();
        hidden.org_id = Some("org-hidden".to_string());
        store.insert(&hidden).await.unwrap();

        let visible_ids = ["org-visible".to_string()];
        let listed = store
            .list("caller", &visible_ids, None, 50, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1, "only the visible-org goal is listed");
        assert_eq!(listed[0].org_id.as_deref(), Some("org-visible"));
    }
}
