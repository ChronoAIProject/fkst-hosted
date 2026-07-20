//! Resolve every registration's EFFECTIVE package set for one reconcile pass (epic
//! #594 I7).
//!
//! A trigger issue can name packages two ways: inline `### Packages` lines, and/or
//! `### Manifest` references (each a JSON file that bundles a package list — see
//! [`crate::reconcile::manifest_expand`]). This module fetches + expands the manifests
//! and merges them with the explicit packages into ONE effective set per session, which
//! every downstream consumer (reachability, `FKST_SESSION_PACKAGE_ROOTS`, and
//! work-label auto-discovery) then reads.
//!
//! ## Effective-set rule
//!
//! `effective = explicit ++ (manifest_1 ++ manifest_2 ++ …)`, deduped by the full
//! `(owner, repo, ref, path)` identity keeping the FIRST occurrence. So the order is:
//! the explicit `### Packages` (author order) first, then each `### Manifest`'s expansion
//! in manifest order (and in-file order within each manifest), and a package that appears
//! both explicitly and via a manifest survives ONCE, in its explicit position
//! (explicit-first).
//!
//! ## Fail-closed
//!
//! A manifest is a required, complete package set. If ANY of a session's manifests fails
//! to fetch/parse/validate ([`expand_manifest`] errors), OR the resulting effective union
//! is empty, the session is DEMOTED: its `(trigger_issue, reason)` marker is returned for
//! the driver to fold into the planner's `invalid` input — the SAME flag/comment/
//! auto-clear path as a work-label collision or a missing label. A manifest-free session
//! can never fail here (its effective set is just its explicit packages).
//!
//! Secret hygiene: the demote reason carries only the public `owner/repo@ref:path`
//! rendering + [`crate::reconcile::manifest_expand::ManifestError`]'s leak-free `Display`
//! — never the token, the fetch URL, or transport detail.

use std::collections::{BTreeSet, HashMap};

use secrecy::SecretString;

use crate::goals::trigger_parse::PackageRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::manifest_expand::expand_manifest;
use crate::reconcile::reachability::render_ref;

/// The identity a package/manifest reference is deduped + cached by: its full
/// `(owner, repo, ref, path)` tuple.
type RefKey = (String, String, String, String);

/// The demote reason when a session ends up with NO packages at all — neither an inline
/// `### Packages` line nor a manifest that expanded into any. The post-expansion guard
/// (the trigger parser already requires ≥1 of the two sections, but a manifest could in
/// principle contribute nothing were the invariant to change, so this stays a real check).
pub const NO_PACKAGES_DETAIL: &str =
    "no packages: add a `### Packages` line or a valid `### Manifest` reference";

/// Project a reference into its dedup/cache key.
fn ref_key(r: &PackageRef) -> RefKey {
    (
        r.owner.clone(),
        r.repo.clone(),
        r.git_ref.clone(),
        r.path.clone(),
    )
}

/// The outcome of resolving every registration's effective package set for one pass.
pub struct EffectivePackages {
    /// `session_id -> effective package set` for every registration that resolved
    /// cleanly. A demoted registration has NO entry here (it is in [`Self::demotions`]).
    pub by_session: HashMap<String, Vec<PackageRef>>,
    /// `(trigger_issue, reason)` demotion markers, sorted ascending by trigger issue, for
    /// registrations whose manifest expansion failed or whose effective union came out
    /// empty. The driver folds these into the planner's `invalid` input (fail-closed).
    pub demotions: Vec<(i64, String)>,
}

/// Resolve the effective package set for each registration, fetching + expanding its
/// `### Manifest` references with `token`. `api_base` is the GitHub API root.
///
/// Each DISTINCT manifest reference is fetched at most once per pass (its expansion
/// result — success or failure — is cached by reference), so a manifest shared across
/// sessions, or repeated within a session, never re-fetches.
pub async fn resolve_effective_packages(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    regs: &[SessionRegistration],
) -> EffectivePackages {
    // Cache each distinct manifest reference's expansion (Ok packages / Err reason) so a
    // manifest shared by two sessions is fetched once. The Err reason is the manifest's
    // own leak-free message (without the per-ref prefix, which is re-added per session).
    let mut cache: HashMap<RefKey, Result<Vec<PackageRef>, String>> = HashMap::new();
    let mut by_session: HashMap<String, Vec<PackageRef>> = HashMap::new();
    let mut demotions: Vec<(i64, String)> = Vec::new();

    for reg in regs {
        match expand_one(http, api_base, token, reg, &mut cache).await {
            Ok(effective) => {
                by_session.insert(reg.session_id.clone(), effective);
            }
            Err(reason) => demotions.push((reg.trigger_issue, reason)),
        }
    }
    // Sort so the driver's fold into `invalid` is order-independent (determinism), like
    // the collision + missing-label detectors.
    demotions.sort_by_key(|(issue, _)| *issue);
    EffectivePackages {
        by_session,
        demotions,
    }
}

/// Compute ONE registration's effective set, or `Err(reason)` when fail-closed.
async fn expand_one(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    reg: &SessionRegistration,
    cache: &mut HashMap<RefKey, Result<Vec<PackageRef>, String>>,
) -> Result<Vec<PackageRef>, String> {
    // Explicit packages first (author order); then each manifest's expansion appended in
    // manifest order (in-file order preserved within each expansion).
    let mut effective: Vec<PackageRef> = reg.def.packages.clone();
    for manifest_ref in &reg.def.manifest_refs {
        let key = ref_key(manifest_ref);
        let resolved = match cache.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let result = expand_manifest(http, api_base, token, manifest_ref)
                    .await
                    .map_err(|err| err.to_string());
                cache.insert(key, result.clone());
                result
            }
        };
        match resolved {
            Ok(packages) => effective.extend(packages),
            // Fail-closed: a manifest a session names but that cannot be expanded demotes
            // the whole session. The reason names the offending manifest + its cause.
            Err(reason) => {
                let detail = format!(
                    "manifest {} could not be expanded: {reason}",
                    render_ref(manifest_ref)
                );
                tracing::info!(
                    session_id = %reg.session_id,
                    trigger_issue = reg.trigger_issue,
                    detail = %detail,
                    "reconcile: manifest expansion failed; demoting session to invalid"
                );
                return Err(detail);
            }
        }
    }

    // Dedup by full reference identity, keeping the FIRST occurrence so an explicit
    // package always wins over a manifest-supplied duplicate (explicit-first).
    let mut seen: BTreeSet<RefKey> = BTreeSet::new();
    let mut deduped: Vec<PackageRef> = Vec::with_capacity(effective.len());
    for package in effective {
        if seen.insert(ref_key(&package)) {
            deduped.push(package);
        }
    }

    if deduped.is_empty() {
        return Err(NO_PACKAGES_DETAIL.to_string());
    }
    tracing::debug!(
        session_id = %reg.session_id,
        explicit = reg.def.packages.len(),
        manifests = reg.def.manifest_refs.len(),
        effective = deduped.len(),
        "reconcile: resolved effective package set"
    );
    Ok(deduped)
}

#[cfg(test)]
#[path = "effective_packages_tests.rs"]
mod tests;
