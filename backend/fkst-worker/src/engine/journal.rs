//! The worker-side journaling glue (issue #151, increment 6c).
//!
//! The worker journals the engine's RAISED stdout lines + its lifecycle
//! transitions DIRECT to GitHub, mirroring the in-process control-plane driver
//! VERBATIM (`fkst-control-plane/src/sessions/service.rs`, the `journal_*`
//! helpers + `start_journaler`). The driver wraps the journaler behind a
//! `ServiceJournaler` alias and threads Mongo (run_key write) into bootstrap;
//! the worker is DB-free, so it uses [`fkst_journal::Journaler`] DIRECTLY and
//! does NOT stamp the run_key anywhere (the run_key survives in the committed
//! journal file). Every helper operates on `&mut Option<Journaler>` so a
//! journaling-off session (`None`) is a no-op — journaling is NEVER
//! load-bearing: every failure is logged and swallowed, and session disposition
//! is decided exclusively by the supervise loop's status reports.
//!
//! DORMANT: the controller ships a `JournalPlan` only once a later activation
//! increment emits a dispatch, so develop behaviour is byte-identical.
//!
//! Secrets (`github_token`) are `SecretString`s end to end — never logged, never
//! in `Debug`.

use std::path::Path;
use std::time::Duration;

use fkst_engine::clone::read_package_files;
use fkst_journal::parse::{parse_raised_line, ParsedLine};
use fkst_journal::{
    package_fingerprint, JournalConfig, Journaler, LifecycleEvent, ProgressSignal, SessionCtx,
    Transition,
};
use fkst_shared::protocol::ResolvedDispatch;

/// Build the per-session journaler for a dispatch, or `None` when journaling is
/// off (no plan) or there is nothing valid to journal as (no package name).
///
/// `fingerprint_root` is the FIRST cloned package dir (request order) — the
/// run's primary package, whose content fingerprints the run (mirrors the
/// driver's `cloned.package_roots.first().map(read_package_files)`).
///
/// Mirrors the driver's `start_journaler` MINUS the Mongo `set_run_key` write:
/// the worker is DB-free (and CI-forbidden from the Mongo driver), and the
/// run_key survives in the committed journal file — so it is never stamped.
pub(crate) async fn start_session_journaler(
    dispatch: &ResolvedDispatch,
    fingerprint_root: Option<&Path>,
) -> Option<Journaler> {
    let plan = dispatch.journal.as_ref()?;
    // `Journaler::start` rejects an empty/invalid package name, so a dispatch
    // with no package roots cannot be journaled — skip it rather than fail.
    let package_name = dispatch.clone_spec.package_roots.first()?.clone();

    // Reconstruct the PROCESS-level JournalConfig from the plan: github is always
    // enabled (the controller ships a plan ONLY when fully configured), the repo
    // + token are present, and the rest map field-for-field.
    let cfg = JournalConfig {
        flush_interval: Duration::from_millis(plan.flush_interval_ms),
        flush_max_batch: plan.flush_max_batch,
        github_enabled: true,
        issue_comments: plan.issue_comments,
        activity_comment_enabled: plan.activity_comment_enabled,
        cas_max_retries: plan.cas_max_retries,
        bootstrap_read_retries: plan.bootstrap_read_retries,
        github_branch: plan.github_branch.clone(),
        github_repo: Some(plan.github_repo.clone()),
        github_api_base: plan.github_api_base.clone(),
        identity_pointers: plan.identity_pointers.clone(),
        max_line_bytes: plan.max_line_bytes,
        github_token: Some(plan.github_token.clone()),
    };

    // Fingerprint the run from the primary package's content. Empty deps: repo-
    // scoped packages fold their composed deps into the fingerprinted file
    // content (an in-repo `composed.deps` file), so there is no separate dep list
    // — exactly as the driver passes `&[]`.
    let files = fingerprint_root.map(read_package_files).unwrap_or_default();
    let fp = package_fingerprint(&files, &[]);

    let ctx = SessionCtx {
        session_id: dispatch.session_id.clone(),
        package_name,
        package_fingerprint: fp,
        pod_id: Some(dispatch.worker_id.clone()),
        fencing_token: dispatch.fencing_id,
    };

    match Journaler::start(ctx, cfg).await {
        Ok(mut journaler) => {
            // Redo bootstrap: committed completed[] -> in-RAM skip-set, fail-open
            // to safe re-execution on any unreachability.
            if let Err(error) = journaler.load_skip_set().await {
                tracing::warn!(error = %error, "skip-set bootstrap failed; proceeding with an empty set");
            }
            Some(journaler)
        }
        Err(error) => {
            tracing::error!(error = %error, "journaler start failed; session proceeds unjournaled");
            None
        }
    }
}

/// Record one signal; failures are swallowed (already logged with context by the
/// journaler / store layers). Mirrors the driver's `journal_record`.
pub(crate) async fn journal_record(journaler: &mut Option<Journaler>, signal: ProgressSignal) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.record(signal).await {
            tracing::warn!(error = %error, "journal record failed (swallowed; session unaffected)");
        }
    }
}

/// Debounced/forced GitHub flush; failures are swallowed (the buffer is retained
/// and retried on the next tick). Mirrors the driver's `journal_flush`.
pub(crate) async fn journal_flush(journaler: &mut Option<Journaler>, force: bool) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.flush(force).await {
            tracing::warn!(error = %error, "journal flush failed (swallowed; retried next tick)");
        }
    }
}

/// Record a lifecycle transition and flush promptly (`force=true` — the spec's
/// lifecycle-flushes-immediately rule). Mirrors the driver's `journal_lifecycle`.
pub(crate) async fn journal_lifecycle(journaler: &mut Option<Journaler>, transition: Transition) {
    journal_record(
        journaler,
        ProgressSignal::Lifecycle(LifecycleEvent::now(transition)),
    )
    .await;
    journal_flush(journaler, true).await;
}

/// Terminal journal: record the terminal lifecycle + final forced flush; failures
/// swallowed. Mirrors the driver's `journal_finish`.
pub(crate) async fn journal_finish(journaler: &mut Option<Journaler>, transition: Transition) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.finish(LifecycleEvent::now(transition)).await {
            tracing::warn!(error = %error, "journal finish failed (swallowed; session unaffected)");
        }
    }
}

/// One engine stdout line: parse the `RAISED:` framing and journal the outcome
/// (raised event / malformed anomaly / debug-logged chatter). Mirrors the
/// driver's `journal_stdout_line` VERBATIM.
pub(crate) async fn journal_stdout_line(journaler: &mut Option<Journaler>, raw: &[u8]) {
    let Some(max_line_bytes) = journaler.as_ref().map(|j| j.config().max_line_bytes) else {
        tracing::debug!(target: "engine.stdout", len = raw.len(), "stdout line (journaling off)");
        return;
    };
    match parse_raised_line(raw, max_line_bytes) {
        ParsedLine::Raised { event_json } => {
            journal_record(journaler, ProgressSignal::Raised { event_json }).await;
            // Debounced: the journaler batches by interval / batch size.
            journal_flush(journaler, false).await;
        }
        ParsedLine::Malformed { excerpt, oversize } => {
            if let Some(j) = journaler.as_mut() {
                j.malformed_raised_total += 1;
                if oversize {
                    j.oversize_raised_total += 1;
                }
                tracing::warn!(
                    oversize,
                    malformed_raised_total = j.malformed_raised_total,
                    oversize_raised_total = j.oversize_raised_total,
                    payload_excerpt = %excerpt,
                    "malformed RAISED line (journaled as anomaly; session continues)"
                );
            }
            journal_record(
                journaler,
                ProgressSignal::Lifecycle(LifecycleEvent::now(Transition::MalformedRaised {
                    detail: excerpt,
                })),
            )
            .await;
        }
        ParsedLine::Other { excerpt } => {
            tracing::debug!(target: "engine.stdout", line_excerpt = %excerpt, "engine stdout chatter");
        }
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
