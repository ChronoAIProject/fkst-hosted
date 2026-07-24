//! Install-time trigger-issue seeder (issue #467; manifest-driven default, epic
//! #594 I9; onboarding intro, issue #3379).
//!
//! When the App is installed on a repo (or a repo is added to an installation)
//! AND `FKST_SEED_TRIGGER_ISSUE_ON_INSTALL` is on (now the default), this
//! best-effort seeder opens ONE trigger issue that onboards the repo to the fkst
//! devloop. Its body is one of two fixed shapes, both parseable by
//! [`crate::goals::trigger_parse::parse_trigger_issue_body`] (round-trip tests pin
//! both), carrying the reconciler's trigger label:
//!
//! - **Manifest-driven (default, I9):** when a `default_manifest` ref is configured
//!   (`FKST_DEFAULT_MANIFEST`, defaulting to the composed default-workflows
//!   manifest), the body carries a `### Manifest` reference and NEITHER `### Packages`
//!   NOR `### Work Label`. The manifest supplies the package set, and the session's
//!   wake labels auto-discover from those packages' `[github].work_labels` — so a
//!   single seeded trigger runs every workflow the manifest bundles.
//! - **Legacy (no manifest configured):** when no default manifest is set (a blank
//!   `FKST_DEFAULT_MANIFEST` override), the body falls back to the original explicit
//!   `### Packages` + `### Work Label` shape.
//!
//! Both shapes open with a VISIBLE onboarding intro (issue #3379) that the parser
//! ignores (parsing starts at the first `### ` heading): it explains what keeping
//! the issue open does, states the session runs in the DEFAULT environment (no
//! named environment profile, no extra configuration or software installations),
//! summarizes the defaults in effect, and links the fkst dashboard when a
//! frontend URL is configured. No intro line may begin with `### ` — that would
//! open a parsed section — so the trigger-section names it mentions are kept
//! mid-line in backticks.
//!
//! Every seed is attributed to the installation event's human sender: the issue is
//! created with that login as its sole assignee and the login is listed first in
//! `### FKST Contributors`. The webhook skips seeding entirely when no sender is
//! available, so it never creates a bot-authored trigger the creator gate cannot
//! attribute.
//!
//! Idempotency: it creates NOTHING if the repo already has an open issue with the
//! trigger label, so a webhook redelivery — or a repo that already has a trigger —
//! never gets a duplicate. Every step is best-effort: a failure is logged and the
//! webhook is unaffected (it always answered 2xx).

use crate::github_app::GithubAppTokens;
use crate::reconcile::branches::DEFAULT_TARGET_BRANCH;

/// The manifest-driven seed session's name (`### Session Name`, I9). A DNS-1123-ish
/// env-name the parser accepts; names the default-workflows bundle for humans.
const SEED_MANIFEST_SESSION_NAME: &str = "default-workflows";

/// The manifest-driven seed issue title (I9).
const SEED_MANIFEST_TITLE: &str = "[session] default-workflows (auto-seeded)";

/// The legacy seed session's name (the `### Session Name` section). A DNS-1123-ish
/// env-name the parser accepts.
const SEED_SESSION_NAME: &str = "evolve";

/// The legacy seed session's work label (`### Work Label`).
const SEED_WORK_LABEL: &str = "fkst-evolve";

/// The legacy issue title.
const SEED_TITLE: &str = "[session] evolve (auto-seeded)";

/// Render the parser-ignored onboarding intro both seed shapes open with
/// (issue #3379). `session_name` names the session the trigger registers;
/// `labels_line` is the per-shape sentence describing how the session's work
/// labels come to be; `frontend_url` (the configured `FKST_FRONTEND_URL`)
/// renders the dashboard pointer when present.
///
/// Invariant: no rendered line begins with `### ` (that would open a parsed
/// section) — the section names mentioned here stay mid-line in backticks.
fn seed_intro(session_name: &str, labels_line: &str, frontend_url: Option<&str>) -> String {
    let dashboard = match frontend_url {
        Some(url) => {
            format!(" Saved profiles are created and managed in the fkst dashboard: {url}")
        }
        None => " Saved profiles are created and managed in the fkst dashboard".to_string(),
    };
    format!(
        "👋 **Welcome to fkst!** This trigger issue was created automatically when the \
         fkst GitHub App was installed on this repository. Keeping it **open** registers \
         the `{session_name}` session for this repo — once the session registers, fkst \
         replies below with its work labels and step-by-step instructions for queueing \
         work. Closing this issue retires the session.\n\n\
         **Environment profile: default.** This session runs in the default session \
         environment — no named environment profile is selected, and no extra \
         configuration or software installations are applied. To run with a customized \
         environment, close this issue and open a new trigger whose `### Environment` \
         section names one of your saved environment profiles.{dashboard}.\n\n\
         **Defaults in effect:** {labels_line} Each work issue is worked as an \
         independent pull request into the `{DEFAULT_TARGET_BRANCH}` target branch \
         (auto-created from the repository default branch when absent), and auto-merge \
         is on.\n\n"
    )
}

/// Render the seed trigger-issue body. `installer` is listed first in `### FKST
/// Contributors`, followed by `owner_login` unless both identify the same account
/// case-insensitively (auto-merge is on). The leading marker comment plus the
/// visible onboarding intro are intro text the parser ignores (parsing starts at
/// the first `### ` heading); the marker tags the issue as auto-seeded for tools.
///
/// When `default_manifest` is `Some`, the body is the manifest-driven shape (I9): a
/// `### Manifest` reference and NO `### Packages`/`### Work Label`, so the manifest
/// supplies the packages and the wake labels auto-discover. When `None`, the body is
/// the legacy shape: the configured `### Packages` refs (one per line — the parser
/// accepts multiple) plus the explicit `### Work Label`.
fn build_seed_body(
    packages: &[String],
    default_manifest: Option<&str>,
    installer: &str,
    owner_login: &str,
    frontend_url: Option<&str>,
) -> String {
    let contributors = seed_contributors(installer, owner_login);
    match default_manifest {
        Some(manifest_ref) => {
            let intro = seed_intro(
                SEED_MANIFEST_SESSION_NAME,
                "the session's work labels are auto-discovered from the manifest's packages \
                 (listed in the registration reply below).",
                frontend_url,
            );
            format!(
                "<!-- fkst auto-seeded trigger (v2) -->\n\n\
                 {intro}\
                 ### Session Name\n{SEED_MANIFEST_SESSION_NAME}\n\n\
                 ### Manifest\n{manifest_ref}\n\n\
                 ### Auto-merge\ntrue\n\n\
                 ### FKST Contributors\n{contributors}\n"
            )
        }
        None => {
            let intro = seed_intro(
                SEED_SESSION_NAME,
                &format!("the session works issues labeled `{SEED_WORK_LABEL}`."),
                frontend_url,
            );
            let package_lines = packages.join("\n");
            format!(
                "<!-- fkst auto-seeded trigger (v2) -->\n\n\
                 {intro}\
                 ### Session Name\n{SEED_SESSION_NAME}\n\n\
                 ### Packages\n{package_lines}\n\n\
                 ### Work Label\n{SEED_WORK_LABEL}\n\n\
                 ### Auto-merge\ntrue\n\n\
                 ### FKST Contributors\n{contributors}\n"
            )
        }
    }
}

fn seed_contributors(installer: &str, owner_login: &str) -> String {
    if installer.eq_ignore_ascii_case(owner_login) {
        installer.to_string()
    } else {
        format!("{installer}\n{owner_login}")
    }
}

/// Best-effort: seed a trigger issue on each of `repos` (`owner/name`) that does
/// not already have an open `trigger_label` issue. `owner_login` is the
/// installation account the repos belong to; `installer` is the human sender who
/// becomes the issue's sole assignee and first log-access principal.
/// `default_manifest` selects the body shape: `Some(ref)` renders the manifest-driven
/// default (I9); `None` renders the legacy `packages` + work-label body.
/// `frontend_url` (the configured `FKST_FRONTEND_URL`) renders the intro's
/// dashboard pointer when present.
///
/// Never returns an error and never panics — every per-repo failure is logged and
/// the rest continue. Assumes the caller has already checked the feature flag.
// Each parameter is one independently-configured seeding input threaded from the
// webhook handler; they are not a cohesive struct worth introducing for a single
// call site (mirrors announce_session_comment).
#[allow(clippy::too_many_arguments)]
pub async fn seed_trigger_issues(
    github: &GithubAppTokens,
    trigger_label: &str,
    packages: &[String],
    default_manifest: Option<&str>,
    owner_login: &str,
    installer: &str,
    repos: &[String],
    frontend_url: Option<&str>,
) {
    let labels = [trigger_label.to_string()];
    let assignees = [installer.to_string()];
    let body = build_seed_body(
        packages,
        default_manifest,
        installer,
        owner_login,
        frontend_url,
    );
    // The title names the seed session, which differs between the two body shapes.
    let title = if default_manifest.is_some() {
        SEED_MANIFEST_TITLE
    } else {
        SEED_TITLE
    };
    for owner_repo in repos {
        match github
            .open_issues_with_label(owner_repo, trigger_label)
            .await
        {
            Ok(existing) if !existing.is_empty() => {
                tracing::info!(
                    repo = %owner_repo,
                    existing = existing.len(),
                    "seed: repo already has an open trigger issue; skipping"
                );
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                // Could not confirm idempotency — skip rather than risk a duplicate.
                tracing::warn!(repo = %owner_repo, error = %error, "seed: idempotency probe failed; skipping");
                continue;
            }
        }
        match github
            .create_issue(owner_repo, title, &body, &labels, &assignees)
            .await
        {
            Ok(number) => {
                tracing::info!(repo = %owner_repo, issue = number, "seed: created trigger issue")
            }
            Err(error) => {
                tracing::warn!(repo = %owner_repo, error = %error, "seed: create trigger issue failed")
            }
        }
    }
}

#[cfg(test)]
#[path = "seed_issue_tests.rs"]
mod tests;
