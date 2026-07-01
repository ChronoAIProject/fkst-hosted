//! The deterministic per-session id derivation shared by the reconciler and the
//! session-Pod naming convention.

/// Fixed namespace for the deterministic per-session UUIDv5. Constant so the
/// same `(installation_id, owner, name, issue_number)` always derives the same
/// session id — a webhook redelivery therefore maps to the SAME session, the
/// SAME `fkst-sess-<id>` Job name, and a `create` that no-ops on AlreadyExists
/// (the at-most-one-Job-per-session guarantee). The bytes are an arbitrary,
/// stable random UUID dedicated to fkst sessions.
const SESSION_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x9f, 0x2a, 0x4c, 0x6e, 0x1b, 0x83, 0x4d, 0x7a, 0xa5, 0xe0, 0x7c, 0x11, 0x3d, 0x52, 0x88, 0x64,
]);

/// Derive the deterministic session id for an issue-triggered session.
///
/// Returns the canonical lowercase, hyphenated UUID string (36 chars), which is
/// a valid DNS-1123 label component, so `fkst-sess-<id>` is a legal Kubernetes
/// object name (46 chars, within the 63-char limit). Same inputs → same id.
pub fn derive_session_id(
    installation_id: i64,
    owner: &str,
    name: &str,
    issue_number: i64,
) -> String {
    // A stable, unambiguous canonical name. `#` separates the issue number so
    // `(owner="a", name="b#1", issue=2)` cannot collide with
    // `(owner="a", name="b", issue="1#2")`-style ambiguity in practice.
    let canonical = format!("{installation_id}/{owner}/{name}#{issue_number}");
    uuid::Uuid::new_v5(&SESSION_NAMESPACE, canonical.as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_deterministic_for_the_same_inputs() {
        let a = derive_session_id(42, "acme", "site", 7);
        let b = derive_session_id(42, "acme", "site", 7);
        assert_eq!(
            a, b,
            "same inputs must derive the same id (redelivery dedup)"
        );
        // Canonical hyphenated lowercase UUID: 36 chars, fits fkst-sess-<id>.
        assert_eq!(a.len(), 36);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    #[test]
    fn session_id_differs_when_any_input_differs() {
        let base = derive_session_id(42, "acme", "site", 7);
        assert_ne!(base, derive_session_id(43, "acme", "site", 7));
        assert_ne!(base, derive_session_id(42, "other", "site", 7));
        assert_ne!(base, derive_session_id(42, "acme", "other", 7));
        assert_ne!(base, derive_session_id(42, "acme", "site", 8));
    }
}
