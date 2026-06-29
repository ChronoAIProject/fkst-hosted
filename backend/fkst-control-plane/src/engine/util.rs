//! Small engine-side helpers with no store dependency.
//!
//! `is_valid_name` lives here (not in the now-removed `packages` store) because
//! it is the engine's *identity rule*: a package is identified by its directory
//! basename, and that basename must fully match `[A-Za-z0-9_-]+` (the same rule
//! the substrate engine enforces). Sessions, the distributor lease key, the
//! goal edge, and the repo-scoped package resolver all share this one rule, so
//! it belongs with the engine plumbing rather than a domain store.

use std::sync::OnceLock;

use regex::Regex;

/// Anchored package-name pattern (also the substrate engine's identity rule).
fn name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_-]+$").expect("static name regex"))
}

/// True when `name` is a valid package name (fully matches `[A-Za-z0-9_-]+`).
///
/// A valid name is always ASCII, path-segment-safe, and URL-path-safe (no `/`,
/// `.`, `$`, whitespace, or NUL can pass), so callers can use it as a directory
/// basename, a lease-key component, or a URL path parameter without further
/// escaping.
pub fn is_valid_name(name: &str) -> bool {
    name_regex().is_match(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_engine_identity_names() {
        for name in ["demo", "My-Pkg_01", "a", "0", "_", "-", "A-Za-z0-9_-"] {
            assert!(is_valid_name(name), "must accept {name:?}");
        }
    }

    #[test]
    fn rejects_invalid_names() {
        for name in [
            "", "a b", "a/b", "a.b", "a$b", "../x", "a\nb", "a\u{0}b", " demo", "demo ", "héllo",
            "host ", // a space makes it invalid; the literal "host" is rejected by callers, not here
        ] {
            assert!(!is_valid_name(name), "must reject {name:?}");
        }
    }
}
