//! Shared read-side projection from a session registration to its GitHub work issues.
//!
//! The reconciler accepts both an explicit `### Work Label` and labels discovered from
//! the registration's effective package set, including packages supplied by a
//! `### Manifest`. Canvas reads must use that same set; otherwise a valid manifest-only
//! session runs work that the dashboard cannot display.

use std::collections::{BTreeSet, HashMap, HashSet};

use secrecy::SecretString;

use crate::error::AppError;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::effective_packages::resolve_effective_packages;
use crate::reconcile::work_labels::resolve_work_label_sets;
use crate::routes::dashboard::{DashboardGithub, IssueWithMeta};

/// Resolve every registration's effective labels, fetch each distinct label's issues
/// once, and return a deduplicated newest-first issue list keyed by session id.
pub(super) async fn work_issues_by_session(
    gh: &DashboardGithub,
    token: &SecretString,
    owner: &str,
    repo: &str,
    regs: &mut [SessionRegistration],
) -> Result<HashMap<String, Vec<IssueWithMeta>>, AppError> {
    let effective = resolve_effective_packages(&gh.client, &gh.api_base, token, regs).await;
    let mut resolved_regs = Vec::with_capacity(regs.len());
    for reg in regs {
        let Some(packages) = effective.by_session.get(&reg.session_id) else {
            if let Some((_, reason)) = effective
                .demotions
                .iter()
                .find(|(issue, _)| *issue == reg.trigger_issue)
            {
                tracing::debug!(
                    session_id = %reg.session_id,
                    trigger_issue = reg.trigger_issue,
                    reason = %reason,
                    "canvas work projection: effective package resolution failed"
                );
            }
            continue;
        };
        reg.effective_packages = packages.clone();
        resolved_regs.push(reg.clone());
    }

    let labels_by_session =
        resolve_work_label_sets(&gh.client, &gh.api_base, token, &resolved_regs).await;

    // Several sessions can share package configuration, and one issue can carry more
    // than one effective label. Fetch each label once, then deduplicate per session.
    let all_labels: BTreeSet<&str> = labels_by_session
        .values()
        .flatten()
        .map(String::as_str)
        .collect();
    let mut issues_by_label: HashMap<String, Vec<IssueWithMeta>> = HashMap::new();
    for label in all_labels {
        let issues = gh.issues_by_label_all(token, owner, repo, label).await?;
        issues_by_label.insert(label.to_string(), issues);
    }

    let mut projected = HashMap::new();
    for reg in resolved_regs {
        let mut seen = HashSet::new();
        let mut issues = Vec::new();
        if let Some(labels) = labels_by_session.get(&reg.session_id) {
            for label in labels {
                if let Some(label_issues) = issues_by_label.get(label) {
                    for issue in label_issues {
                        if seen.insert(issue.summary.number) {
                            issues.push(issue.clone());
                        }
                    }
                }
            }
        }
        // Each GitHub label query is newest-first. Restore that contract after merging
        // several independently ordered result sets.
        issues.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        projected.insert(reg.session_id, issues);
    }
    Ok(projected)
}
