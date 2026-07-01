//! Package-ref reachability pre-flight (issue #359, "packages are FETCHED").
//!
//! Model B FETCHES a session's packages from PUBLIC GitHub at runtime
//! (`owner/repo@ref:path`) rather than from repo-local `.fkst/packages/`. Before
//! the reconciler spawns a session pod it verifies every referenced package is
//! actually reachable — a typo'd owner/repo, a non-existent ref, a wrong path, or
//! a private repo would otherwise fail deep inside the pod's clone step with no
//! feedback to the issue author. This pre-flight surfaces the problem UP FRONT so
//! the planner can flag the trigger issue instead of spawning a doomed pod.
//!
//! It probes each ref UNAUTHENTICATED (public repos only): a `GET
//! /repos/{owner}/{repo}/contents/{path}/fkst.toml?ref={git_ref}`. A `200` proves
//! the repo, the ref, the path, AND that a package manifest lives there; a `404`
//! means any of those is wrong (or the repo is private); any other status is
//! reported verbatim so a transient GitHub error is distinguishable from a genuine
//! miss. ALL refs are probed and ALL failures collected, so the author sees every
//! bad line at once rather than fixing them one redelivery at a time.
//!
//! Secret hygiene: package refs are non-secret public metadata; the probe carries
//! no credential.

use crate::goals::trigger_parse::PackageRef;

/// The manifest file a valid package directory must contain. Its presence at
/// `{path}/fkst.toml` is what the `200` proves.
const PACKAGE_MANIFEST: &str = "fkst.toml";

/// Human-readable `owner/repo@ref:path` rendering of a ref, used in every failure
/// tuple + the flag comment so the author can match it to their `### Packages` line.
pub fn render_ref(r: &PackageRef) -> String {
    format!("{}/{}@{}:{}", r.owner, r.repo, r.git_ref, r.path)
}

/// Probe every package ref for reachability. `Ok(())` when all are reachable;
/// otherwise `Err` carrying one `(ref_display, reason)` per UNREACHABLE ref (all
/// failures collected, not just the first).
///
/// `github_api_base` is the REST base (e.g. `https://api.github.com`, trailing `/`
/// trimmed by the caller). The probe is unauthenticated: public repos only.
pub async fn check_reachable(
    refs: &[PackageRef],
    http: &reqwest::Client,
    github_api_base: &str,
) -> Result<(), Vec<(String, String)>> {
    let base = github_api_base.trim_end_matches('/');
    let mut failures: Vec<(String, String)> = Vec::new();

    for r in refs {
        let display = render_ref(r);
        match probe_one(r, http, base).await {
            Ok(()) => {}
            Err(reason) => failures.push((display, reason)),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Probe a single ref: `Ok(())` on a `200`, else `Err(reason)`. A `404` is the
/// canonical "missing repo / ref / path (or private)"; any other status (or a
/// transport error) is reported so a transient failure is not mistaken for a miss.
async fn probe_one(r: &PackageRef, http: &reqwest::Client, base: &str) -> Result<(), String> {
    let url = format!(
        "{base}/repos/{}/{}/contents/{}/{PACKAGE_MANIFEST}",
        r.owner, r.repo, r.path
    );
    let response = http
        .get(&url)
        .query(&[("ref", r.git_ref.as_str())])
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        // GitHub rejects an API request with no User-Agent (403); set it here so
        // the probe works regardless of how the shared client was built.
        .header(reqwest::header::USER_AGENT, "fkst-hosted-api")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Err(format!(
            "not reachable ({PACKAGE_MANIFEST} not found at ref {:?} path {:?}); check the \
             owner/repo, the ref, and the path — and that the repo is PUBLIC",
            r.git_ref, r.path
        ))
    } else {
        Err(format!(
            "unexpected status {status} probing {PACKAGE_MANIFEST}"
        ))
    }
}

#[cfg(test)]
#[path = "reachability_tests.rs"]
mod tests;
