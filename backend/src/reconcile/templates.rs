//! Version-aware issue-template ensure: keep every reconciled repo's
//! `.github/ISSUE_TEMPLATE/` at the bundled [`FKST_ISSUE_TEMPLATES_VERSION`].
//!
//! Called best-effort from the per-repo driver on EVERY reconcile (which fires on
//! every repo-touching webhook event, the periodic sweep, and the full-resync).
//! To avoid a GitHub round-trip on the overwhelming majority of reconciles it is
//! GATED by an in-memory [`EnsuredTemplates`] map: a repo is re-checked only when
//! the bundled version is newer than what we last recorded for it, or the record
//! is older than [`ENSURED_TEMPLATES_TTL`]. A current repo is a cheap no-op after
//! the first check (idempotent).
//!
//! Error discipline: this NEVER returns an error and NEVER aborts the reconcile.
//! A read/install failure is logged and NOT recorded, so the next reconcile
//! retries. A deferred install ([`TemplateInstallOutcome::Deferred`] — the PR is
//! open but this App could not merge it) IS recorded: the PR stays open for the
//! repo's own merge flow and the next attempt waits for the TTL, so a protected
//! repo never sees PR churn. The installation token is minted inside
//! [`IssueTemplateGithub`] with a least-privilege permission set and is never
//! logged.

use std::time::Instant;

use crate::github_app::templates::FKST_ISSUE_TEMPLATES_VERSION;
use crate::github_app::{IssueTemplateGithub, TemplateInstallOutcome};

use super::{EnsuredMark, EnsuredTemplates, RepoKey, ENSURED_TEMPLATES_TTL};

/// Whether a fresh GitHub check is due for `key` at `now`. Due when we have never
/// checked it, the bundled version is newer than the recorded one, or the record
/// is older than [`ENSURED_TEMPLATES_TTL`]. Poison-safe (a panic elsewhere while
/// the lock is held never wedges the reconciler).
fn check_due(ensured: &EnsuredTemplates, key: &RepoKey, now: Instant) -> bool {
    let map = ensured.lock().unwrap_or_else(|e| e.into_inner());
    match map.get(key) {
        None => true,
        Some(mark) => {
            FKST_ISSUE_TEMPLATES_VERSION > mark.version
                || now.duration_since(mark.checked_at) >= ENSURED_TEMPLATES_TTL
        }
    }
}

/// Record that `key` was handled at `version` as of `now` — either confirmed
/// installed or left as an open, deferred install PR (both gate the next check
/// to the TTL). Poison-safe.
fn record(ensured: &EnsuredTemplates, key: &RepoKey, version: u32, now: Instant) {
    ensured.lock().unwrap_or_else(|e| e.into_inner()).insert(
        key.clone(),
        EnsuredMark {
            version,
            checked_at: now,
        },
    );
}

/// Best-effort, NON-failing ensure that `owner_repo`'s issue templates are at the
/// bundled version. Gated to one GitHub round-trip per repo per `(version, TTL)`.
/// Never returns an error; never records on failure (so it retries next reconcile).
/// Returns true when this pass INSTALLED (or updated) the templates, which is the
/// signal the caller uses to run the equally rare label bootstrap alongside it.
pub async fn ensure_issue_templates(
    key: RepoKey,
    owner_repo: &str,
    github: &(dyn IssueTemplateGithub + '_),
    ensured: &EnsuredTemplates,
) -> bool {
    // Cheap no-op after the first check within the (version, TTL) window.
    if !check_due(ensured, &key, Instant::now()) {
        return false;
    }

    let installed = match github.installed_templates_version(owner_repo).await {
        Ok(version) => version,
        Err(error) => {
            tracing::warn!(
                owner_repo = %owner_repo,
                error = %error,
                "issue-templates: version read failed; will retry next reconcile"
            );
            return false;
        }
    };

    if installed >= FKST_ISSUE_TEMPLATES_VERSION {
        tracing::debug!(
            owner_repo = %owner_repo,
            installed,
            "issue-templates: already current"
        );
        record(ensured, &key, installed, Instant::now());
        return false;
    }

    tracing::info!(
        owner_repo = %owner_repo,
        installed,
        target = FKST_ISSUE_TEMPLATES_VERSION,
        "issue-templates: outdated/missing; opening install PR"
    );
    match github
        .install_templates(owner_repo, FKST_ISSUE_TEMPLATES_VERSION)
        .await
    {
        Ok(TemplateInstallOutcome::Merged) => {
            tracing::info!(
                owner_repo = %owner_repo,
                version = FKST_ISSUE_TEMPLATES_VERSION,
                "issue-templates: installed via merged PR"
            );
            record(ensured, &key, FKST_ISSUE_TEMPLATES_VERSION, Instant::now());
            return true;
        }
        Ok(TemplateInstallOutcome::Deferred { pull }) => {
            // The install PR is open but this App could not merge it (the base
            // branch gates merges, or the branch is held by a PR it did not
            // write). RECORD the target version anyway: the mark means "handled
            // as of now", gating the next attempt to one per TTL instead of one
            // per reconcile — the churn loop of issue #5578. The TTL re-check's
            // version read decides whether the PR landed in the meantime.
            tracing::info!(
                owner_repo = %owner_repo,
                version = FKST_ISSUE_TEMPLATES_VERSION,
                pull,
                "issue-templates: install PR left open; re-checked after TTL"
            );
            record(ensured, &key, FKST_ISSUE_TEMPLATES_VERSION, Instant::now());
            return false;
        }
        Err(error) => tracing::warn!(
            owner_repo = %owner_repo,
            error = %error,
            "issue-templates: install failed; NOT recording (retries next reconcile)"
        ),
    }
    false
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;

    use crate::github_app::GithubAppError;
    use crate::models::RepoRef;

    use super::super::new_ensured_templates;
    use super::*;

    /// A fake [`IssueTemplateGithub`] that records call counts and can be told to
    /// fail either operation or report a deferred (unmergeable) install PR, so the ensure
    /// gate + orchestration are testable without a live GitHub.
    struct FakeTemplates {
        installed: u32,
        version_calls: AtomicUsize,
        install_calls: AtomicUsize,
        fail_version: bool,
        fail_install: bool,
        deferred: bool,
    }

    impl FakeTemplates {
        fn new(installed: u32) -> Self {
            Self {
                installed,
                version_calls: AtomicUsize::new(0),
                install_calls: AtomicUsize::new(0),
                fail_version: false,
                fail_install: false,
                deferred: false,
            }
        }

        fn with_install_failure(installed: u32) -> Self {
            Self {
                fail_install: true,
                ..Self::new(installed)
            }
        }

        fn with_version_failure() -> Self {
            Self {
                fail_version: true,
                ..Self::new(0)
            }
        }

        fn with_deferred(installed: u32) -> Self {
            Self {
                deferred: true,
                ..Self::new(installed)
            }
        }

        fn version_calls(&self) -> usize {
            self.version_calls.load(Ordering::SeqCst)
        }

        fn install_calls(&self) -> usize {
            self.install_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl IssueTemplateGithub for FakeTemplates {
        async fn installed_templates_version(
            &self,
            _owner_repo: &str,
        ) -> Result<u32, GithubAppError> {
            self.version_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_version {
                return Err(GithubAppError::Http("boom".to_string()));
            }
            Ok(self.installed)
        }

        async fn install_templates(
            &self,
            _owner_repo: &str,
            _target_version: u32,
        ) -> Result<TemplateInstallOutcome, GithubAppError> {
            self.install_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_install {
                return Err(GithubAppError::Http("boom".to_string()));
            }
            if self.deferred {
                return Ok(TemplateInstallOutcome::Deferred { pull: 7 });
            }
            Ok(TemplateInstallOutcome::Merged)
        }
    }

    fn key() -> RepoKey {
        (
            42,
            RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
        )
    }

    /// `installed` at least one below the bundled const (0 when the const is 1),
    /// so "outdated" tests are meaningful regardless of the current version.
    fn outdated() -> u32 {
        FKST_ISSUE_TEMPLATES_VERSION.saturating_sub(1)
    }

    fn mark_at(ensured: &EnsuredTemplates, k: &RepoKey, version: u32, checked_at: Instant) {
        ensured.lock().unwrap().insert(
            k.clone(),
            EnsuredMark {
                version,
                checked_at,
            },
        );
    }

    #[tokio::test]
    async fn skips_when_already_current() {
        let fake = FakeTemplates::new(FKST_ISSUE_TEMPLATES_VERSION);
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 0, "current repo must not install");
        assert_eq!(
            ensured.lock().unwrap().get(&key()).unwrap().version,
            FKST_ISSUE_TEMPLATES_VERSION,
            "current version is recorded"
        );
    }

    #[tokio::test]
    async fn installs_when_outdated() {
        let fake = FakeTemplates::new(outdated());
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 1, "outdated repo installs once");
        assert_eq!(
            ensured.lock().unwrap().get(&key()).unwrap().version,
            FKST_ISSUE_TEMPLATES_VERSION,
            "records the target version after install"
        );
    }

    #[tokio::test]
    async fn installs_when_missing() {
        // A missing file surfaces as installed version 0.
        let fake = FakeTemplates::new(0);
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 1, "missing templates trigger install");
    }

    #[tokio::test]
    async fn gate_skips_second_call_same_version() {
        let fake = FakeTemplates::new(FKST_ISSUE_TEMPLATES_VERSION);
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(
            fake.version_calls(),
            1,
            "the second call is a gated no-op (no extra round-trip)"
        );
    }

    #[tokio::test]
    async fn gate_rechecks_after_ttl() {
        let fake = FakeTemplates::new(FKST_ISSUE_TEMPLATES_VERSION);
        let ensured = new_ensured_templates();
        // Pre-seed a stale record (checked > TTL ago) => the gate must re-check.
        let stale = Instant::now() - (ENSURED_TEMPLATES_TTL + Duration::from_secs(3600));
        mark_at(&ensured, &key(), FKST_ISSUE_TEMPLATES_VERSION, stale);
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.version_calls(), 1, "a stale record forces a re-read");
    }

    #[tokio::test]
    async fn failure_does_not_record_and_retries() {
        let fake = FakeTemplates::with_install_failure(outdated());
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert!(
            ensured.lock().unwrap().get(&key()).is_none(),
            "a failed install must NOT be recorded"
        );
        // Next reconcile re-attempts (not gated away, since nothing was recorded).
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 2, "install is retried after failure");
    }

    #[tokio::test]
    async fn deferred_records_and_gates_until_ttl() {
        // A repo whose install PR this App cannot merge must get ONE attempt
        // per (version, TTL) — not one churned PR per reconcile.
        let fake = FakeTemplates::with_deferred(outdated());
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 1, "one install attempt");
        assert_eq!(
            ensured.lock().unwrap().get(&key()).unwrap().version,
            FKST_ISSUE_TEMPLATES_VERSION,
            "a deferred PR is recorded like an install (gates to the TTL)"
        );
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(
            fake.version_calls(),
            1,
            "the next reconcile is a gated no-op — no churned replacement PR"
        );
    }

    #[tokio::test]
    async fn deferred_retries_after_ttl() {
        // After the TTL the gate re-opens: the still-outdated repo gets one
        // fresh attempt (which reuses + retries the pending PR downstream).
        let fake = FakeTemplates::with_deferred(outdated());
        let ensured = new_ensured_templates();
        let stale = Instant::now() - (ENSURED_TEMPLATES_TTL + Duration::from_secs(3600));
        mark_at(&ensured, &key(), FKST_ISSUE_TEMPLATES_VERSION, stale);
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert_eq!(fake.install_calls(), 1, "a TTL-stale deferred repo retries");
    }

    #[tokio::test]
    async fn version_read_error_does_not_record() {
        let fake = FakeTemplates::with_version_failure();
        let ensured = new_ensured_templates();
        ensure_issue_templates(key(), "acme/site", &fake, &ensured).await;
        assert!(
            ensured.lock().unwrap().get(&key()).is_none(),
            "a failed version read must NOT be recorded"
        );
        assert_eq!(
            fake.install_calls(),
            0,
            "no install attempted on read error"
        );
    }

    #[test]
    fn check_due_true_when_bundled_newer_than_recorded() {
        // A fresh record at a version BELOW the bundled const is still due,
        // regardless of TTL (the version bump wins).
        let ensured = new_ensured_templates();
        let recorded = FKST_ISSUE_TEMPLATES_VERSION.saturating_sub(1);
        let now = Instant::now();
        mark_at(&ensured, &key(), recorded, now);
        if recorded < FKST_ISSUE_TEMPLATES_VERSION {
            assert!(
                check_due(&ensured, &key(), now),
                "a newer bundled version must force a re-check"
            );
        }
        // At/above the bundled version and within TTL: not due.
        mark_at(&ensured, &key(), FKST_ISSUE_TEMPLATES_VERSION, now);
        assert!(
            !check_due(&ensured, &key(), now),
            "current version within TTL is not due"
        );
    }
}
