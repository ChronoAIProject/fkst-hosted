//! `issues.opened` -> pod session pipeline (issue #303).
//!
//! The token-less entrypoint: a qualifying GitHub `issues.opened` webhook drives
//! the whole pipeline — parse the issue, mint the GitHub App installation token,
//! attach the static LLM API key from config, build the SessionSpec +
//! per-session Secret, and launch the Job. Everything flows from
//! `installation.id`; no user token is present.

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use super::Handled;
use crate::goals::issue_parse::{parse_goal_issue_body, ParsedGoal};
use crate::goals::labels::GOAL_LABEL;
use crate::k8s::{user_store, KubeClient, PodSessionLauncher, SessionSecrets};
use crate::models::RepoRef;
use crate::session_spec::{derive_session_id, SessionGoal, SessionSpec};
use crate::state::AppState;

/// The subset of a GitHub `issues` webhook payload we consume.
#[derive(Debug, Deserialize)]
pub(super) struct IssuesEvent {
    pub action: String,
    pub issue: IssuePayload,
    pub repository: RepoPayload,
    pub installation: InstallationPayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssuePayload {
    pub number: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Vec<LabelPayload>,
    /// The issue author. Its `login` is the session's authorization subject (the
    /// only identity allowed to drive `/stop` + `/status` later, see PR3).
    pub user: ActorPayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct LabelPayload {
    pub name: String,
}

/// A GitHub actor (issue author / comment author / sender). Reused across the
/// `issues` and `issue_comment` webhook shapes.
#[derive(Debug, Deserialize)]
pub(super) struct ActorPayload {
    pub login: String,
    /// The actor's immutable numeric GitHub id. For an issue author it keys their
    /// per-user env store (`fkst-user-<id>`), the ONLY identity used to resolve
    /// injected env (PR4b). Required: GitHub always includes `user.id`, and a
    /// silent `0` fallback would resolve the WRONG store.
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct RepoPayload {
    pub owner: OwnerPayload,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OwnerPayload {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InstallationPayload {
    pub id: i64,
}

/// Whether this event should auto-trigger a session: a freshly opened issue
/// carrying the configured trigger label.
pub(super) fn should_trigger(event: &IssuesEvent, trigger_label: &str) -> bool {
    event.action == "opened" && event.issue.labels.iter().any(|l| l.name == trigger_label)
}

/// Build the non-secret SessionSpec for an issue-triggered session. The
/// `session_id` is deterministic, so a webhook redelivery maps to the same
/// session (at-most-one-Job).
///
/// `owner` is the **repo** owner (it scopes the App token and seeds the session
/// id); `author_login` is the **issue author** and becomes `SessionSpec.owner_login`
/// — the authorization subject for the issue-comment control path. The two are
/// usually different identities, so they are passed separately on purpose.
pub(super) fn build_session_spec(
    installation_id: i64,
    owner: &str,
    name: &str,
    issue_number: i64,
    author_login: &str,
    title: &str,
    parsed: &ParsedGoal,
) -> SessionSpec {
    let session_id = derive_session_id(installation_id, owner, name, issue_number);
    SessionSpec {
        run_key: session_id.clone(),
        log_branch: format!("fkst/session-{session_id}"),
        session_id,
        installation_id,
        repo: RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        owner_login: author_login.to_string(),
        issue_number,
        goal: SessionGoal {
            title: title.to_string(),
            prompt: parsed.description.clone(),
        },
        package_names: parsed.package_names.clone(),
    }
}

/// Handle an `issues` webhook event: ignore non-qualifying ones, else trigger a
/// session (posting a failure comment if the launch fails).
pub(super) async fn handle_issues(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: IssuesEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse issues event: {e}"))?;
    if !should_trigger(&event, &state.config.webhook_trigger_label) {
        return Ok(Handled::Ignored);
    }
    if let Err(error) = trigger_session(state, &event).await {
        tracing::error!(error = %error, issue = event.issue.number, "webhook trigger: session launch failed");
        if let Some(gh) = &state.github_app {
            let owner_repo = format!("{}/{}", event.repository.owner.login, event.repository.name);
            let _ = gh
                .post_issue_comment(
                    &owner_repo,
                    event.issue.number as u64,
                    &format!("⚠️ fkst could not start a session for this issue: {error}"),
                )
                .await;
        }
    }
    Ok(Handled::Triggered)
}

/// Drive the full launch for a qualifying issue.
async fn trigger_session(state: &AppState, event: &IssuesEvent) -> Result<(), String> {
    let owner = &event.repository.owner.login;
    let name = &event.repository.name;
    let owner_repo = format!("{owner}/{name}");
    let github_app = state
        .github_app
        .as_ref()
        .ok_or("github app not configured")?;

    let body = event.issue.body.clone().unwrap_or_default();
    let parsed = parse_goal_issue_body(&body).map_err(|e| format!("parse issue body: {e}"))?;
    let spec = build_session_spec(
        event.installation.id,
        owner,
        name,
        event.issue.number,
        &event.issue.user.login,
        &event.issue.title,
        &parsed,
    );

    let github_token = github_app
        .token_for_repo(&owner_repo, None)
        .await
        .map_err(|e| format!("mint app token: {e}"))?;

    let kube = KubeClient::from_inferred(&state.config.pod.namespace)
        .await
        .map_err(|e| format!("kubernetes client: {e}"))?;

    // Resolve the issue author's requested env (PR4b) from their per-user store,
    // reusing the SAME client the launcher uses. Best-effort: a store read error
    // yields no user env (logged), never a launch failure — optional env must not
    // sink the whole session.
    let user_env = resolve_user_env(&kube, event.issue.user.id, &parsed.env_keys).await;

    // The LLM credential is a static config value (FKST_LLM_API_KEY). It is
    // written into the session Secret and read by the engine's codex provider
    // under LLM_API_KEY.
    let secrets = SessionSecrets {
        github_token,
        llm_api_key: state.config.llm_api_key.clone(),
        user_env,
    };

    let launcher = PodSessionLauncher::new(
        kube.client().clone(),
        state.config.pod.namespace.clone(),
        state.config.pod.clone(),
    );
    launcher
        .launch(&spec, secrets)
        .await
        .map_err(|e| format!("launch session job: {e}"))?;

    // Mark the issue as an fkst goal (best-effort).
    let _ = github_app
        .add_issue_labels(
            &owner_repo,
            event.issue.number as u64,
            &[GOAL_LABEL.to_string()],
        )
        .await;
    tracing::info!(session_id = %spec.session_id, owner = %owner, "webhook trigger: session job launched");
    Ok(())
}

/// Resolve the issue author's requested env into a secret map by reading their
/// `fkst-user-<id>` store (PR4b).
///
/// Best-effort by design: an empty `requested` short-circuits to no work, and a
/// store read failure logs + yields an EMPTY map rather than failing the launch —
/// optional env must never sink the whole session. The selection itself is the
/// pure [`select_user_env`]; this wrapper adds only the I/O + the per-key
/// "requested but absent from the store" log line.
async fn resolve_user_env(
    kube: &KubeClient,
    author_id: i64,
    requested: &[String],
) -> BTreeMap<String, SecretString> {
    if requested.is_empty() {
        return BTreeMap::new();
    }
    let variables = match user_store::get_env(kube, author_id).await {
        Ok(variables) => variables,
        Err(error) => {
            tracing::warn!(github_user_id = author_id, error = %error, "webhook trigger: could not read user env store; proceeding with no user env");
            return BTreeMap::new();
        }
    };
    let secrets = match user_store::get_secret_values(kube, author_id).await {
        Ok(secrets) => secrets,
        Err(error) => {
            tracing::warn!(github_user_id = author_id, error = %error, "webhook trigger: could not read user secret store; proceeding with no user env");
            return BTreeMap::new();
        }
    };
    let resolved = select_user_env(requested, &variables, &secrets);
    for key in requested {
        if !resolved.contains_key(key) {
            tracing::info!(github_user_id = author_id, key = %key, "webhook trigger: requested env key not in user store; skipped");
        }
    }
    tracing::info!(
        github_user_id = author_id,
        requested = requested.len(),
        injected = resolved.len(),
        "webhook trigger: resolved user env for injection"
    );
    resolved
}

/// Pure selector: take ONLY the `requested` keys, looking each up FIRST in the
/// user's `secrets`, then in their non-secret `variables` (a secret of the same
/// name wins, so a plaintext can never override a secret). A requested key in
/// NEITHER map is dropped. I/O-free + cluster-free so it is unit-testable.
fn select_user_env(
    requested: &[String],
    variables: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
) -> BTreeMap<String, SecretString> {
    let mut resolved = BTreeMap::new();
    for key in requested {
        if let Some(value) = secrets.get(key).or_else(|| variables.get(key)) {
            resolved.insert(key.clone(), SecretString::from(value.clone()));
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(action: &str, labels: &[&str]) -> IssuesEvent {
        IssuesEvent {
            action: action.to_string(),
            issue: IssuePayload {
                number: 7,
                title: "Add dark mode".to_string(),
                body: Some("### Goal\ndo it\n\n### Package Name List\nweb\n".to_string()),
                labels: labels
                    .iter()
                    .map(|n| LabelPayload {
                        name: n.to_string(),
                    })
                    .collect(),
                user: ActorPayload {
                    login: "octocat".to_string(),
                    id: 583231,
                },
            },
            repository: RepoPayload {
                owner: OwnerPayload {
                    login: "acme".to_string(),
                },
                name: "site".to_string(),
            },
            installation: InstallationPayload { id: 42 },
        }
    }

    #[test]
    fn triggers_only_on_opened_with_the_label() {
        assert!(should_trigger(&event("opened", &["fkst"]), "fkst"));
        assert!(!should_trigger(&event("opened", &["other"]), "fkst"));
        assert!(!should_trigger(&event("edited", &["fkst"]), "fkst"));
        assert!(!should_trigger(&event("opened", &[]), "fkst"));
    }

    #[test]
    fn issues_event_parses_a_representative_payload() {
        let payload = serde_json::json!({
            "action": "opened",
            "issue": {
                "number": 9,
                "title": "T",
                "body": "B",
                "labels": [{"name": "fkst"}],
                "user": { "login": "octocat", "id": 583231 }
            },
            "repository": { "owner": { "login": "acme" }, "name": "site" },
            "installation": { "id": 42 }
        });
        let event: IssuesEvent = serde_json::from_value(payload).expect("parses");
        assert_eq!(event.issue.number, 9);
        assert_eq!(event.issue.user.login, "octocat");
        assert_eq!(event.issue.user.id, 583231);
        assert_eq!(event.repository.owner.login, "acme");
        assert_eq!(event.installation.id, 42);
        assert!(should_trigger(&event, "fkst"));
    }

    #[test]
    fn build_spec_maps_fields_and_is_deterministic() {
        let parsed = ParsedGoal {
            description: "do it".to_string(),
            package_names: vec!["web".to_string()],
            env_keys: vec![],
        };
        let a = build_session_spec(42, "acme", "site", 7, "octocat", "T", &parsed);
        let b = build_session_spec(42, "acme", "site", 7, "octocat", "T", &parsed);
        assert_eq!(
            a.session_id, b.session_id,
            "deterministic id (redelivery dedup)"
        );
        assert_eq!(a.repo.owner, "acme", "repo owner scopes the token + id");
        assert_eq!(
            a.owner_login, "octocat",
            "owner_login is the issue author (authz subject), not the repo owner"
        );
        assert_eq!(a.issue_number, 7);
        assert_eq!(a.goal.prompt, "do it");
        assert_eq!(a.package_names, vec!["web".to_string()]);
        assert_eq!(a.log_branch, format!("fkst/session-{}", a.session_id));
    }

    // ---- select_user_env (pure; no cluster) ----------------------------------

    use secrecy::ExposeSecret;

    #[test]
    fn select_user_env_picks_only_requested_present_keys_from_either_store() {
        let variables = BTreeMap::from([
            ("PLAIN".to_string(), "p-val".to_string()),
            ("UNREQUESTED".to_string(), "ignored".to_string()),
        ]);
        let secrets = BTreeMap::from([("SECRET".to_string(), "s-val".to_string())]);
        let requested = vec![
            "PLAIN".to_string(),
            "SECRET".to_string(),
            "MISSING".to_string(),
        ];

        let resolved = select_user_env(&requested, &variables, &secrets);

        assert_eq!(resolved.len(), 2, "only the present requested keys");
        assert_eq!(resolved["PLAIN"].expose_secret(), "p-val");
        assert_eq!(resolved["SECRET"].expose_secret(), "s-val");
        // A requested key absent from BOTH stores is dropped (the caller logs it).
        assert!(!resolved.contains_key("MISSING"));
        // A stored key that was NOT requested is never injected.
        assert!(!resolved.contains_key("UNREQUESTED"));
    }

    #[test]
    fn select_user_env_prefers_a_secret_over_a_plaintext_of_the_same_name() {
        let variables = BTreeMap::from([("DUP".to_string(), "plain".to_string())]);
        let secrets = BTreeMap::from([("DUP".to_string(), "secret".to_string())]);
        let resolved = select_user_env(&["DUP".to_string()], &variables, &secrets);
        assert_eq!(resolved["DUP"].expose_secret(), "secret");
    }

    #[test]
    fn select_user_env_empty_request_resolves_to_empty() {
        let resolved = select_user_env(&[], &BTreeMap::new(), &BTreeMap::new());
        assert!(resolved.is_empty());
    }
}
