//! In-memory `session_id -> log-access context` registry.
//!
//! The identity-gated log-download endpoint ([`crate::routes::logs`]) is reached
//! with only a `session_id` in the path (that is all the announce-comment link
//! carries). But `session_id` is a one-way UUIDv5 over `(installation, owner, repo,
//! issue)` (see [`crate::session_spec::derive_session_id`]) — it cannot be reversed
//! to recover the trigger context the authorization check needs (the issue author's
//! id + the `### Log Access Allowlist` allow-list).
//!
//! This registry is that reverse map. The reconciler already resolves EVERY open
//! trigger issue into a [`crate::reconcile::desired::SessionRegistration`] on each
//! per-repo sweep, so it upserts each one's context here as a cheap side effect;
//! the endpoint then looks it up. Both halves run in the same control-plane process
//! and share ONE [`LogAccessRegistry`] (a cheap `Arc`-backed handle) — the reconciler
//! writes, the endpoint reads.
//!
//! Consistency with the announce link: the announce comment (which carries the
//! download URL) is ALSO produced by the reconciler, so a URL only exists once the
//! reconciler is live and has seen the session — i.e. once its context is (or will,
//! next sweep, be) in this registry. The only gap is a cold control-plane restart:
//! the map is empty until the first sweep re-populates it (a bounded delay), during
//! which the endpoint returns 404 rather than serving unauthorized. That fail-closed
//! behaviour is deliberate.
//!
//! Nothing sensitive lives here: the allow-list is the public `### Log Access Allowlist`
//! content and the ids are public GitHub numeric ids — never a token or a secret.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::models::RepoRef;

/// The trigger context one session's log-download authorization needs, keyed in the
/// registry by the session's deterministic `session_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSessionContext {
    /// The GitHub App installation the session belongs to (traceability).
    pub installation_id: i64,
    /// The `owner/name` repository the session works (traceability).
    pub repo: RepoRef,
    /// The trigger issue number the session was launched from (traceability).
    pub trigger_issue: i64,
    /// The numeric GitHub id of the trigger issue's author (authz tier 1).
    pub author_id: i64,
    /// The frozen `### Log Access Allowlist` allow-list — logins/ids permitted to download the
    /// logs beyond the author + the global admins (authz tier 2).
    pub log_access: Vec<String>,
}

/// A shared, in-memory `session_id -> `[`LogSessionContext`] map. Cloning it shares
/// the same backing store (an `Arc`), so the reconciler (writer) and the endpoint
/// (reader) hold independent handles onto one registry.
#[derive(Clone, Default)]
pub struct LogAccessRegistry {
    inner: Arc<RwLock<HashMap<String, LogSessionContext>>>,
}

impl LogAccessRegistry {
    /// A fresh, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite the context for `session_id`. Poison-safe: a panic
    /// elsewhere while the lock was held never wedges the registry (the lock is
    /// recovered rather than propagated).
    pub fn upsert(&self, session_id: String, context: LogSessionContext) {
        let mut map = self.inner.write().unwrap_or_else(|e| e.into_inner());
        map.insert(session_id, context);
    }

    /// Look up the context for `session_id`, cloning it out so the lock is not held
    /// across the caller's subsequent (async) work. `None` when the session is
    /// unknown (never registered, or not yet re-populated after a restart).
    pub fn get(&self, session_id: &str) -> Option<LogSessionContext> {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        map.get(session_id).cloned()
    }

    /// The number of sessions currently tracked (diagnostics + tests).
    pub fn len(&self) -> usize {
        let map = self.inner.read().unwrap_or_else(|e| e.into_inner());
        map.len()
    }

    /// Whether the registry is empty (diagnostics + tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for LogAccessRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Render only the size, never the entries (keeps a `{:?}` of AppState cheap
        // and avoids incidentally dumping the whole map into a log line).
        f.debug_struct("LogAccessRegistry")
            .field("sessions", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(author_id: i64, allow: &[&str]) -> LogSessionContext {
        LogSessionContext {
            installation_id: 1,
            repo: RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
            trigger_issue: 7,
            author_id,
            log_access: allow.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn upsert_then_get_round_trips() {
        let reg = LogAccessRegistry::new();
        assert!(reg.is_empty());
        reg.upsert("sess-1".to_string(), ctx(42, &["alice"]));
        let got = reg.get("sess-1").expect("present after upsert");
        assert_eq!(got.author_id, 42);
        assert_eq!(got.log_access, vec!["alice".to_string()]);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn get_unknown_session_is_none() {
        let reg = LogAccessRegistry::new();
        assert!(reg.get("nope").is_none());
    }

    #[test]
    fn upsert_overwrites_prior_context() {
        let reg = LogAccessRegistry::new();
        reg.upsert("sess-1".to_string(), ctx(42, &["alice"]));
        reg.upsert("sess-1".to_string(), ctx(42, &["alice", "bob"]));
        assert_eq!(reg.len(), 1, "same key overwrites, not appends");
        assert_eq!(
            reg.get("sess-1").unwrap().log_access,
            vec!["alice".to_string(), "bob".to_string()]
        );
    }

    #[test]
    fn a_clone_shares_the_same_backing_store() {
        let reg = LogAccessRegistry::new();
        let handle = reg.clone();
        handle.upsert("sess-1".to_string(), ctx(1, &[]));
        assert!(
            reg.get("sess-1").is_some(),
            "a write through one handle is visible through another"
        );
    }

    #[test]
    fn debug_reports_only_the_size() {
        let reg = LogAccessRegistry::new();
        reg.upsert("sess-1".to_string(), ctx(1, &["alice"]));
        let debug = format!("{reg:?}");
        assert!(debug.contains("sessions"), "{debug}");
        assert!(
            !debug.contains("alice"),
            "entries must not be dumped: {debug}"
        );
    }
}
