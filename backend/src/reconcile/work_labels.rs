//! Auto-discovery of a session's GitHub work labels from its packages.
//!
//! A github-issue-driven fkst package declares the label(s) it polls in a
//! host-readable `[github] work_labels = [...]` section of its `fkst.toml` (the
//! engine ignores the section; see the `github-issue` library in fkst-packages).
//! This module resolves, for a session's package set, the UNION of those declared
//! labels — walking each package's `[event_deps]` transitively, since a composed
//! package's effective labels include those of the sibling packages it pulls in.
//!
//! The result feeds the reconcile wake-gate ([`crate::reconcile::pending`]): a
//! session's pod is spawned/kept alive when ANY of its discovered labels (plus the
//! trigger's explicit work label) has an open issue — so a package's own label
//! wakes the session without the operator restating it in the trigger issue.
//!
//! Reads are best-effort and tolerant: an unreachable manifest or a package
//! without a `[github]` section simply contributes nothing. Everything is a plain
//! authenticated `contents` fetch + a lenient TOML parse — no engine, no Lua.

use std::collections::BTreeSet;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::goals::trigger_parse::PackageRef;

/// The slice of a package `fkst.toml` this module reads. `#[serde(default)]` +
/// no `deny_unknown_fields`: every other manifest section (`[code]`, `[lib_deps]`,
/// …) is ignored, and a manifest with neither section parses to empty.
#[derive(Debug, Default, Deserialize)]
struct Manifest {
    #[serde(default)]
    github: GithubMeta,
    #[serde(default)]
    event_deps: EventDeps,
}

#[derive(Debug, Default, Deserialize)]
struct GithubMeta {
    #[serde(default)]
    work_labels: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct EventDeps {
    #[serde(default)]
    packages: Vec<String>,
}

/// Resolve the union of `[github].work_labels` declared across `packages` and
/// their transitive `[event_deps]` closure. Never errors: unreachable or
/// unparseable manifests contribute nothing (the wake-gate degrades to the
/// trigger's explicit label). `api_base` is the GitHub API root; `token` is a
/// repo-scoped installation (or user) token.
pub async fn resolve_work_labels(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    packages: &[PackageRef],
) -> BTreeSet<String> {
    let base = api_base.trim_end_matches('/');
    let mut labels: BTreeSet<String> = BTreeSet::new();
    // Visited/worklist keyed by (owner, repo, git_ref, path) so a diamond in the
    // event-dep graph is fetched once and a cycle terminates.
    let mut visited: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    let mut worklist: Vec<(String, String, String, String)> = packages
        .iter()
        .map(|p| {
            (
                p.owner.clone(),
                p.repo.clone(),
                p.git_ref.clone(),
                p.path.clone(),
            )
        })
        .collect();

    while let Some(node) = worklist.pop() {
        if !visited.insert(node.clone()) {
            continue;
        }
        let (owner, repo, git_ref, path) = &node;
        let Some(manifest) = fetch_manifest(http, base, token, owner, repo, git_ref, path).await
        else {
            continue;
        };
        for label in manifest.github.work_labels {
            let trimmed = label.trim();
            if !trimmed.is_empty() {
                labels.insert(trimmed.to_string());
            }
        }
        // Event-dep siblings live under `packages/<name>` on the SAME repo+ref
        // (workspace convention: package name == directory basename).
        for name in manifest.event_deps.packages {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            worklist.push((
                owner.clone(),
                repo.clone(),
                git_ref.clone(),
                format!("packages/{name}"),
            ));
        }
    }
    labels
}

/// Fetch + parse one package's `fkst.toml` via the GitHub raw contents API.
/// `None` on any failure (transport, non-2xx, or unparseable TOML).
async fn fetch_manifest(
    http: &reqwest::Client,
    base: &str,
    token: &SecretString,
    owner: &str,
    repo: &str,
    git_ref: &str,
    path: &str,
) -> Option<Manifest> {
    let url = format!("{base}/repos/{owner}/{repo}/contents/{path}/fkst.toml");
    let response = http
        .get(&url)
        .query(&[("ref", git_ref)])
        // `raw` returns the file bytes directly rather than the base64 envelope.
        .header(reqwest::header::ACCEPT, "application/vnd.github.raw")
        .header(reqwest::header::USER_AGENT, "fkst-hosted-api")
        .bearer_auth(token.expose_secret())
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().await.ok()?;
    toml::from_str::<Manifest>(&body).ok()
}

#[cfg(test)]
#[path = "work_labels_tests.rs"]
mod tests;
