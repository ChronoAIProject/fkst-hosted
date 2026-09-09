//! Repository label bootstrap for the platform-owned label vocabulary.
//!
//! The control plane applies `fkst-cron-*` labels with `add_issue_labels`, and
//! GitHub will materialize a missing label implicitly — with a random colour and
//! no description. For labels a HUMAN reads and toggles by hand
//! (`fkst-cron-paused` above all) that is a poor experience: the one label users
//! are meant to touch would arrive looking like an accident.
//!
//! Two rules make this safe to run on every repository:
//!
//! - **Create, never update.** A 422 "already exists" is SUCCESS. Colour and
//!   description are cosmetic, and PATCHing over a maintainer's customisation on
//!   every sweep would be both rude and noisy in the repository's audit log.
//! - **Best-effort throughout.** A failure is logged and skipped. Labels are a
//!   presentation concern; failing a reconcile over one would trade something that
//!   matters for something that does not.

use secrecy::ExposeSecret;

use super::{GithubAppError, GithubAppTokens};

/// The platform-owned labels, with the meaning each one shows in GitHub's label
/// list. One colour family so they read as a set.
///
/// `fkst-cron-paused` is described in the second person because it is the only
/// one a human is meant to apply — every other label here is control-plane owned
/// and is written by the reconciler alone.
const PLATFORM_LABELS: &[(&str, &str, &str)] = &[
    (
        crate::reconcile::reserved_labels::SCHEDULED_WORKFLOW_LABEL,
        "1D76DB",
        "A scheduled workflow definition: which workflow runs, with which arguments, how often.",
    ),
    (
        crate::reconcile::reserved_labels::CRON_RUNNING_LABEL,
        "0E8A16",
        "A scheduled run is in flight (control-plane owned).",
    ),
    (
        crate::reconcile::reserved_labels::CRON_PAUSED_LABEL,
        "FBCA04",
        "Add this yourself to pause a scheduled workflow without closing it. Remove it to resume.",
    ),
    (
        crate::reconcile::reserved_labels::SCHEDULE_INVALID_LABEL,
        "B60205",
        "The schedule was rejected — see the comment. Clears automatically once fixed.",
    ),
    (
        crate::reconcile::reserved_labels::CRON_FAILED_LABEL,
        "D93F0B",
        "The last scheduled run failed.",
    ),
    (
        crate::reconcile::reserved_labels::CRON_TIMEOUT_LABEL,
        "D93F0B",
        "The last scheduled run exceeded its budget and was released by the watchdog.",
    ),
    // The two run-issue labels are WORK labels, so a deployment that sets a
    // work-label namespace routes on the suffixed form and these bootstrap only the
    // bare names. Listing them is still worth it: an unnamespaced deployment gets
    // the colour and description, and the reserved-name invariant below is what
    // stops either name being adopted by a session.
    (
        crate::reconcile::reserved_labels::WORKFLOW_RUN_LABEL,
        "1D76DB",
        "A one-time workflow run. Worked by the workflow runner, not the dev loop.",
    ),
    (
        crate::reconcile::reserved_labels::WORKFLOW_SCHEDULED_RUN_LABEL,
        "1D76DB",
        "A scheduled workflow run. Worked by the workflow runner, not the dev loop.",
    ),
];

/// Create any missing platform label on `owner_repo`. Never propagates.
pub async fn ensure_platform_labels(github: &GithubAppTokens, owner_repo: &str) {
    for (name, color, description) in PLATFORM_LABELS {
        match ensure_label(github, owner_repo, name, color, description).await {
            Ok(()) => {}
            Err(error) => {
                tracing::debug!(
                    owner_repo = %owner_repo,
                    label = name,
                    error = %error,
                    "label bootstrap skipped"
                );
            }
        }
    }
}

/// `POST /repos/{owner}/{repo}/labels`, treating "already exists" as success.
async fn ensure_label(
    github: &GithubAppTokens,
    owner_repo: &str,
    name: &str,
    color: &str,
    description: &str,
) -> Result<(), GithubAppError> {
    let (owner, repo) = owner_repo
        .split_once('/')
        .ok_or(GithubAppError::InvalidRepoRef)?;
    let token = github.token_for_repo(owner_repo, None).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("fkst-hosted-api")
        .build()
        .map_err(|error| GithubAppError::Http(format!("label client build: {error}")))?;
    let response = client
        .post(format!(
            "{}/repos/{owner}/{repo}/labels",
            github.rest_api_base().trim_end_matches('/')
        ))
        .header("accept", "application/vnd.github+json")
        .bearer_auth(token.expose_secret())
        .json(&serde_json::json!({
            "name": name,
            "color": color,
            "description": description,
        }))
        .send()
        .await
        .map_err(|error| GithubAppError::Http(format!("ensure_label: {error}")))?;

    let status = response.status();
    // 201 created, or 422 because it is already there — both are the desired end
    // state. Anything else is a real failure the caller logs and moves past.
    if status.is_success() || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(GithubAppError::Http(format!(
        "ensure_label status {status}: {body}"
    )))
}

/// The label vocabulary, exposed so a test can assert it covers the reserved set.
pub fn platform_label_names() -> Vec<&'static str> {
    PLATFORM_LABELS.iter().map(|(name, _, _)| *name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::reserved_labels::RESERVED_LABELS;

    #[test]
    fn every_reserved_label_is_bootstrapped() {
        // A reserved label missing here would still work — GitHub materializes it
        // implicitly — but would arrive with a random colour and no description,
        // which is exactly the experience this bootstrap exists to avoid.
        let bootstrapped = platform_label_names();
        for reserved in RESERVED_LABELS {
            assert!(
                bootstrapped.contains(reserved),
                "{reserved} must be bootstrapped with a colour and a description"
            );
        }
    }

    #[test]
    fn the_user_applied_label_is_described_in_the_second_person() {
        // It is the only one a human is meant to touch, so its description has to
        // tell them so from the label list itself.
        let (_, _, description) = PLATFORM_LABELS
            .iter()
            .find(|(name, _, _)| *name == crate::reconcile::reserved_labels::CRON_PAUSED_LABEL)
            .expect("the paused label is bootstrapped");
        assert!(description.contains("yourself"), "{description}");
        assert!(description.contains("resume"), "{description}");
    }

    #[test]
    fn every_colour_is_a_six_digit_hex_github_accepts() {
        for (name, color, _) in PLATFORM_LABELS {
            assert!(
                color.len() == 6 && color.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{name} has an invalid colour {color}"
            );
        }
    }
}
