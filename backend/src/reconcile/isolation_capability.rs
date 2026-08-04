//! Session-isolation capability pre-flight (issue #5770).
//!
//! "Only act on entities assigned to my session's creator" is implemented in the
//! **`devloop` library** the pod FETCHES at runtime, not in this control plane —
//! `is_routed_to_session` / `issue_owned_by_session` / `claim_admission_precheck`
//! in `libraries/devloop/claims.lua`, called from `observe_issue`,
//! `liveness_scan`, `github_poll` and the admission path.
//!
//! So the rule only binds a session whose package refs resolve to a tree that
//! CARRIES it. A trigger pointing at an older tree runs with no rule at all, and
//! no change to this repository's own package tree can prevent that — the
//! offending pod never loads it. In prod that let one session redrive, terminally
//! drop, and even implement-and-merge another creator's issues onto its own
//! branch.
//!
//! Sharing a work label across creators is INTENDED (sole-assignee routing is what
//! separates them), so the fix is not to forbid the sharing but to refuse to start
//! a session that cannot honour it.
//!
//! This probes the real file rather than trusting a declaration: a self-declared
//! capability in `fkst.toml` is a promise, and a source allowlist keys on
//! provenance rather than capability (rejecting a legitimate fork that does carry
//! the rule). Presence of the symbol is evidence.
//!
//! Probed once per DISTINCT `(owner, repo, ref)` — libraries are repo-level, so N
//! packages from one tree cost one request. Authenticated with the same
//! installation token the reachability probe uses, for the same 5000/hour budget
//! reason. Secret hygiene matches `reachability`: refs are non-secret public
//! metadata and the token travels only as an `Authorization` header.

use std::collections::BTreeSet;

use crate::goals::trigger_parse::PackageRef;

/// The library file that carries the isolation rule, relative to a package tree root.
const CLAIMS_PATH: &str = "libraries/devloop/claims.lua";

/// The symbol whose presence proves the tree carries the rule. This is the gate the
/// ungated entry points (`observe_issue`, `liveness_scan`) call; a tree predating it
/// has the label-family check alone, which cannot tell two sessions apart.
const ISOLATION_SYMBOL: &str = "issue_owned_by_session";

/// `owner/repo@ref` rendering of a probed tree. Deliberately without `:path` — the
/// finding is about the TREE, not one package inside it.
fn render_tree(owner: &str, repo: &str, git_ref: &str) -> String {
    format!("{owner}/{repo}@{git_ref}")
}

/// Verify every distinct package tree carries the session-isolation rule.
///
/// `Ok(())` when all do; otherwise `Err` carrying one `(tree_display, reason)` per
/// offending tree — ALL failures collected, so an author sees every bad ref at once.
///
/// A tree with NO `libraries/devloop/claims.lua` passes: it ships no `devloop`
/// library and therefore contributes no ungated entry point. Only a tree that ships
/// the library WITHOUT the rule is refused.
pub async fn check_isolation_capability(
    refs: &[PackageRef],
    http: &reqwest::Client,
    github_api_base: &str,
    token: Option<&str>,
) -> Result<(), Vec<(String, String)>> {
    let base = github_api_base.trim_end_matches('/');

    // Distinct trees only. BTreeSet also makes the probe order — and therefore the
    // collected failure order — deterministic regardless of how refs were assembled.
    let trees: BTreeSet<(&str, &str, &str)> = refs
        .iter()
        .map(|r| (r.owner.as_str(), r.repo.as_str(), r.git_ref.as_str()))
        .collect();

    let mut failures: Vec<(String, String)> = Vec::new();
    for (owner, repo, git_ref) in trees {
        if let Err(reason) = probe_tree(owner, repo, git_ref, http, base, token).await {
            failures.push((render_tree(owner, repo, git_ref), reason));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Probe one tree. `Ok(())` when the rule is present, or when the tree ships no
/// `devloop` library at all.
async fn probe_tree(
    owner: &str,
    repo: &str,
    git_ref: &str,
    http: &reqwest::Client,
    base: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let url = format!("{base}/repos/{owner}/{repo}/contents/{CLAIMS_PATH}");
    let mut request = http
        .get(&url)
        .query(&[("ref", git_ref)])
        // `raw` gives the file body directly, so the check reads source rather than
        // decoding the base64 envelope the default JSON media type returns.
        .header(reqwest::header::ACCEPT, "application/vnd.github.raw")
        .header(reqwest::header::USER_AGENT, "fkst-hosted-api");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = response.status();
    // No devloop library in this tree ⇒ no ungated entry point from it.
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    if !status.is_success() {
        // Verbatim, like `reachability`: a transient GitHub error must stay
        // distinguishable from a genuine miss rather than silently passing.
        return Err(format!("unexpected status {status} probing {CLAIMS_PATH}"));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("reading {CLAIMS_PATH} failed: {e}"))?;
    if body.contains(ISOLATION_SYMBOL) {
        return Ok(());
    }
    Err(format!(
        "package tree ships {CLAIMS_PATH} WITHOUT the session-isolation rule \
         ({ISOLATION_SYMBOL}); a session running this tree would act on issues \
         assigned to other sessions' creators. Point this ref at a tree that \
         carries the rule"
    ))
}

#[cfg(test)]
#[path = "isolation_capability_tests.rs"]
mod tests;
