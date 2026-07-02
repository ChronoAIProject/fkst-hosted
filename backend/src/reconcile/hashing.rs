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
/// references, its work label, and its optional environment. Mirrors
/// [`crate::k8s::env_store_meta::content_hash`] (canonical JSON → SHA-256 hex) so a
/// live pod's recorded hash can be compared for drift. Stable and, for a fixed
/// package ORDER, deterministic (packages are author-ordered, so order is part of
/// the identity).
pub fn config_hash(packages: &[PackageRef], work_label: &str, environment: Option<&str>) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        packages: Vec<CanonPackage<'a>>,
        work_label: &'a str,
        environment: Option<&'a str>,
    }
    let canonical = Canonical {
        packages: canon_packages(packages),
        work_label,
        environment,
    };
    hex_digest(&canonical, "config-hash")
}

/// A stable content hash over a registration's FULL launch config — the superset of
/// [`config_hash`]: the ordered package refs, work label, environment, session name,
/// and BOTH opt-ins (`auto_merge`, `log_streaming`).
///
/// Where [`config_hash`] covers only the pod-affecting subset (so pod drift ignores
/// the two opt-ins), this covers everything the trigger author can set. It is the
/// basis for the immutability check: a registration whose full hash changed has had
/// *some* config edited, even one (like an opt-in) that does not respawn the pod.
///
/// Canonical form: SHA-256 over the canonical JSON of
/// `{packages, work_label, environment, name, auto_merge, log_streaming}`. The field
/// order below IS part of the canonical form (serde serialises in declaration order),
/// so identical inputs always hash identically and any changed field flips the hash.
pub fn full_config_hash(reg: &SessionRegistration) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        packages: Vec<CanonPackage<'a>>,
        work_label: &'a str,
        environment: Option<&'a str>,
        name: &'a str,
        auto_merge: bool,
        log_streaming: bool,
    }
    let canonical = Canonical {
        packages: canon_packages(&reg.def.packages),
        work_label: &reg.def.work_label,
        environment: reg.def.environment.as_deref(),
        name: &reg.def.name,
        auto_merge: reg.auto_merge,
        log_streaming: reg.log_streaming,
    };
    hex_digest(&canonical, "full-config-hash")
}
