//! `issues` webhook -> Model B reconcile nudge (issue #359 §4.3, PR6).
//!
//! The webhook is a thin classifier, NOT a launcher. A reconcile-relevant `issues`
//! action (open / reopen / close / label / unlabel) is a level-based *nudge*: it
//! enqueues the event's `(installation_id, repo)` onto the reconcile queue and
//! returns. The reconciler ([`crate::reconcile`]) then re-reads the repo's open
//! trigger issues + live pods and decides spawn-vs-kill itself, so the webhook does
//! NOT inspect labels or the issue body — it only needs the repo identity plus the
//! installation id that scopes the App token. A missing reconciler (`FKST_POD_DISPATCH`
//! off) or a non-reconcile action is simply ignored; a malformed body is an `Err`
//! the caller maps to a 202.

use serde::Deserialize;

use super::Handled;
use crate::models::RepoRef;
use crate::state::AppState;

/// The subset of a GitHub `issues` webhook payload the nudge consumes: the
/// `action` (to gate relevance), the repo (`owner/name`, the reconcile target),
/// the installation id (scopes the App token), and the issue number (traceability
/// only). GitHub sends far more; serde ignores the rest.
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

/// Whether an `issues` action can change a repo's desired session state and so
/// warrants a reconcile nudge: an issue being opened / reopened / closed, or a
/// label being added / removed (a trigger label, or a work label the reconciler
/// gates on). Everything else (`edited`, `assigned`, ...) is inert.
fn is_reconcile_relevant(action: &str) -> bool {
    matches!(
        action,
        "opened" | "reopened" | "closed" | "labeled" | "unlabeled"
    )
}

/// Classify an `issues` event and, when it is reconcile-relevant AND Model B is
/// live, enqueue its `(installation_id, repo)` onto the reconcile queue.
///
/// Returns [`Handled::Reconciled`] when a nudge was enqueued, [`Handled::Ignored`]
/// when there is no reconciler (dispatch off) or the action is not relevant, and
/// `Err` for a malformed body (the caller logs it + returns a 202).
pub(super) async fn classify_and_enqueue(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: IssuesEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse issues event: {e}"))?;

    // No reconciler => Model B is not live (FKST_POD_DISPATCH off / loop not
    // spawned): there is nothing to nudge, so acknowledge and ignore.
    let Some(reconciler) = &state.reconciler else {
        return Ok(Handled::Ignored);
    };
    if !is_reconcile_relevant(&event.action) {
        return Ok(Handled::Ignored);
    }

    let repo = RepoRef {
        owner: event.repository.owner.login.clone(),
        name: event.repository.name.clone(),
    };
    let installation_id = event.installation.id;
    tracing::info!(
        installation = installation_id,
        owner = %repo.owner,
        name = %repo.name,
        action = %event.action,
        issue = event.issue.number,
        "webhook: enqueuing repo for reconcile"
    );
    reconciler.enqueue((installation_id, repo));
    Ok(Handled::Reconciled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::reconcile::{reconcile_channel, ReconcileHandle};

    fn issues_body(action: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "action": action,
            "issue": { "number": 7 },
            "repository": { "owner": { "login": "acme" }, "name": "site" },
            "installation": { "id": 42 }
        }))
        .expect("serialize")
    }

    fn state(reconciler: Option<ReconcileHandle>) -> AppState {
        AppState {
            config: Config::default(),
            github_app: None,
            github_app_webhook_secret: None,
            reconciler,
        }
    }

    #[test]
    fn relevance_covers_the_state_changing_actions_only() {
        for yes in ["opened", "reopened", "closed", "labeled", "unlabeled"] {
            assert!(is_reconcile_relevant(yes), "{yes} must be relevant");
        }
        for no in ["edited", "assigned", "milestoned", "deleted", ""] {
            assert!(!is_reconcile_relevant(no), "{no} must be irrelevant");
        }
    }

    #[tokio::test]
    async fn a_relevant_action_enqueues_and_reports_reconciled() {
        let (handle, mut rx) = reconcile_channel(8);
        let st = state(Some(handle));
        for action in ["opened", "reopened", "closed", "labeled", "unlabeled"] {
            let handled = classify_and_enqueue(&st, &issues_body(action))
                .await
                .expect("ok");
            assert_eq!(handled.as_str(), "reconciled", "{action} must nudge");
            let got = rx.try_recv().expect("one key enqueued");
            assert_eq!(
                got,
                (
                    42,
                    RepoRef {
                        owner: "acme".to_string(),
                        name: "site".to_string()
                    }
                )
            );
        }
    }

    #[tokio::test]
    async fn an_irrelevant_action_is_ignored_and_does_not_enqueue() {
        let (handle, mut rx) = reconcile_channel(8);
        let st = state(Some(handle));
        let handled = classify_and_enqueue(&st, &issues_body("edited"))
            .await
            .expect("ok");
        assert_eq!(handled.as_str(), "ignored");
        assert!(
            rx.try_recv().is_err(),
            "an irrelevant action must not enqueue"
        );
    }

    #[tokio::test]
    async fn without_a_reconciler_a_relevant_action_is_ignored() {
        // Model B not live: a valid, relevant event is acknowledged (Ignored),
        // never enqueued (there is no queue).
        let handled = classify_and_enqueue(&state(None), &issues_body("opened"))
            .await
            .expect("ok");
        assert_eq!(handled.as_str(), "ignored");
    }

    #[tokio::test]
    async fn a_malformed_body_is_an_error_not_a_panic() {
        let (handle, _rx) = reconcile_channel(8);
        let err = classify_and_enqueue(&state(Some(handle)), b"not json")
            .await
            .expect_err("malformed body must Err");
        assert!(
            err.contains("parse issues event"),
            "names the boundary: {err}"
        );
    }

    #[test]
    fn issues_event_parses_a_representative_payload() {
        let event: IssuesEvent = serde_json::from_slice(&issues_body("opened")).expect("parses");
        assert_eq!(event.action, "opened");
        assert_eq!(event.issue.number, 7);
        assert_eq!(event.repository.owner.login, "acme");
        assert_eq!(event.repository.name, "site");
        assert_eq!(event.installation.id, 42);
    }
}
