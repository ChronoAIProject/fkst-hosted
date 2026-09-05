//! The Evolution write boundary: a fixed prefix comparison.
//!
//! Evolution may create, modify or delete paths under `.fkst/evolution/`, and
//! nothing else — never `config.yaml`, never `intent/**`, never `.fkst/packages/`.
//!
//! **This module deliberately takes no configuration.** It lives apart from
//! [`super::config`] for exactly that reason: `config.yaml` is repository
//! content, so a boundary derived from it would be a boundary the thing it
//! bounds gets to move. Keeping the comparison fixed is what makes "repository
//! content cannot expand the agent's write boundary" true rather than
//! aspirational.
//!
//! The check is a **merge-time veto, not a write prevention**. An installation
//! token has no ref or path scope, so the credential cannot stop the write
//! itself; a branch ruleset is what stops a confined-at-merge path set from
//! being bypassed by a direct push. This is what detects it.

/// The single root Evolution writes under.
pub const EVOLUTION_ROOT: &str = ".fkst/evolution/";
/// Owner-controlled policy — read, never written.
pub const CONFIG_PATH: &str = ".fkst/evolution/config.yaml";
/// Owner-controlled intent — read, never written in a sync pull request.
pub const INTENT_PREFIX: &str = ".fkst/evolution/intent/";
/// The independent repo-local workflow catalog.
pub const PACKAGES_ROOT: &str = ".fkst/packages/";

/// Whether Evolution may write this path in a sync pull request.
pub fn is_writable_by_evolution(path: &str) -> bool {
    // A `..` segment is refused outright. Git trees never produce one, so it can
    // only arrive from generator-supplied input — and
    // `.fkst/evolution/../../backend/src/main.rs` passes a naive prefix test
    // while naming a file outside the root.
    if path.split('/').any(|segment| segment == "..") {
        return false;
    }
    // A symlink is compared by its OWN path, never by its target, and a submodule
    // pointer change is a change to the submodule path. Both fall out of
    // comparing the path string and nothing else.
    path.starts_with(EVOLUTION_ROOT) && path != CONFIG_PATH && !path.starts_with(INTENT_PREFIX)
}

/// Paths in `changed` that Evolution may not write, in the order given.
///
/// A non-empty result blocks the lane. It is returned rather than reduced to a
/// boolean so the diagnostic can name the offending paths — an operator told
/// only "confinement failed" has to go looking.
pub fn confinement_violations<'a, I>(changed: I) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    changed
        .into_iter()
        .filter(|path| !is_writable_by_evolution(path))
        .collect()
}

/// Whether a path is excluded from BOTH source fingerprints unconditionally.
///
/// Configuration cannot re-include these. `config.yaml` and `intent/**` are the
/// exception the caller adds back into the product-relevant fingerprint only —
/// an owner who rewrites product positioning must cause regeneration.
pub fn is_reserved(path: &str) -> bool {
    path.starts_with(EVOLUTION_ROOT) || path.starts_with(PACKAGES_ROOT)
}

/// Whether a path is owner-controlled intent folded into `productRelevant`.
pub fn is_owner_intent(path: &str) -> bool {
    path == CONFIG_PATH || path.starts_with(INTENT_PREFIX)
}

#[cfg(test)]
#[path = "boundary_tests.rs"]
mod tests;
