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

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    /// `session_id -> effective per-package configuration` for every registration
    /// that resolved cleanly. Manifest-supplied defaults merged with the trigger's
    /// own `### Package Env`, the trigger winning PER KEY (see [`merge_package_env`]).
    pub package_env_by_session: HashMap<String, crate::goals::package_env::PackageEnv>,
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
    mandatory: &[PackageRef],
) -> EffectivePackages {
    // Cache each distinct manifest reference's expansion (Ok packages / Err reason) so a
    // manifest shared by two sessions is fetched once. The Err reason is the manifest's
    // own leak-free message (without the per-ref prefix, which is re-added per session).
    let mut cache: HashMap<
        RefKey,
        Result<crate::reconcile::manifest_expand::ExpandedManifest, String>,
    > = HashMap::new();
    let mut by_session: HashMap<String, Vec<PackageRef>> = HashMap::new();
    let mut package_env_by_session: HashMap<String, crate::goals::package_env::PackageEnv> =
        HashMap::new();
    let mut demotions: Vec<(i64, String)> = Vec::new();

    for reg in regs {
        match expand_one(http, api_base, token, reg, &mut cache, mandatory).await {
            Ok((effective, package_env)) => {
                by_session.insert(reg.session_id.clone(), effective);
                package_env_by_session.insert(reg.session_id.clone(), package_env);
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
        package_env_by_session,
    }
}

/// Compute ONE registration's effective set, or `Err(reason)` when fail-closed.
async fn expand_one(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    reg: &SessionRegistration,
    cache: &mut HashMap<
        RefKey,
        Result<crate::reconcile::manifest_expand::ExpandedManifest, String>,
    >,
    mandatory: &[PackageRef],
) -> Result<(Vec<PackageRef>, crate::goals::package_env::PackageEnv), String> {
    // Mandatory packages FIRST, then explicit packages (author order), then each
    // manifest's expansion in manifest order (in-file order preserved within each).
    //
    // Prepended rather than appended so the dedup below -- first occurrence wins by
    // full (owner, repo, ref, path) -- resolves a session that ALSO declares one of
    // them to a single copy in the mandatory position. Order matters downstream:
    // FKST_SESSION_PACKAGE_ROOTS is built from this list.
    let mut effective: Vec<PackageRef> = mandatory.to_vec();
    effective.extend(reg.def.packages.iter().cloned());
    // Manifest-supplied configuration, folded in manifest order. The FIRST manifest
    // to set a key wins, matching how the package list keeps its first occurrence.
    let mut manifest_env = crate::goals::package_env::PackageEnv::new();
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
            Ok(expanded) => {
                effective.extend(expanded.packages);
                // First manifest wins, like the package list's first occurrence.
                merge_package_env(&mut manifest_env, expanded.package_env, false);
            }
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

    // Trigger over manifest, PER KEY. Per key and not per block on purpose: a
    // trigger overriding one setting must not silently discard the manifest's
    // other settings for that same package.
    let mut package_env = manifest_env;
    merge_package_env(&mut package_env, reg.def.package_env.clone(), true);

    if let Some((key, first, second)) = conflicting_package_env_key(&package_env) {
        return Err(format!(
            "package env sets {key} under both `#### {first}` and `#### {second}`: a key may \
             be configured by only one package"
        ));
    }

    Ok((deduped, package_env))
}

/// Fold `overlay` onto `base`, `overlay` winning per key.
///
/// Blocks are merged rather than replaced, so overriding one key leaves the rest of
/// that package's configuration intact.
///
/// `overwrite` states the precedence explicitly because this is used twice with
/// OPPOSITE precedence, and an unconditional insert silently gave last-wins to both:
/// manifest over manifest keeps the FIRST value (matching how the effective package
/// list keeps its first occurrence), while the trigger always overrides.
fn merge_package_env(
    base: &mut crate::goals::package_env::PackageEnv,
    overlay: crate::goals::package_env::PackageEnv,
    overwrite: bool,
) {
    for (package, keys) in overlay {
        let block = base.entry(package).or_default();
        for (key, value) in keys {
            match block.entry(key) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(value);
                }
                std::collections::btree_map::Entry::Occupied(mut slot) if overwrite => {
                    slot.insert(value);
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
        }
    }
}

/// The key that two different package blocks both configure, if any.
///
/// The trigger's own section rejects this, and each manifest is checked in
/// isolation, but their UNION is not — and the pod receives one flat environment,
/// so a conflict there is not ambiguous, it is fatal: the in-pod reader errors at
/// boot and the session never starts. Detecting it here turns a dead pod into a
/// demotion the author can see on their trigger issue.
fn conflicting_package_env_key(
    package_env: &crate::goals::package_env::PackageEnv,
) -> Option<(String, String, String)> {
    let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
    for (package, keys) in package_env {
        for key in keys.keys() {
            match owner.get(key.as_str()) {
                Some(first) if *first != package.as_str() => {
                    return Some((key.clone(), (*first).to_string(), package.clone()));
                }
                _ => {
                    owner.insert(key.as_str(), package.as_str());
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "effective_packages_tests.rs"]
mod tests;
