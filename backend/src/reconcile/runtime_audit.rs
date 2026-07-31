//! The delete-side audit facts the planner hands to the executor.
//!
//! Kept out of [`super::desired`] because it is not desired state: it is
//! evidence ABOUT an observed runtime, captured at the one moment it is still
//! knowable, so the record written when that runtime disappears can say who it
//! belonged to and which incarnation it was.

use k8s_openapi::chrono::{DateTime, Utc};

use super::desired::{LivePod, SessionRegistration};

/// Everything a DELETE-side lifecycle record needs about the runtime an action
/// is about to remove, captured by the planner from what it already holds
/// (issue #5673, epic `AUD-05`).
///
/// It exists because the executor cannot recover any of it after the fact: by
/// the time a kill runs, the runtime is being deleted and its registration may
/// already be gone. Two things would otherwise be lost:
///
/// - **The incarnation.** A session id is derived from its trigger issue and is
///   therefore identical across a kill/respawn cycle, as is the Kubernetes Pod
///   name built from it. `created_at` is what makes the second runtime's
///   deterministic event ids differ from the first's instead of being discarded
///   as PostHog duplicates.
/// - **The correlation.** A `Kill { Idle }` / `Kill { ConfigChanged }` is
///   planned with the full [`SessionRegistration`] in hand; an orphan kill has
///   only the runtime's own durable attribution stamp. Both are real evidence,
///   and dropping them would make every deletion unfilterable by repository or
///   creator while creations remain filterable.
///
/// Nothing here is ever an authorization input — it is display and correlation
/// data, exactly like the runtime stamp it is partly recovered from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeAudit {
    /// When the observed runtime came into existence: the incarnation key.
    pub created_at: Option<DateTime<Utc>>,
    /// The App installation the session belongs to. Known only from a
    /// registration — a runtime stamp does not carry it.
    pub installation_id: Option<i64>,
    /// The trigger issue the runtime was launched for.
    pub trigger_issue: Option<i64>,
    /// The effective creator's GitHub id. `None` is legitimate (an
    /// assignee-derived creator has none) as well as simply unknown.
    pub creator_id: Option<i64>,
    /// The effective creator's normalized GitHub login.
    pub creator_login: Option<String>,
    /// The trigger issue author's GitHub id.
    pub trigger_author_id: Option<i64>,
    /// The trigger issue author's normalized GitHub login.
    pub trigger_author_login: Option<String>,
}

impl RuntimeAudit {
    /// What a matched registration plus its observed runtime know. The
    /// registration is the authoritative attribution source here: it was parsed
    /// from the trigger issue this pass, whereas a stamp is only as good as
    /// whatever wrote it.
    pub fn from_registration(reg: &SessionRegistration, pod: Option<&LivePod>) -> Self {
        let identity = crate::runtime_identity::RuntimeIdentityMetadata::new(
            reg.creator_id,
            &reg.creator_login,
            reg.trigger_author_id,
            &reg.trigger_author_login,
        );
        Self {
            created_at: pod.map(|pod| pod.created_at),
            installation_id: Some(reg.installation_id),
            trigger_issue: Some(reg.trigger_issue),
            creator_id: identity.creator_id,
            creator_login: non_empty(identity.creator_login),
            trigger_author_id: Some(identity.trigger_author_id),
            trigger_author_login: non_empty(identity.trigger_author_login),
        }
    }

    /// What an ORPHAN runtime knows about itself: its own durable attribution
    /// stamp and nothing else. A runtime that predates the stamp yields no
    /// attribution at all rather than a guess from the repository owner or the
    /// App identity — that guess is exactly what `unknown_legacy` exists to
    /// prevent.
    pub fn from_observed(pod: &LivePod) -> Self {
        Self {
            created_at: Some(pod.created_at),
            installation_id: None,
            trigger_issue: (pod.trigger_issue != 0).then_some(pod.trigger_issue),
            creator_id: pod.identity.creator_id,
            creator_login: pod.identity.creator_login.clone(),
            trigger_author_id: pod.identity.trigger_author_id,
            trigger_author_login: pod.identity.trigger_author_login.clone(),
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
