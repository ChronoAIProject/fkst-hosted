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

use std::collections::{BTreeMap, BTreeSet, HashMap};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::goals::trigger_parse::PackageRef;
use crate::reconcile::desired::SessionRegistration;

/// GitHub's maximum label-name length, measured in Unicode scalar values.
pub const GITHUB_LABEL_NAME_MAX_CHARS: usize = 50;

/// The package-side contract carrying the logical-to-effective label mapping.
pub const SESSION_WORK_LABEL_MAP_JSON_ENV: &str = "FKST_SESSION_WORK_LABEL_MAP_JSON";
/// The provider namespace injected into a hosted substrate runtime. Package
/// runtimes enforce this value as the family-translation authority and verify
/// that the explicit logical-to-effective map agrees with it.
pub const WORK_LABEL_NAMESPACE_ENV: &str = "FKST_WORK_LABEL_NAMESPACE";

/// Render the GitHub trigger-issue title used by a namespaced hosted provider.
/// The namespace is already validated as a lowercase hyphenated slug at config
/// load; its human-facing form is uppercase with each hyphen rendered as a space.
pub(crate) fn provider_session_issue_title(namespace: &str, session_name: &str) -> String {
    let provider_name = namespace.replace('-', " ").to_ascii_uppercase();
    format!("🔔[{provider_name} SESSION] {session_name}")
}

/// A session's provider-neutral labels and their deployment-effective identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveWorkLabels {
    pub logical: Vec<String>,
    pub effective: Vec<String>,
    pub logical_to_effective: BTreeMap<String, String>,
    namespaced: bool,
}

impl EffectiveWorkLabels {
    /// Deterministic JSON for the package runtime. An unnamespaced deployment omits
    /// the variable entirely so its historical environment surface stays unchanged.
    pub fn map_json(&self) -> Option<String> {
        self.namespaced.then(|| {
            serde_json::to_string(&self.logical_to_effective)
                .expect("a string-to-string work-label map always serializes")
        })
    }
}

/// A configuration or package label that cannot be represented safely on GitHub.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct WorkLabelError(String);

impl WorkLabelError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Validate the provider namespace configured by `FKST_WORK_LABEL_NAMESPACE`.
///
/// The 48-character bound is derived from GitHub's 50-character label limit: it
/// leaves room for the shortest valid logical label plus the joining hyphen.
pub fn validate_work_label_namespace(namespace: &str) -> Result<(), WorkLabelError> {
    let len = namespace.chars().count();
    let valid_shape = !namespace.is_empty()
        && namespace.is_ascii()
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !namespace.starts_with('-')
        && !namespace.ends_with('-')
        && !namespace.contains("--");
    if !valid_shape {
        return Err(WorkLabelError::new(
            "must be a lowercase ASCII slug (letters, digits, and single interior hyphens)",
        ));
    }
    if len > GITHUB_LABEL_NAME_MAX_CHARS - 2 {
        return Err(WorkLabelError::new(format!(
            "must be at most {} characters",
            GITHUB_LABEL_NAME_MAX_CHARS - 2
        )));
    }
    Ok(())
}

fn validate_effective_label(label: &str) -> Result<(), WorkLabelError> {
    if label.is_empty() || label.trim() != label {
        return Err(WorkLabelError::new(format!(
            "effective work label `{label}` must be non-empty with no surrounding whitespace"
        )));
    }
    if label.contains(',') {
        return Err(WorkLabelError::new(format!(
            "effective work label `{label}` cannot contain a comma"
        )));
    }
    if label.chars().any(char::is_control) {
        return Err(WorkLabelError::new(format!(
            "effective work label `{label}` cannot contain control characters"
        )));
    }
    let len = label.chars().count();
    if len > GITHUB_LABEL_NAME_MAX_CHARS {
        return Err(WorkLabelError::new(format!(
            "effective work label `{label}` is {len} characters; GitHub allows at most {GITHUB_LABEL_NAME_MAX_CHARS}"
        )));
    }
    Ok(())
}

/// Apply an optional provider namespace to a complete logical work-label set.
///
/// Exact labels become `<logical>-<namespace>`. The result is sorted and
/// deduplicated, and case-insensitive output collisions fail closed because GitHub
/// label identity is case-insensitive.
pub fn apply_work_label_namespace(
    labels: &[String],
    namespace: Option<&str>,
) -> Result<EffectiveWorkLabels, WorkLabelError> {
    if let Some(namespace) = namespace {
        validate_work_label_namespace(namespace)?;
    }

    let logical: BTreeSet<String> = labels.iter().cloned().collect();
    let mut effective = Vec::with_capacity(logical.len());
    let mut logical_to_effective = BTreeMap::new();
    let mut owners_by_folded_effective: HashMap<String, String> = HashMap::new();

    for logical_label in &logical {
        if logical_label.is_empty() {
            return Err(WorkLabelError::new("logical work labels must be non-empty"));
        }
        let effective_label = match namespace {
            Some(namespace) => format!("{logical_label}-{namespace}"),
            None => logical_label.clone(),
        };
        validate_effective_label(&effective_label)?;

        let folded = effective_label.to_lowercase();
        if let Some(owner) = owners_by_folded_effective.insert(folded, logical_label.clone()) {
            if owner != *logical_label {
                return Err(WorkLabelError::new(format!(
                    "logical work labels `{owner}` and `{logical_label}` collide as effective label `{effective_label}`"
                )));
            }
        }
        logical_to_effective.insert(logical_label.clone(), effective_label.clone());
        effective.push(effective_label);
    }

    Ok(EffectiveWorkLabels {
        logical: logical.into_iter().collect(),
        effective,
        logical_to_effective,
        namespaced: namespace.is_some(),
    })
}

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

/// Resolve every registration's complete work-label set: its explicit
/// `### Work Label` plus every label auto-discovered from its effective package
/// set. Results are keyed by session id and sorted/deduplicated by label.
///
/// Registrations must already carry their manifest-expanded
/// [`SessionRegistration::effective_packages`]. Discovery is cached per config
/// hash for the duration of this call so sessions with identical immutable
/// package configuration do not repeat the same GitHub contents reads.
pub async fn resolve_work_label_sets(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    regs: &[SessionRegistration],
) -> HashMap<String, Vec<String>> {
    let mut discovered_cache: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for reg in regs {
        let discovered = match discovered_cache.get(&reg.config_hash) {
            Some(set) => set.clone(),
            None => {
                let set = resolve_work_labels(http, api_base, token, &reg.effective_packages).await;
                discovered_cache.insert(reg.config_hash.clone(), set.clone());
                set
            }
        };
        let mut labels = discovered;
        if let Some(work_label) = &reg.def.work_label {
            labels.insert(work_label.clone());
        }
        out.insert(reg.session_id.clone(), labels.into_iter().collect());
    }
    out
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
