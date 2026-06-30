//! `issue_comment` -> session control path (PR3).
//!
//! A session lives in its GitHub issue. Once it exists, subsequent comments on
//! that issue drive it: fkst recognizes two slash commands on the FIRST line of a
//! comment — `/stop` and `/status` — authorized PURELY by GitHub identity. The
//! commenter MUST be the issue author; anyone else gets a "permission denied"
//! comment. There is no datastore: every `issue_comment` payload carries both the
//! issue author (`issue.user.login`) and the commenter (`sender.login`), so the
//! authorization decision is a field comparison.
//!
//! Every outcome returns 2xx to GitHub (a non-2xx triggers a redelivery storm):
//! a non-command comment is silently ignored, an unauthorized command earns a
//! single denial comment, and a processing failure is logged + a best-effort
//! comment — never surfaced as a non-2xx that would make GitHub retry-storm.

use serde::Deserialize;

use super::issue_trigger::{ActorPayload, InstallationPayload, RepoPayload};
use super::Handled;
use crate::k8s::{job_disposition, KubeClient};
use crate::routes::session_ops::{
    delete_session_job, engine_version, find_session_job, job_pod, kube_client, status_str,
};
use crate::session_spec::derive_session_id;
use crate::state::AppState;
use k8s_openapi::api::batch::v1::Job;

/// The subset of a GitHub `issue_comment` webhook payload we consume.
#[derive(Debug, Deserialize)]
struct IssueCommentEvent {
    action: String,
    issue: CommentIssuePayload,
    comment: CommentPayload,
    repository: RepoPayload,
    installation: InstallationPayload,
    sender: SenderPayload,
}

/// The issue the comment is on. `user` is the issue AUTHOR — the authorization
/// subject for the control commands.
#[derive(Debug, Deserialize)]
struct CommentIssuePayload {
    number: i64,
    user: ActorPayload,
}

#[derive(Debug, Deserialize)]
struct CommentPayload {
    #[serde(default)]
    body: String,
}

/// The actor who created the comment. `type` distinguishes a human `User` from a
/// `Bot`/App so fkst can skip its OWN comments (and avoid a feedback loop).
#[derive(Debug, Deserialize)]
struct SenderPayload {
    login: String,
    #[serde(rename = "type", default)]
    actor_type: String,
}

/// A recognized control command parsed from a comment's first line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Stop,
    Status,
}

/// Parse the FIRST non-empty line of a comment body into a control command.
///
/// Pure + I/O-free: take the first non-empty line, trim it; if it does not start
/// with `/`, it is not a command. Otherwise the first whitespace-delimited token,
/// lowercased, IS the command (`/stop` -> Stop, `/status` -> Status). Anything
/// else (unknown command, no leading slash, blank body) returns `None` and is
/// silently ignored by the caller.
fn parse_command(body: &str) -> Option<Command> {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    if !line.starts_with('/') {
        return None;
    }
    let token = line.split_whitespace().next()?.to_ascii_lowercase();
    match token.as_str() {
        "/stop" => Some(Command::Stop),
        "/status" => Some(Command::Status),
        _ => None,
    }
}

/// Whether the comment was authored by a bot/App. fkst posts its own comments as
/// the App, so it MUST skip bot-authored comments or it would react to itself.
fn is_bot(sender: &SenderPayload) -> bool {
    sender.actor_type.eq_ignore_ascii_case("Bot") || sender.login.ends_with("[bot]")
}

/// Authorization predicate: only the issue author may control the session.
fn is_authorized(sender_login: &str, author_login: &str) -> bool {
    sender_login == author_login
}

/// Handle an `issue_comment` webhook event (see the module docs). Returns a
/// [`Handled`] for the response code; parse failures surface as `Err` (also a
/// 2xx at the dispatch layer), every other outcome is `Ok`.
pub(super) async fn handle_issue_comment(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: IssueCommentEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse issue_comment event: {e}"))?;

    // Only newly-created comments carry a fresh command; edits/deletes are noise.
    if event.action != "created" {
        return Ok(Handled::Ignored);
    }
    // Never react to fkst's own (App/bot) comments — that would feedback-loop.
    if is_bot(&event.sender) {
        tracing::debug!(sender = %event.sender.login, "issue_comment from bot ignored");
        return Ok(Handled::Ignored);
    }
    // A non-command comment is normal conversation: silently ignore it.
    let Some(command) = parse_command(&event.comment.body) else {
        return Ok(Handled::Ignored);
    };

    // Authorize by GitHub identity alone: the commenter must be the issue author.
    if !is_authorized(&event.sender.login, &event.issue.user.login) {
        tracing::info!(
            sender = %event.sender.login,
            author = %event.issue.user.login,
            issue = event.issue.number,
            "issue_comment: command from non-author denied"
        );
        deny(state, &event).await;
        return Ok(Handled::Ignored);
    }

    if let Err(error) = run_command(state, &event, command).await {
        tracing::error!(
            error = %error,
            issue = event.issue.number,
            command = ?command,
            "issue_comment: command processing failed"
        );
        post_comment(
            state,
            &event,
            &format!("⚠️ fkst could not handle that command: {error}"),
        )
        .await;
    }
    Ok(Handled::CommentControl)
}

/// Execute an authorized control command against the session's Kubernetes Job.
async fn run_command(
    state: &AppState,
    event: &IssueCommentEvent,
    command: Command,
) -> Result<(), String> {
    let owner = &event.repository.owner.login;
    let repo = &event.repository.name;
    let kube = kube_client(state).await.map_err(|e| e.to_string())?;

    match command {
        Command::Stop => {
            // Stop by the deterministic Job name (the token scope / id is the
            // REPO owner, not the issue author). A 404 is already-gone.
            let session_id =
                derive_session_id(event.installation.id, owner, repo, event.issue.number);
            let job_name = format!("fkst-sess-{session_id}");
            delete_session_job(&kube, &job_name)
                .await
                .map_err(|e| e.to_string())?;
            tracing::info!(owner = %owner, repo = %repo, issue = event.issue.number, "issue_comment: session stopped");
            post_comment(state, event, "🛑 Session stopped.").await;
        }
        Command::Status => {
            let markdown = match find_session_job(&kube, owner, repo, event.issue.number)
                .await
                .map_err(|e| e.to_string())?
            {
                None => "ℹ️ No active session for this issue.".to_string(),
                Some(job) => status_markdown(&kube, &job).await,
            };
            post_comment(state, event, &markdown).await;
        }
    }
    Ok(())
}

/// Render a session's status as a markdown comment, reading the Job + its pod via
/// the SAME shared helpers the REST `GET` uses (so the two views never diverge).
async fn status_markdown(kube: &KubeClient, job: &Job) -> String {
    let job_name = job
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "fkst-sess".to_string());
    let pod = job_pod(kube, &job_name).await;
    let pod_id = pod.as_ref().and_then(|p| p.metadata.name.clone());
    let start = pod
        .as_ref()
        .and_then(|p| p.status.as_ref())
        .and_then(|s| s.start_time.as_ref())
        .map(|t| t.0.to_rfc3339());
    let status = status_str(job_disposition(job), pod_id.is_some());

    format!(
        "### fkst session status\n\n\
         - **Status:** `{status}`\n\
         - **Pod:** {}\n\
         - **Started:** {}\n\
         - **Engine:** {}\n",
        pod_id.unwrap_or_else(|| "—".to_string()),
        start.unwrap_or_else(|| "—".to_string()),
        engine_version().unwrap_or_else(|| "—".to_string()),
    )
}

/// Post the permission-denied comment naming the issue author.
async fn deny(state: &AppState, event: &IssueCommentEvent) {
    let body = format!(
        "⛔ Permission denied: only the issue author (@{}) can control this session.",
        event.issue.user.login
    );
    post_comment(state, event, &body).await;
}

/// Best-effort App-token comment on the event's issue. A missing App config or a
/// failed post is logged, never propagated — the webhook still returns 2xx.
async fn post_comment(state: &AppState, event: &IssueCommentEvent, body: &str) {
    let Some(gh) = &state.github_app else {
        tracing::warn!("issue_comment: github app not configured; cannot post comment");
        return;
    };
    let owner_repo = format!("{}/{}", event.repository.owner.login, event.repository.name);
    if let Err(error) = gh
        .post_issue_comment(&owner_repo, event.issue.number as u64, body)
        .await
    {
        tracing::error!(error = %error, issue = event.issue.number, "issue_comment: failed to post comment");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn sender(login: &str, actor_type: &str) -> SenderPayload {
        SenderPayload {
            login: login.to_string(),
            actor_type: actor_type.to_string(),
        }
    }

    // ---- pure command parser -------------------------------------------------

    #[test]
    fn parses_the_two_commands() {
        assert_eq!(parse_command("/stop"), Some(Command::Stop));
        assert_eq!(parse_command("/status"), Some(Command::Status));
    }

    #[test]
    fn command_parsing_is_case_insensitive() {
        assert_eq!(parse_command("/STOP"), Some(Command::Stop));
        assert_eq!(parse_command("/Status"), Some(Command::Status));
    }

    #[test]
    fn only_the_first_token_of_the_first_line_matters() {
        assert_eq!(parse_command("/stop now please"), Some(Command::Stop));
        // A non-command first line wins even if a later line is a command.
        assert_eq!(parse_command("hello\n/stop"), None);
    }

    #[test]
    fn leading_whitespace_and_blank_lines_are_tolerated() {
        assert_eq!(parse_command("   /stop"), Some(Command::Stop));
        assert_eq!(parse_command("\n\n   /status  "), Some(Command::Status));
    }

    #[test]
    fn non_commands_are_ignored() {
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("   "), None);
        assert_eq!(parse_command("stop"), None); // no leading slash
        assert_eq!(parse_command("/help"), None); // unknown command
        assert_eq!(parse_command("/"), None); // bare slash
        assert_eq!(parse_command("please /stop"), None); // slash not first
    }

    // ---- bot-skip ------------------------------------------------------------

    #[test]
    fn bot_authors_are_detected_by_type_or_login_suffix() {
        assert!(is_bot(&sender("fkst-hosted[bot]", "Bot")));
        assert!(is_bot(&sender("some-app[bot]", "User"))); // login suffix alone
        assert!(is_bot(&sender("anyone", "bot"))); // case-insensitive type
        assert!(!is_bot(&sender("octocat", "User")));
    }

    // ---- authorization predicate ---------------------------------------------

    #[test]
    fn only_the_issue_author_is_authorized() {
        assert!(is_authorized("octocat", "octocat"));
        assert!(!is_authorized("intruder", "octocat"));
    }

    // ---- dispatch (no live cluster) ------------------------------------------

    fn state() -> AppState {
        // pod.dispatch defaults to false, so the authorized command path errors
        // at `kube_client` BEFORE any real Kubernetes call — exercising dispatch
        // + the best-effort error comment without a cluster. `github_app: None`
        // makes every `post_comment` a logged no-op.
        AppState {
            config: Config::default(),
            github_app: None,
            github_app_webhook_secret: None,
        }
    }

    fn comment_body(action: &str, sender_login: &str, sender_type: &str, body: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "action": action,
            "issue": { "number": 7, "user": { "login": "octocat" } },
            "comment": { "body": body, "user": { "login": sender_login, "type": sender_type } },
            "repository": { "owner": { "login": "acme" }, "name": "site" },
            "installation": { "id": 42 },
            "sender": { "login": sender_login, "type": sender_type }
        }))
        .expect("serialize")
    }

    #[tokio::test]
    async fn a_non_command_comment_routes_to_the_handler_and_is_ignored() {
        let body = comment_body("created", "octocat", "User", "just a normal comment");
        let handled = handle_issue_comment(&state(), &body).await.expect("ok");
        assert_eq!(handled.as_str(), "ignored");
    }

    #[tokio::test]
    async fn a_command_payload_routes_to_the_handler() {
        // Authorized `/status`: dispatch reaches `run_command`, which errors on
        // the disabled cluster, posts a best-effort comment (no-op), and the
        // handler still returns the comment-handled outcome (a 2xx).
        let body = comment_body("created", "octocat", "User", "/status");
        let handled = handle_issue_comment(&state(), &body).await.expect("ok");
        assert_eq!(handled.as_str(), "comment_control");
    }

    #[tokio::test]
    async fn non_created_actions_are_ignored() {
        let body = comment_body("edited", "octocat", "User", "/stop");
        let handled = handle_issue_comment(&state(), &body).await.expect("ok");
        assert_eq!(handled.as_str(), "ignored");
    }

    #[tokio::test]
    async fn bot_authored_commands_are_ignored() {
        let body = comment_body("created", "fkst-hosted[bot]", "Bot", "/stop");
        let handled = handle_issue_comment(&state(), &body).await.expect("ok");
        assert_eq!(handled.as_str(), "ignored");
    }

    #[tokio::test]
    async fn a_non_author_command_takes_the_permission_denied_branch() {
        // sender != issue.user.login -> denied (a logged no-op comment here since
        // `github_app` is None) -> Ignored, never reaching `run_command`.
        let body = comment_body("created", "intruder", "User", "/stop");
        let handled = handle_issue_comment(&state(), &body).await.expect("ok");
        assert_eq!(handled.as_str(), "ignored");
    }
}
