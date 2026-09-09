//! Per-row session authorization for the live inventory.
//!
//! One runtime, one decision, no I/O beyond an in-memory registry read — which is
//! the whole point: a fleet read must never become N GitHub calls (epic
//! `SBOX-04`). Every tier the decision may use lives in
//! [`crate::session_access::policy`], so this module contributes no new notion of
//! "may see".
//!
//! ## The three ways a row is hidden in `accessible`
//!
//! 1. **No usable session id.** An orphan (no stamp) or a malformed stamp cannot
//!    be looked up at all. It is hidden — never charitably matched by creator
//!    annotation, repository, or name.
//! 2. **No registry context.** The projection is ready and simply does not know
//!    that session: it is retired, foreign, or was never a trigger. Fail closed.
//! 3. **A denying policy decision.** Evaluated WITHOUT the deployment
//!    global-admin bypass, so an administrator who deliberately selected
//!    `accessible` sees exactly what they directly own or were granted.
//!
//! A `Unavailable` registry answer is an ERROR, not a hidden row. If the
//! projection stopped being trustworthy between the readiness gate and this
//! decision, the honest answer is `503` — silently dropping every row would
//! render as a confident, complete, empty fleet, which is the one failure mode
//! the readiness machinery exists to prevent.
//!
//! ## `all` is not "skip authorization"
//!
//! It is the verified global-administrator scope, resolved server-side from the
//! deployment access policy by [`crate::session_access::resolve_operations_scope`]
//! before this module is ever reached. A [`ViewerScope::All`] value cannot be
//! constructed from request input.

use crate::access_policy::AccessPolicy;
use crate::audit::arguments::bounds::safe_session_id;
use crate::error::AppError;
use crate::session_access::{
    decide, AuthenticatedViewer, ContextLookup, PolicyEnvironment, SessionAccessRegistry,
    SessionAccessRequest, SessionCapability, VerifiedCaller, ViewerScope,
};
use crate::session_backend::inventory::RuntimeInventoryItem;

/// The bounded reason one row was dropped. Structured-log and test detail only —
/// it never reaches a response, a count, or a metric label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HiddenReason {
    /// No session-id stamp, or one that is not a valid session id.
    UnusableSessionId,
    /// The ready projection has no context for that session.
    UnknownContext,
    /// The capability tiers denied this caller.
    NotAuthorized,
}

impl HiddenReason {
    /// Every variant, in the fixed order [`HiddenTally`] reports them.
    pub const ALL: [HiddenReason; 3] = [
        HiddenReason::UnusableSessionId,
        HiddenReason::UnknownContext,
        HiddenReason::NotAuthorized,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            HiddenReason::UnusableSessionId => "unusable_session_id",
            HiddenReason::UnknownContext => "unknown_context",
            HiddenReason::NotAuthorized => "not_authorized",
        }
    }

    /// This reason's slot in [`HiddenTally`].
    fn slot(self) -> usize {
        match self {
            HiddenReason::UnusableSessionId => 0,
            HiddenReason::UnknownContext => 1,
            HiddenReason::NotAuthorized => 2,
        }
    }
}

/// How many rows one request withheld, per reason.
///
/// Aggregated rather than logged row by row: a five-thousand-runtime fleet must
/// not be able to turn one inventory read into five thousand log lines. It holds
/// counts and closed reasons ONLY — never a runtime id, a session id, or the
/// probed filter — so an operator answering "why is my sandbox not listed?" can
/// see which gate withheld it, while the trail never becomes the enumeration
/// oracle the response refuses to be. These numbers are operator diagnostics and
/// reach no caller, no response field, and no metric label.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HiddenTally {
    counts: [u64; HiddenReason::ALL.len()],
}

impl HiddenTally {
    /// Count one withheld row.
    pub fn record(&mut self, reason: HiddenReason) {
        self.counts[reason.slot()] = self.counts[reason.slot()].saturating_add(1);
    }

    /// How many rows this reason withheld.
    pub fn count(&self, reason: HiddenReason) -> u64 {
        self.counts[reason.slot()]
    }

    /// Emit at most one line per reason, and nothing at all when the caller was
    /// shown everything the fleet holds.
    pub fn trace(&self) {
        for reason in HiddenReason::ALL {
            let count = self.count(reason);
            if count > 0 {
                tracing::debug!(
                    reason = reason.as_str(),
                    count,
                    "operations: inventory rows withheld from this caller"
                );
            }
        }
    }
}

/// A reusable, borrowed decision context: built once per request, applied to
/// every listed runtime.
pub struct RowAuthorizer<'a> {
    registry: &'a SessionAccessRegistry,
    viewer: &'a AuthenticatedViewer,
    scope: &'a ViewerScope,
    access: &'a AccessPolicy,
    legacy_log_admins: &'a [String],
}

impl<'a> RowAuthorizer<'a> {
    /// Build the authorizer for one request.
    pub fn new(
        registry: &'a SessionAccessRegistry,
        viewer: &'a AuthenticatedViewer,
        scope: &'a ViewerScope,
        access: &'a AccessPolicy,
        legacy_log_admins: &'a [String],
    ) -> Self {
        Self {
            registry,
            viewer,
            scope,
            access,
            legacy_log_admins,
        }
    }

    /// Whether this caller may see one listed runtime.
    ///
    /// `Ok(None)` means visible; `Ok(Some(reason))` means hidden. The error arm is
    /// reserved for a projection that cannot answer at all.
    pub fn decide_row(
        &self,
        item: &RuntimeInventoryItem,
    ) -> Result<Option<HiddenReason>, AppError> {
        if self.scope.is_global() {
            // A verified global administrator sees every managed runtime,
            // including the malformed, orphan, conflict, and unknown-legacy rows
            // no regular user can be given (epic `AUTH-05`).
            return Ok(None);
        }
        let Some(session_id) = item.session_id.as_deref().and_then(safe_session_id) else {
            return Ok(Some(HiddenReason::UnusableSessionId));
        };
        let context = match self.registry.lookup(&session_id) {
            ContextLookup::Found(context) => context,
            ContextLookup::Unknown => return Ok(Some(HiddenReason::UnknownContext)),
            ContextLookup::Unavailable => {
                tracing::info!(
                    "operations: session visibility projection stopped answering mid-inventory"
                );
                return Err(AppError::SessionVisibilityUnavailable(
                    "session visibility is still recovering; retry shortly".to_string(),
                ));
            }
        };
        let request = SessionAccessRequest::new(
            SessionCapability::OperationsVisibility,
            VerifiedCaller::from_verified_user(self.viewer.user()),
            context.facts(),
            PolicyEnvironment {
                access: self.access,
                legacy_log_admins: self.legacy_log_admins,
                github_bot_login: None,
            },
        )
        // `accessible` evaluates even an administrator on their DIRECT tiers, so
        // "the sessions I own or was granted" is a view they can actually select.
        .without_global_admin();
        if decide(&request).allowed() {
            Ok(None)
        } else {
            Ok(Some(HiddenReason::NotAuthorized))
        }
    }
}

#[cfg(test)]
#[path = "authorize_tests.rs"]
mod tests;
