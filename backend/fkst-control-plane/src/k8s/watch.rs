//! The Job watcher: maps a session Job's terminal status onto its goal issue
//! (lifecycle labels + a final summary comment via the App token) and keeps the
//! NyxID token fresh while the session runs (issue #301).
//!
//! The pure mappers ([`job_disposition`], [`summary_comment`], [`terminal_labels`])
//! are unit-tested; the live watch loop + refresh ticker are integration glue
//! (they need a cluster), structured here over those tested pieces.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::batch::v1::Job;
use kube::api::Api;
use kube::runtime::{watcher, WatchStreamExt};

use crate::github_app::GithubAppTokens;
use crate::goals::labels::{LABEL_COMPLETED, LABEL_FAILED, LABEL_RUNNING};
use crate::k8s::refresh::{NyxidRefresh, SessionSecretWriter, NYXID_REFRESH_INTERVAL};
use crate::nyxid_connect::{BrokerBindingStore, BrokerClientConfig};

/// A session Job's disposition derived from its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobDisposition {
    /// Still running (no terminal status yet).
    Running,
    /// At least one pod succeeded (exit 0).
    Completed,
    /// A pod failed, or the Job hit a Failed / DeadlineExceeded condition.
    Failed,
}

/// Map a Job's status onto a disposition (pure).
pub fn job_disposition(job: &Job) -> JobDisposition {
    let Some(status) = &job.status else {
        return JobDisposition::Running;
    };
    if status.succeeded.unwrap_or(0) >= 1 {
        return JobDisposition::Completed;
    }
    if status.failed.unwrap_or(0) >= 1 {
        return JobDisposition::Failed;
    }
    if let Some(conditions) = &status.conditions {
        for c in conditions {
            if (c.type_ == "Failed") && c.status == "True" {
                return JobDisposition::Failed;
            }
            if c.type_ == "Complete" && c.status == "True" {
                return JobDisposition::Completed;
            }
        }
    }
    JobDisposition::Running
}

/// The labels to (add, remove) for a terminal disposition. `Running` adds none.
pub fn terminal_labels(disposition: JobDisposition) -> (Vec<&'static str>, Vec<&'static str>) {
    match disposition {
        JobDisposition::Running => (vec![], vec![]),
        JobDisposition::Completed => (vec![LABEL_COMPLETED], vec![LABEL_RUNNING]),
        JobDisposition::Failed => (vec![LABEL_FAILED], vec![LABEL_RUNNING]),
    }
}

/// The final summary comment body posted to the goal issue (pure).
pub fn summary_comment(
    disposition: JobDisposition,
    owner: &str,
    repo: &str,
    log_branch: &str,
) -> String {
    let verdict = match disposition {
        JobDisposition::Completed => "✅ completed",
        JobDisposition::Failed => "❌ failed",
        JobDisposition::Running => "still running",
    };
    format!(
        "**fkst session {verdict}.**\n\nSession log: [`{log_branch}`](https://github.com/{owner}/{repo}/tree/{log_branch}/.fkst/log)."
    )
}

/// One session's issue coordinates, read off the Job annotations the launcher
/// stamps. `None` when an annotation is missing (a Job we did not spawn).
struct JobIssueRef {
    owner: String,
    repo: String,
    number: u64,
    log_branch: String,
    session_id: String,
}

fn issue_ref(job: &Job) -> Option<JobIssueRef> {
    let ann = job.metadata.annotations.as_ref()?;
    let labels = job.metadata.labels.as_ref()?;
    Some(JobIssueRef {
        owner: ann.get("fkst.chrono-ai.fun/owner")?.clone(),
        repo: ann.get("fkst.chrono-ai.fun/repo")?.clone(),
        number: ann.get("fkst.chrono-ai.fun/issue-number")?.parse().ok()?,
        log_branch: ann.get("fkst.chrono-ai.fun/log-branch")?.clone(),
        session_id: labels.get("fkst.chrono-ai.fun/session-id")?.clone(),
    })
}

/// Watches session Jobs and reports their terminal disposition to the goal
/// issue, while driving the per-session NyxID token refresh.
#[derive(Clone)]
pub struct JobWatcher {
    client: kube::Client,
    namespace: String,
    github_app: GithubAppTokens,
    bindings: BrokerBindingStore,
    broker: Option<BrokerClientConfig>,
    nyxid_base_url: Option<String>,
    http: reqwest::Client,
    /// Sessions terminally reported (so a re-Applied event does not double-post).
    reported: Arc<Mutex<HashSet<String>>>,
    /// Sessions with a refresh ticker already running.
    refreshing: Arc<Mutex<HashSet<String>>>,
}

impl JobWatcher {
    /// Assemble a watcher from the shared services.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: kube::Client,
        namespace: impl Into<String>,
        github_app: GithubAppTokens,
        bindings: BrokerBindingStore,
        broker: Option<BrokerClientConfig>,
        nyxid_base_url: Option<String>,
    ) -> Self {
        Self {
            client,
            namespace: namespace.into(),
            github_app,
            bindings,
            broker,
            nyxid_base_url,
            http: reqwest::Client::new(),
            reported: Arc::new(Mutex::new(HashSet::new())),
            refreshing: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Run the watch loop until the stream ends (or errors). Each applied Job is
    /// handled: a running session gets a refresh ticker; a terminal session is
    /// reported to its goal issue exactly once.
    pub async fn run(&self) {
        let api: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let conf = watcher::Config::default().labels("app.kubernetes.io/component=session");
        let mut stream = watcher(api, conf).applied_objects().boxed();
        tracing::info!(namespace = %self.namespace, "job watcher: started");
        loop {
            match stream.try_next().await {
                Ok(Some(job)) => self.handle(job).await,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(error = %error, "job watcher: stream error; continuing");
                }
            }
        }
        tracing::info!("job watcher: stream ended");
    }

    async fn handle(&self, job: Job) {
        let Some(reference) = issue_ref(&job) else {
            return;
        };
        match job_disposition(&job) {
            JobDisposition::Running => self.ensure_refresh(&reference),
            terminal => self.report_terminal(&reference, terminal).await,
        }
    }

    /// Start a self-terminating refresh ticker for a session once (idempotent).
    fn ensure_refresh(&self, reference: &JobIssueRef) {
        let (Some(broker), Some(base_url)) = (self.broker.clone(), self.nyxid_base_url.clone())
        else {
            return; // connect not configured: no refresh to drive.
        };
        let Some(binding) = self.bindings.binding_for_owner(&reference.owner) else {
            tracing::warn!(owner = %reference.owner, "job watcher: no nyxid binding; token refresh disabled for session");
            return;
        };
        {
            let mut set = self.refreshing.lock().expect("refreshing lock");
            if !set.insert(reference.session_id.clone()) {
                return; // already ticking
            }
        }
        let refresh = NyxidRefresh::new(
            SessionSecretWriter::new(self.client.clone(), self.namespace.clone()),
            self.http.clone(),
            base_url,
            broker,
            binding.binding_id,
            reference.session_id.clone(),
        );
        let session_id = reference.session_id.clone();
        let refreshing = self.refreshing.clone();
        tokio::spawn(async move {
            run_refresh_ticker(refresh, &session_id).await;
            refreshing
                .lock()
                .expect("refreshing lock")
                .remove(&session_id);
        });
    }

    /// Report a session's terminal disposition to its goal issue exactly once.
    async fn report_terminal(&self, reference: &JobIssueRef, disposition: JobDisposition) {
        {
            let mut set = self.reported.lock().expect("reported lock");
            if !set.insert(reference.session_id.clone()) {
                return; // already reported
            }
        }
        let owner_repo = format!("{}/{}", reference.owner, reference.repo);
        let (add, remove) = terminal_labels(disposition);
        let add: Vec<String> = add.into_iter().map(String::from).collect();
        if let Err(error) = self
            .github_app
            .add_issue_labels(&owner_repo, reference.number, &add)
            .await
        {
            tracing::warn!(error = %error, "job watcher: failed to add terminal label");
        }
        for label in remove {
            if let Err(error) = self
                .github_app
                .remove_issue_label(&owner_repo, reference.number, label)
                .await
            {
                tracing::warn!(error = %error, label, "job watcher: failed to remove label");
            }
        }
        let body = summary_comment(
            disposition,
            &reference.owner,
            &reference.repo,
            &reference.log_branch,
        );
        if let Err(error) = self
            .github_app
            .post_issue_comment(&owner_repo, reference.number, &body)
            .await
        {
            tracing::warn!(error = %error, "job watcher: failed to post summary comment");
        }
        tracing::info!(
            session_id = %reference.session_id,
            ?disposition,
            "job watcher: session disposition reported"
        );
    }
}

/// The self-terminating refresh loop: rotate the session token every interval
/// until the per-session Secret is gone (the Job was GC'd) or the session ends.
async fn run_refresh_ticker(refresh: NyxidRefresh, session_id: &str) {
    let mut tick = tokio::time::interval(NYXID_REFRESH_INTERVAL);
    tick.tick().await; // the immediate first tick: spawn already minted the first token
    loop {
        tick.tick().await;
        match refresh.refresh_session_token().await {
            Ok(()) => {}
            Err(crate::k8s::refresh::RefreshError::Patch(kube::Error::Api(e))) if e.code == 404 => {
                tracing::info!(
                    session_id,
                    "job watcher: session secret gone; stopping refresh"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(error = %error, session_id, "job watcher: token refresh failed; retrying next tick");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::batch::v1::{JobCondition, JobStatus};

    fn job_with(status: Option<JobStatus>) -> Job {
        Job {
            status,
            ..Default::default()
        }
    }

    #[test]
    fn disposition_running_when_no_terminal_status() {
        assert_eq!(job_disposition(&job_with(None)), JobDisposition::Running);
        assert_eq!(
            job_disposition(&job_with(Some(JobStatus::default()))),
            JobDisposition::Running
        );
    }

    #[test]
    fn disposition_completed_on_succeeded() {
        let s = JobStatus {
            succeeded: Some(1),
            ..Default::default()
        };
        assert_eq!(
            job_disposition(&job_with(Some(s))),
            JobDisposition::Completed
        );
    }

    #[test]
    fn disposition_failed_on_failed_count_or_condition() {
        let s = JobStatus {
            failed: Some(1),
            ..Default::default()
        };
        assert_eq!(job_disposition(&job_with(Some(s))), JobDisposition::Failed);

        let cond = JobCondition {
            type_: "Failed".to_string(),
            status: "True".to_string(),
            ..Default::default()
        };
        let s = JobStatus {
            conditions: Some(vec![cond]),
            ..Default::default()
        };
        assert_eq!(job_disposition(&job_with(Some(s))), JobDisposition::Failed);
    }

    #[test]
    fn terminal_labels_swap_running_for_the_outcome() {
        assert_eq!(
            terminal_labels(JobDisposition::Completed),
            (vec![LABEL_COMPLETED], vec![LABEL_RUNNING])
        );
        assert_eq!(
            terminal_labels(JobDisposition::Failed),
            (vec![LABEL_FAILED], vec![LABEL_RUNNING])
        );
        assert_eq!(terminal_labels(JobDisposition::Running), (vec![], vec![]));
    }

    #[test]
    fn summary_comment_carries_verdict_and_log_link() {
        let body = summary_comment(JobDisposition::Completed, "acme", "site", "fkst/session-x");
        assert!(body.contains("completed"));
        assert!(body.contains("acme/site/tree/fkst/session-x/.fkst/log"));
    }
}
