//! Synchronous submit-time pre-flight validation (#179, gap **G3**).
//!
//! Before any session is spawned, [`validate_submission`] checks a submission
//! comprehensively and, on failure, returns a [`SubmissionErrors`] that lists
//! **every** problem at once (never first-fail) so the caller can fix all of them
//! in one edit cycle. Three classes:
//!
//! 1. **Issue format** — field-level errors the issue-sourced submit path seeds
//!    (empty for the inline path).
//! 2. **Package correctness** — each requested package exists at
//!    `<repo>/.fkst/packages/<name>/` with a valid entry file
//!    `departments/<name>/main.lua`.
//! 3. **Ornn availability** — each pin resolves in the Ornn catalog (the
//!    skill/skillset exists, the version is available and not deprecated-only,
//!    and the expanded closure has no version conflict).
//!
//! **Chosen approach — Contents API, NOT a shallow clone.** Package correctness
//! is verified with `GET /repos/.../contents/...` (via [`ContentsReader`]), which
//! avoids cloning the repo and creating a temp dir for what is a read-only
//! existence check: it is strictly cheaper, has no filesystem side effects, and
//! never starts an engine run. This is why this pre-flight reuses the App Contents
//! helper rather than `fkst-engine`'s `clone_repo_packages`.
//!
//! **Engine rules by reference.** The name rule (`^[A-Za-z0-9_-]+$`), the
//! reserved name `"host"`, and the `departments/<name>/main.lua` entry-file rule
//! are applied via thin `pub` re-exports in `fkst-engine`
//! ([`is_valid_package_name`], [`RESERVED_PACKAGE_NAME`], [`is_department_main`])
//! — never re-derived here — so this stays in lock-step with the engine.
//!
//! **Secret hygiene.** This module only READS repo contents and the Ornn catalog.
//! It never echoes the goal prompt, any secret, or any env value — in the 422
//! body or in logs. Package names, pin names/versions, and field names are
//! non-sensitive and may appear in errors.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use secrecy::SecretString;
use serde::Serialize;

use fkst_engine::{is_department_main, is_valid_package_name, RESERVED_PACKAGE_NAME};
use fkst_shared::models::RepoRef;

use crate::auth::AuthContext;
use crate::github_app::{ContentsReader, GithubAppError, GithubAppTokens};
use crate::ornn::types::OrnnSkillPin;
use crate::ornn::OrnnClient;

use super::preflight_ornn::check_ornn;

/// Max number of concurrent Contents reads in flight across packages. With
/// `packages` bounded `1..=16` (`validate_goal_fields`) and ≤2 reads per package,
/// the total call count is ≤32; this window keeps the GitHub REST burst small
/// while still overlapping the per-package round-trips.
const PACKAGE_CHECK_CONCURRENCY: usize = 4;

/// One issue-format field error: which `field` failed and a human `message`.
/// Seeded by the issue-sourced submit/parse path; empty for the inline path.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// One package-correctness failure: the package `name` and the `reason`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackageError {
    pub name: String,
    pub reason: String,
}

/// One Ornn-availability failure: the pin `kind` (`"skill"`/`"skillset"`), the
/// pin `name`, the requested `version`, and the `reason`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinError {
    pub kind: String,
    pub name: String,
    pub version: String,
    pub reason: String,
}

/// Aggregated pre-flight failures, grouped by class. Rendered as a single
/// **HTTP 422** enumerating EVERY entry (never first-fail).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SubmissionErrors {
    pub issue_format: Vec<FieldError>,
    pub packages: Vec<PackageError>,
    pub ornn: Vec<PinError>,
}

impl SubmissionErrors {
    /// True when no class carries any failure (the submission passed).
    pub fn is_empty(&self) -> bool {
        self.issue_format.is_empty() && self.packages.is_empty() && self.ornn.is_empty()
    }

    /// Total number of accumulated failures across all classes.
    fn total(&self) -> usize {
        self.issue_format.len() + self.packages.len() + self.ornn.len()
    }
}

/// The 422 JSON body: a stable `error` code, a human `message`, and the full
/// structured breakdown so a UI can map each entry back to its source.
#[derive(Debug, Serialize)]
struct SubmissionErrorBody {
    error: &'static str,
    message: String,
    #[serde(flatten)]
    errors: SubmissionErrors,
}

impl IntoResponse for SubmissionErrors {
    fn into_response(self) -> Response {
        // Log only counts — never the field/package/pin detail beyond a count
        // (the entries are non-sensitive, but the count is all the operator
        // needs and keeps the log line bounded).
        tracing::debug!(
            issue_format = self.issue_format.len(),
            packages = self.packages.len(),
            ornn = self.ornn.len(),
            "submit pre-flight rejected a submission"
        );
        let message = format!(
            "submission failed pre-flight validation with {} error(s)",
            self.total()
        );
        let body = SubmissionErrorBody {
            error: "submission_invalid",
            message,
            errors: self,
        };
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}

/// Validate a submission before it is placed. Read-only: it performs only
/// read-only GitHub (Contents) and Ornn (catalog) calls and mutates nothing.
///
/// `issue_format_errors` is seeded by the caller (the issue-sourced submit path
/// supplies field errors; the inline path passes empty). `repo` is the persisted
/// [`RepoRef`] (`{ owner, name }`); `owner_repo` for the App helper is composed as
/// `"{owner}/{name}"`. All three check classes run and their failures accumulate
/// into ONE [`SubmissionErrors`]; the result is `Err` iff that is non-empty.
///
/// `ctx.user_access_token` is forwarded to Ornn so visibility is honored; it is
/// SECRET and never logged. The goal prompt is never read or echoed here.
pub async fn validate_submission(
    ctx: &AuthContext,
    github_app: &GithubAppTokens,
    repo: &RepoRef,
    package_names: &[String],
    ornn_skills: &[OrnnSkillPin],
    issue_format_errors: Vec<FieldError>,
    ornn: &OrnnClient,
) -> Result<(), SubmissionErrors> {
    // The package check depends on the `ContentsReader` ABSTRACTION, not the
    // concrete App service, so the inner driver is unit-testable against a fake.
    let reader: Arc<dyn ContentsReader> = Arc::new(github_app.clone());
    validate_submission_with(
        reader.as_ref(),
        ctx.user_access_token.as_ref(),
        repo,
        package_names,
        ornn_skills,
        issue_format_errors,
        ornn,
    )
    .await
}

/// Run ONLY the Ornn-availability class (plus any seeded issue-format errors),
/// for the degraded path where the GitHub App is not configured and the
/// package-correctness Contents check cannot run. The engine's own spawn-time
/// `.fkst/packages/<name>/` resolution (#115) remains the package backstop, so
/// skipping the Contents check here only loses the EARLY surface, never the
/// guarantee. Read-only; returns `Err` iff non-empty.
///
/// `token` is the caller's NyxID token (visibility-honoring), never logged.
pub async fn validate_ornn_availability(
    ctx: &AuthContext,
    ornn_skills: &[OrnnSkillPin],
    issue_format_errors: Vec<FieldError>,
    ornn: &OrnnClient,
) -> Result<(), SubmissionErrors> {
    let errors = SubmissionErrors {
        issue_format: issue_format_errors,
        ornn: check_ornn(ornn, ctx.user_access_token.as_ref(), ornn_skills).await,
        ..SubmissionErrors::default()
    };
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The injectable core of [`validate_submission`]: runs the package + Ornn checks
/// against the supplied `reader`/`token`, accumulating EVERY failure into one
/// [`SubmissionErrors`]. Read-only; returns `Err` iff non-empty. Split from the
/// public entry point so tests can drive it with a fake [`ContentsReader`].
#[allow(clippy::too_many_arguments)]
async fn validate_submission_with(
    reader: &dyn ContentsReader,
    token: Option<&SecretString>,
    repo: &RepoRef,
    package_names: &[String],
    ornn_skills: &[OrnnSkillPin],
    issue_format_errors: Vec<FieldError>,
    ornn: &OrnnClient,
) -> Result<(), SubmissionErrors> {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);

    let mut errors = SubmissionErrors {
        issue_format: issue_format_errors,
        ..SubmissionErrors::default()
    };

    // Package correctness via the Contents API (no clone/temp dir/engine run).
    match check_packages(reader, &owner_repo, package_names).await {
        Ok(package_errors) => errors.packages = package_errors,
        Err(PackagePreflightAbort::InstallBlocked(reason)) => {
            // App not installed / awaiting approval: a whole-repo blocker, not a
            // per-package fault. Surface it as a single package-class entry that
            // reuses the existing install-hint URL; do NOT fabricate a URL.
            errors.packages.push(PackageError {
                name: owner_repo.clone(),
                reason,
            });
        }
    }

    // Ornn availability (after the cheap format check already ran upstream).
    errors.ornn = check_ornn(ornn, token, ornn_skills).await;

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Package correctness (Contents API)
// ---------------------------------------------------------------------------

/// A whole-submission abort raised by the package check: the App is not installed
/// (or its permission is awaiting approval) on the repo, so per-package checks are
/// moot. Carries the actionable install-hint reason.
#[derive(Debug)]
enum PackagePreflightAbort {
    InstallBlocked(String),
}

/// Check every requested package against the repo's `.fkst/packages/` tree via
/// the Contents API, accumulating one [`PackageError`] per faulty package (never
/// first-fail). Bounds concurrency to [`PACKAGE_CHECK_CONCURRENCY`] in-flight
/// reads. An App-not-installed / awaiting-approval outcome short-circuits to
/// [`PackagePreflightAbort::InstallBlocked`] (a repo-wide blocker).
async fn check_packages(
    reader: &dyn ContentsReader,
    owner_repo: &str,
    package_names: &[String],
) -> Result<Vec<PackageError>, PackagePreflightAbort> {
    // Limit how many package checks run at once. A JoinSet would need `'static`
    // futures; the names are few (≤16) so a simple bounded-chunk drive keeps the
    // burst small without spawning or cloning the reader per task.
    let mut errors: Vec<PackageError> = Vec::new();
    for chunk in package_names.chunks(PACKAGE_CHECK_CONCURRENCY) {
        let mut futures = Vec::with_capacity(chunk.len());
        for name in chunk {
            futures.push(check_one_package(reader, owner_repo, name));
        }
        // Await this bounded batch; each future yields per-package outcome.
        for result in join_bounded(futures).await {
            match result {
                PackageCheck::Ok => {}
                PackageCheck::Error(err) => errors.push(err),
                PackageCheck::InstallBlocked(reason) => {
                    return Err(PackagePreflightAbort::InstallBlocked(reason));
                }
            }
        }
    }
    Ok(errors)
}

/// Per-package outcome.
enum PackageCheck {
    Ok,
    Error(PackageError),
    InstallBlocked(String),
}

/// Drive a bounded batch of futures to completion, preserving order. The batch
/// is already ≤ [`PACKAGE_CHECK_CONCURRENCY`]; this awaits them as a unit so the
/// in-flight count never exceeds the window.
async fn join_bounded<F, T>(futures: Vec<F>) -> Vec<T>
where
    F: std::future::Future<Output = T>,
{
    let mut out = Vec::with_capacity(futures.len());
    // `futures::future::join_all` is not a dependency; a manual await over the
    // pinned set keeps the batch concurrent up to the caller's window without
    // adding a crate. Each future is polled cooperatively by the runtime.
    let mut pinned: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    for fut in &mut pinned {
        out.push(fut.as_mut().await);
    }
    out
}

/// Check a single package: the `.fkst/packages/<name>` dir must exist and the
/// entry file `departments/<name>/main.lua` must resolve to a `file`. At most TWO
/// Contents reads. Applies the engine name rules (by reference) first.
async fn check_one_package(
    reader: &dyn ContentsReader,
    owner_repo: &str,
    name: &str,
) -> PackageCheck {
    // 1. Engine name rules (single source of truth in `fkst-engine`).
    if !is_valid_package_name(name) {
        return PackageCheck::Error(PackageError {
            name: name.to_string(),
            reason: "invalid package name: must fully match [A-Za-z0-9_-]+".to_string(),
        });
    }
    if name == RESERVED_PACKAGE_NAME {
        return PackageCheck::Error(PackageError {
            name: name.to_string(),
            reason: format!("reserved package name not allowed: {RESERVED_PACKAGE_NAME:?}"),
        });
    }

    // 2. Directory existence (read #1).
    let dir_path = format!(".fkst/packages/{name}");
    match reader.get_contents(owner_repo, &dir_path).await {
        Ok(_) => {}
        Err(GithubAppError::NotFound { .. }) => {
            return PackageCheck::Error(PackageError {
                name: name.to_string(),
                reason: format!("missing .fkst/packages/{name}"),
            });
        }
        Err(err) => return classify_app_error_for_package(name, err),
    }

    // 3. Entry-file existence + kind (read #2). The path shape mirrors the
    //    engine's `is_department_main` rule (asserted below as a sanity check).
    let entry_rel = format!("departments/{name}/main.lua");
    debug_assert!(
        is_department_main(&entry_rel),
        "entry path must satisfy the engine's is_department_main rule"
    );
    let entry_path = format!(".fkst/packages/{name}/{entry_rel}");
    match reader.get_contents(owner_repo, &entry_path).await {
        Ok(listing) if listing.is_single_file() => PackageCheck::Ok,
        Ok(_) => PackageCheck::Error(PackageError {
            name: name.to_string(),
            reason: format!("missing entry file departments/{name}/main.lua"),
        }),
        Err(GithubAppError::NotFound { .. }) => PackageCheck::Error(PackageError {
            name: name.to_string(),
            reason: format!("missing entry file departments/{name}/main.lua"),
        }),
        Err(err) => classify_app_error_for_package(name, err),
    }
}

/// Map a non-`NotFound` App error from a Contents read into a package outcome.
/// The install-lifecycle errors ([`GithubAppError::NotInstalled`] /
/// [`GithubAppError::InstallationGone`]) become a repo-wide
/// [`PackageCheck::InstallBlocked`] whose reason reuses the typed install-hint
/// URL (never fabricated); everything else is a per-package error whose reason is
/// client-safe (no token/body detail ever surfaces here).
fn classify_app_error_for_package(name: &str, err: GithubAppError) -> PackageCheck {
    match err {
        GithubAppError::NotInstalled { install_url, .. } => {
            PackageCheck::InstallBlocked(install_blocked_reason(install_url))
        }
        // A vanished installation is the same actionable state from the user's
        // side: the App must be (re)installed before contents can be read.
        GithubAppError::InstallationGone { .. } => {
            PackageCheck::InstallBlocked(install_blocked_reason(None))
        }
        other => PackageCheck::Error(PackageError {
            name: name.to_string(),
            // The error's Display is already secret-redacting (see `github_app`);
            // we still surface only a fixed, generic reason to avoid leaking any
            // upstream text.
            reason: format!(
                "could not read package contents ({})",
                app_error_kind(&other)
            ),
        }),
    }
}

/// Compose the install-hint reason from the typed install URL the App layer
/// surfaced. Mirrors the trigger handler's `NotInstalled` rendering: a present
/// URL is appended; an absent one falls back to the admin-install hint.
fn install_blocked_reason(install_url: Option<String>) -> String {
    match install_url {
        Some(url) => format!("the fkst-hosted GitHub App is not installed on this repo ({url})"),
        None => "the fkst-hosted GitHub App is not installed on this repo \
                 (ask an admin to install it)"
            .to_string(),
    }
}

/// A short, fixed, secret-free label for a Contents App error (never the detail).
fn app_error_kind(err: &GithubAppError) -> &'static str {
    match err {
        GithubAppError::RateLimited(_) => "github rate limited",
        GithubAppError::AppAuth => "github app auth failed",
        GithubAppError::InvalidRepoRef => "invalid repo reference",
        GithubAppError::Http(_) => "github http error",
        _ => "github error",
    }
}

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod tests;
