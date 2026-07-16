//! Install-time trigger-issue seeder (issue #467).
//!
//! When the App is installed on a repo (or a repo is added to an installation)
//! AND `FKST_SEED_TRIGGER_ISSUE_ON_INSTALL` is on, this best-effort seeder opens
//! ONE trigger issue that onboards the repo to the GitHub devloop — its body is
//! the fixed [`SEED_*`] specification, parseable by
//! [`crate::goals::trigger_parse::parse_trigger_issue_body`] (a round-trip test
//! pins it), carrying the reconciler's trigger label.
//!
//! Idempotency: it creates NOTHING if the repo already has an open issue with the
//! trigger label, so a webhook redelivery — or a repo that already has a trigger —
//! never gets a duplicate. Every step is best-effort: a failure is logged and the
//! webhook is unaffected (it always answered 2xx).

use crate::github_app::GithubAppTokens;

/// The seed session's name (the `### Session Name` section). A DNS-1123-ish
/// env-name the parser accepts.
const SEED_SESSION_NAME: &str = "evolve";

/// The seed session's work label (`### Work Label`).
const SEED_WORK_LABEL: &str = "fkst-evolve";

/// The issue title.
const SEED_TITLE: &str = "[session] evolve (auto-seeded)";

/// Render the seed trigger-issue body. `packages` are the configured
/// `### Packages` refs (one per line — the parser accepts multiple), and
/// `log_access_owner` is the repo owner login placed in `### FKST Contributors`
/// (auto-merge is on). The leading marker comment is intro text the parser
/// ignores (it starts at the first `### ` heading) and marks the issue as
/// auto-seeded for humans.
fn build_seed_body(packages: &[String], log_access_owner: &str) -> String {
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

/// Best-effort: seed a trigger issue on each of `repos` (`owner/name`) that does
/// not already have an open `trigger_label` issue. `owner_login` is the
/// installation account the repos belong to (it fills the log-access allowlist).
///
/// Never returns an error and never panics — every per-repo failure is logged and
/// the rest continue. Assumes the caller has already checked the feature flag.
pub async fn seed_trigger_issues(
    github: &GithubAppTokens,
    trigger_label: &str,
    packages: &[String],
    owner_login: &str,
    repos: &[String],
) {
    let labels = [trigger_label.to_string()];
    let body = build_seed_body(packages, owner_login);
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
            .create_issue(owner_repo, SEED_TITLE, &body, &labels)
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
