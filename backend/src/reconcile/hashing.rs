//! The reconciler's content hashes over a session's launch config.
//!
//! Two pure, deterministic SHA-256-over-canonical-JSON digests, split out of
//! [`crate::reconcile::desired`] to keep that module focused on the desired-state
//! types + planner:
//!
//! - [`config_hash`] — the POD-AFFECTING subset (packages, work label, environment).
//!   A live pod's recorded hash is compared against this for drift (respawn on change).
//! - [`full_config_hash`] — the FULL superset (the above + session name + both
//!   opt-ins). The basis of the config-immutability check: any edited field flips it,
//!   even an opt-in that does not respawn the pod.
//!
//! Both project each `PackageRef` through a borrow-only canonical struct (so
//! `PackageRef` need not be `Serialize`); the field set + order IS the canonical form
//! (serde serialises in declaration order), so identical inputs always hash identically.

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::goals::trigger_parse::PackageRef;
use crate::reconcile::desired::SessionRegistration;

/// A borrow-only projection of a package so `PackageRef` need not itself be
/// `Serialize`; the field set + order is the canonical package identity.
#[derive(Serialize)]
struct CanonPackage<'a> {
    owner: &'a str,
    repo: &'a str,
    git_ref: &'a str,
    path: &'a str,
}

/// Canonicalise a package list into the borrow-only projection both hashes share.
fn canon_packages(packages: &[PackageRef]) -> Vec<CanonPackage<'_>> {
    packages
        .iter()
        .map(|p| CanonPackage {
            owner: &p.owner,
            repo: &p.repo,
            git_ref: &p.git_ref,
            path: &p.path,
        })
        .collect()
}

/// SHA-256 hex over the canonical JSON `value`.
fn hex_digest<T: Serialize>(value: &T, what: &str) -> String {
    let json =
        serde_json::to_vec(value).unwrap_or_else(|_| panic!("canonical {what} json is infallible"));
    let digest = Sha256::digest(&json);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A stable content hash over a session's launch inputs: its ordered package
/// references, its work label, its optional environment, and its optional output
/// language. Mirrors [`crate::k8s::env_store_meta::content_hash`] (canonical JSON
/// → SHA-256 hex) so a live pod's recorded hash can be compared for drift. Stable
/// and, for a fixed package ORDER, deterministic (packages are author-ordered, so
/// order is part of the identity).
///
/// DIGEST-STABILITY INVARIANT: fields added after the original trio serialize
/// ONLY when set (`skip_serializing_if`), so a config that does not use them
/// hashes byte-identically across deploys. Without the skip, every live
/// session's recomputed hash would differ from the one latched at announce —
/// tripping the immutability check fleet-wide (false `fkst-config-rejected` +
/// spawn suppression). Guarded by `config_hash_is_digest_stable_for_old_configs`.
pub fn config_hash(
    packages: &[PackageRef],
    work_label: Option<&str>,
    environment: Option<&str>,
    output_lang: Option<&str>,
    engine_config: &BTreeMap<String, String>,
) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        packages: Vec<CanonPackage<'a>>,
        // Skip-if-none keeps the digest stable: a `Some("x")` serializes
        // byte-identically to the pre-optional `&str "x"`, so existing sessions
        // never drift; an absent label simply omits the field.
        #[serde(skip_serializing_if = "Option::is_none")]
        work_label: Option<&'a str>,
        environment: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_lang: Option<&'a str>,
        #[serde(skip_serializing_if = "BTreeMap::is_empty")]
        engine_config: &'a BTreeMap<String, String>,
    }
    let canonical = Canonical {
        packages: canon_packages(packages),
        work_label,
        environment,
        output_lang,
        engine_config,
    };
    hex_digest(&canonical, "config-hash")
}

/// A stable content hash over a registration's FULL launch config — the superset of
/// [`config_hash`]: the ordered package refs, work label, environment, session name,
/// the `auto_merge` opt-in, and the `log_access` allow-list.
///
/// Where [`config_hash`] covers only the pod-affecting subset (so pod drift ignores
/// the opt-ins), this covers everything the trigger author can set. It is the basis of
/// the immutability check: a registration whose full hash changed has had *some* config
/// edited, even one (like the auto-merge opt-in) that does not respawn the pod.
///
/// Canonical form: SHA-256 over the canonical JSON of
/// `{packages, work_label, environment, name, auto_merge, log_access}`. The field order
/// below IS part of the canonical form (serde serialises in declaration order), so
/// identical inputs always hash identically and any changed field flips the hash —
/// including `log_access`, so the log allow-list is FROZEN by the config-immutability
/// check (it cannot be widened after registration).
pub fn full_config_hash(reg: &SessionRegistration) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        packages: Vec<CanonPackage<'a>>,
        // See config_hash: skip-if-none keeps old (Some) configs byte-stable.
        #[serde(skip_serializing_if = "Option::is_none")]
        work_label: Option<&'a str>,
        environment: Option<&'a str>,
        name: &'a str,
        auto_merge: bool,
        log_access: &'a [String],
        // Serialized ONLY when set — see the digest-stability invariant on
        // [`config_hash`]; an old config must hash identically after a deploy.
        #[serde(skip_serializing_if = "Option::is_none")]
        output_lang: Option<&'a str>,
        #[serde(skip_serializing_if = "BTreeMap::is_empty")]
        engine_config: &'a BTreeMap<String, String>,
    }
    let canonical = Canonical {
        packages: canon_packages(&reg.def.packages),
        work_label: reg.def.work_label.as_deref(),
        environment: reg.def.environment.as_deref(),
        name: &reg.def.name,
        auto_merge: reg.auto_merge,
        log_access: &reg.log_access,
        output_lang: reg.def.output_lang.as_deref(),
        engine_config: &reg.def.engine_config,
    };
    hex_digest(&canonical, "full-config-hash")
}
