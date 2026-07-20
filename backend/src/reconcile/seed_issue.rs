//! Install-time trigger-issue seeder (issue #467; manifest-driven default, epic
//! #594 I9).
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
//! Idempotency: it creates NOTHING if the repo already has an open issue with the
//! trigger label, so a webhook redelivery — or a repo that already has a trigger —
//! never gets a duplicate. Every step is best-effort: a failure is logged and the
//! webhook is unaffected (it always answered 2xx).

use crate::github_app::GithubAppTokens;

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

/// Render the seed trigger-issue body. `log_access_owner` is the repo owner login
/// placed in `### FKST Contributors` (auto-merge is on). The leading marker comment
/// is intro text the parser ignores (parsing starts at the first `### ` heading) and
/// marks the issue as auto-seeded for humans.
///
/// When `default_manifest` is `Some`, the body is the manifest-driven shape (I9): a
/// `### Manifest` reference and NO `### Packages`/`### Work Label`, so the manifest
/// supplies the packages and the wake labels auto-discover. When `None`, the body is
/// the legacy shape: the configured `### Packages` refs (one per line — the parser
/// accepts multiple) plus the explicit `### Work Label`.
fn build_seed_body(
    packages: &[String],
    default_manifest: Option<&str>,
    log_access_owner: &str,
) -> String {
    match default_manifest {
        Some(manifest_ref) => format!(
            "<!-- fkst auto-seeded trigger (v1) -->\n\n\
             ### Session Name\n{SEED_MANIFEST_SESSION_NAME}\n\n\
             ### Manifest\n{manifest_ref}\n\n\
             ### Auto-merge\ntrue\n\n\
             ### FKST Contributors\n{log_access_owner}\n"
        ),
        None => {
            let package_lines = packages.join("\n");
            format!(
                "<!-- fkst auto-seeded trigger (v1) -->\n\n\
                 ### Session Name\n{SEED_SESSION_NAME}\n\n\
                 ### Packages\n{package_lines}\n\n\
                 ### Work Label\n{SEED_WORK_LABEL}\n\n\
                 ### Auto-merge\ntrue\n\n\
                 ### FKST Contributors\n{log_access_owner}\n"
            )
        }
    }
}

/// Best-effort: seed a trigger issue on each of `repos` (`owner/name`) that does
/// not already have an open `trigger_label` issue. `owner_login` is the
/// installation account the repos belong to (it fills the log-access allowlist).
/// `default_manifest` selects the body shape: `Some(ref)` renders the manifest-driven
/// default (I9); `None` renders the legacy `packages` + work-label body.
///
/// Never returns an error and never panics — every per-repo failure is logged and
/// the rest continue. Assumes the caller has already checked the feature flag.
pub async fn seed_trigger_issues(
    github: &GithubAppTokens,
    trigger_label: &str,
    packages: &[String],
    default_manifest: Option<&str>,
    owner_login: &str,
    repos: &[String],
) {
    let labels = [trigger_label.to_string()];
    let body = build_seed_body(packages, default_manifest, owner_login);
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
        match github.create_issue(owner_repo, title, &body, &labels).await {
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
