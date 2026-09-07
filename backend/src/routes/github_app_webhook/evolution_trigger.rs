//! `push` / `pull_request` / `release` / `repository` webhooks -> reconcile nudge.
//!
//! FKST Evolution reacts to changes that reach a repository's trusted branch, not
//! only to issue activity. The webhook router previously dispatched `installation`,
//! `installation_repositories` and `issues` and dropped everything else, so those
//! changes reached the reconciler only on the periodic full resync — a ~10 minute
//! latency for the event class Evolution is built around.
//!
//! Like [`super::issue_trigger`], this is a thin classifier and NOT a launcher: a
//! relevant event enqueues the event's `(installation_id, repo)` hint and returns.
//! The reconciler re-reads GitHub and decides what to do. Correctness comes from
//! level-triggered reconciliation; a webhook is only a latency optimisation.
//!
//! Relevance is decided from the payload alone. Every predicate below reads fields
//! GitHub already sends — notably `repository.default_branch` — so classification
//! costs no API call. That matters because the alternative, resolving the default
//! branch per event, would add a round trip to the hot path of every push in every
//! installed repository.

use serde::Deserialize;

use super::Handled;
use crate::models::RepoRef;
use crate::state::AppState;

/// Tag prefix reserved for Evolution's own Release asset sets.
///
/// A Release Evolution published must never trigger the full rebuild that
/// publishing a release otherwise means, or the system drives itself in a loop:
/// rebuild -> publish -> rebuild.
pub(super) const EVOLUTION_TAG_PREFIX: &str = "fkst-evolution/";

#[derive(Debug, Deserialize)]
pub(super) struct RepoPayload {
    pub owner: OwnerPayload,
    pub name: String,
    /// GitHub sends the repository's *current* default branch on every event,
    /// which is what makes the `@default` sentinel resolvable without a lookup.
    #[serde(default)]
    pub default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OwnerPayload {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InstallationPayload {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct PushEvent {
    /// Fully qualified, e.g. `refs/heads/develop`.
    pub r#ref: String,
    pub repository: RepoPayload,
    pub installation: InstallationPayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct PullRequestEvent {
    pub action: String,
    pub pull_request: PullRequestPayload,
    pub repository: RepoPayload,
    pub installation: InstallationPayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct PullRequestPayload {
    pub number: i64,
    pub base: BasePayload,
    #[serde(default)]
    pub merged: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct BasePayload {
    pub r#ref: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseEvent {
    pub action: String,
    pub release: ReleasePayload,
    pub repository: RepoPayload,
    pub installation: InstallationPayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleasePayload {
    pub tag_name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RepositoryEvent {
    pub action: String,
    pub repository: RepoPayload,
    pub installation: InstallationPayload,
    #[serde(default)]
    pub changes: Option<RepositoryChanges>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RepositoryChanges {
    #[serde(default)]
    pub default_branch: Option<serde_json::Value>,
}

/// Strip `refs/heads/` from a push ref, returning `None` for tag and note refs.
///
/// A tag push carries `refs/tags/…` and can never be the default branch, so it is
/// filtered here rather than compared and accidentally matched.
pub(super) fn branch_of_ref(git_ref: &str) -> Option<&str> {
    git_ref.strip_prefix("refs/heads/")
}

/// A push matters when it landed on the repository's current default branch.
///
/// Pushes to other refs are ignored: Evolution generates from a commit that has
/// reached the trusted branch, and a topic-branch push is covered by the
/// pull_request path instead.
pub(super) fn push_is_relevant(git_ref: &str, default_branch: Option<&str>) -> bool {
    match (branch_of_ref(git_ref), default_branch) {
        (Some(branch), Some(default)) => branch == default,
        // Without a default branch in the payload the safe choice is to nudge:
        // a spurious reconcile is cheap and idempotent, whereas a dropped one
        // leaves the trusted head uncovered until the next full resync.
        (Some(_), None) => true,
        _ => false,
    }
}

/// Whether a `pull_request` action can change what Evolution should do.
///
/// `closed` is relevant in both directions but for different reasons: a merged PR
/// nudges canonical reconciliation, and an unmerged one may need an active preview
/// status cleared. Both are the same repository hint, so they are not distinguished
/// here — the reconciler reads the PR state itself.
pub(super) fn pull_request_is_relevant(
    action: &str,
    base_ref: &str,
    default_branch: Option<&str>,
) -> bool {
    let relevant_action = matches!(
        action,
        "opened" | "reopened" | "synchronize" | "ready_for_review" | "edited" | "closed"
    );
    if !relevant_action {
        return false;
    }
    match default_branch {
        Some(default) => base_ref == default,
        None => true,
    }
}

/// A published release triggers a full rebuild — unless Evolution published it.
///
/// Evolution's own namespaced Release sets are excluded, without which the
/// two-phase publication protocol would re-trigger the cycle that produced it.
pub(super) fn release_is_relevant(action: &str, tag_name: &str) -> bool {
    action == "published" && !tag_name.starts_with(EVOLUTION_TAG_PREFIX)
}

/// A `repository` event matters only when the default branch itself moved.
///
/// That is the event the `@default` sentinel exists for: a repository that renames
/// or switches its default branch must stop reconciling against the old one.
pub(super) fn repository_is_relevant(action: &str, changes: Option<&RepositoryChanges>) -> bool {
    action == "edited" && changes.is_some_and(|c| c.default_branch.is_some())
}

/// Enqueue a repository hint, or ignore when the reconciler is not running.
fn nudge(
    state: &AppState,
    repository: &RepoPayload,
    installation_id: i64,
    event: &str,
    detail: &str,
) -> Handled {
    let Some(reconciler) = &state.reconciler else {
        return Handled::Ignored;
    };
    let repo = RepoRef {
        owner: repository.owner.login.clone(),
        name: repository.name.clone(),
    };
    tracing::info!(
        installation = installation_id,
        owner = %repo.owner,
        name = %repo.name,
        event = %event,
        detail = %detail,
        "webhook: enqueuing repo for reconcile"
    );
    reconciler.enqueue((installation_id, repo));
    Handled::Reconciled
}

/// Classify a `push` event.
pub(super) async fn classify_push(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: PushEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse push event: {e}"))?;
    if !push_is_relevant(&event.r#ref, event.repository.default_branch.as_deref()) {
        return Ok(Handled::Ignored);
    }
    Ok(nudge(
        state,
        &event.repository,
        event.installation.id,
        "push",
        &event.r#ref,
    ))
}

/// Classify a `pull_request` event.
pub(super) async fn classify_pull_request(
    state: &AppState,
    body: &[u8],
) -> Result<Handled, String> {
    let event: PullRequestEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse pull_request event: {e}"))?;
    if !pull_request_is_relevant(
        &event.action,
        &event.pull_request.base.r#ref,
        event.repository.default_branch.as_deref(),
    ) {
        return Ok(Handled::Ignored);
    }
    // A merged pull request produces BOTH this event and a `push`. Both converge
    // on the same repository hint, and the reconciler is level-triggered, so the
    // pair yields one reconcile pass rather than two work items.
    let detail = format!(
        "#{} {} merged={}",
        event.pull_request.number, event.action, event.pull_request.merged
    );
    Ok(nudge(
        state,
        &event.repository,
        event.installation.id,
        "pull_request",
        &detail,
    ))
}

/// Classify a `release` event.
pub(super) async fn classify_release(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: ReleaseEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse release event: {e}"))?;
    if !release_is_relevant(&event.action, &event.release.tag_name) {
        return Ok(Handled::Ignored);
    }
    Ok(nudge(
        state,
        &event.repository,
        event.installation.id,
        "release",
        &event.release.tag_name,
    ))
}

/// Classify a `repository` event.
pub(super) async fn classify_repository(state: &AppState, body: &[u8]) -> Result<Handled, String> {
    let event: RepositoryEvent =
        serde_json::from_slice(body).map_err(|e| format!("parse repository event: {e}"))?;
    if !repository_is_relevant(&event.action, event.changes.as_ref()) {
        return Ok(Handled::Ignored);
    }
    Ok(nudge(
        state,
        &event.repository,
        event.installation.id,
        "repository",
        "default_branch changed",
    ))
}

#[cfg(test)]
#[path = "evolution_trigger_tests.rs"]
mod tests;
