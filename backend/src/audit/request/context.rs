//! The per-request audit context: write-once slots an extractor or handler fills
//! and the outer middleware reads back after the response exists.
//!
//! ## Why slots instead of "the middleware just reads the request"
//!
//! By the time the middleware sees a response it has already moved the `Request`
//! into `next.run(req)`; anything an extractor wrote into request extensions is
//! gone. So the middleware installs a cheap `Arc`-shared context *before*
//! dispatch and keeps its own clone. Whoever proves a fact — the verified
//! identity, the session a call touched, the safe arguments — writes it there,
//! and the middleware reads it afterwards.
//!
//! ## Write-once, first-verified-wins, never concatenated
//!
//! Each slot holds one value. A second write of the SAME value is a harmless
//! no-op (two layers agreeing). A second write of a DIFFERENT value is a
//! programmer error: it means two places each believe they own the field. The
//! resolution is fail-closed — keep the first, count the conflict, and log the
//! field NAME only. Concatenating or preferring the later value would let a
//! later, less-verified writer overwrite a verified one, which is exactly the
//! attack the verified-actor contract exists to prevent.
//!
//! ## What must never be stored here
//!
//! The request, its body bytes, its header map, its URI, any token, the response
//! body, or an arbitrary error object. The slots below are the complete list —
//! bounded, typed, credential-free values only (epic `AUD-03`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::http::Extensions;

use crate::audit::event::{ArgumentsParseStatus, Correlation};
use crate::audit::identity::{AuditIdentity, AuditIdentitySlot};

/// The allowlisted arguments for one call, plus how they were obtained.
///
/// Populated by the endpoint argument contract (a separate issue); the middleware
/// only carries whatever is present at completion.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditArguments {
    pub values: serde_json::Map<String, serde_json::Value>,
    pub status: ArgumentsParseStatus,
}

impl Default for AuditArguments {
    /// "No argument contract ran" — written by hand rather than derived because
    /// [`ArgumentsParseStatus`] is a closed wire enum with no meaningful default
    /// of its own; `NotApplicable` is this middleware's convention for a call
    /// whose arguments were never extracted.
    fn default() -> Self {
        Self {
            values: serde_json::Map::new(),
            status: ArgumentsParseStatus::NotApplicable,
        }
    }
}

/// One write-once slot with conflict accounting.
#[derive(Debug)]
struct Slot<T> {
    value: Mutex<Option<T>>,
    conflicts: AtomicU32,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
            conflicts: AtomicU32::new(0),
        }
    }
}

impl<T: Clone + PartialEq> Slot<T> {
    /// Record `value`, keeping whatever was written first.
    fn record(&self, field: &'static str, value: T) {
        let mut guard = self.lock();
        match guard.as_ref() {
            None => *guard = Some(value),
            // Two writers agreeing is normal (e.g. a route parameter and the
            // resolved resource naming the same session).
            Some(existing) if *existing == value => {}
            Some(_) => {
                self.conflicts.fetch_add(1, Ordering::Relaxed);
                // The FIELD is named; the values are not. A conflicting value may
                // itself be sensitive, and the field name is enough to find the
                // offending writer.
                tracing::error!(
                    field = field,
                    "audit request context received a conflicting write; keeping the first value"
                );
            }
        }
    }

    fn get(&self) -> Option<T> {
        self.lock().clone()
    }

    fn conflicts(&self) -> u32 {
        self.conflicts.load(Ordering::Relaxed)
    }

    fn is_filled(&self) -> bool {
        self.lock().is_some()
    }

    /// A poisoned mutex only means some task panicked while holding it; the
    /// recorded value is still readable, so recover rather than double-panic on
    /// a path that must never fail a product request.
    fn lock(&self) -> MutexGuard<'_, Option<T>> {
        self.value.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, Default)]
struct ContextInner {
    identity: AuditIdentitySlot,
    arguments: Slot<AuditArguments>,
    error_code: Slot<String>,
    session_id: Slot<String>,
    repo_full_name: Slot<String>,
    installation_id: Slot<i64>,
    trigger_issue: Slot<i64>,
    webhook_delivery_id: Slot<String>,
}

/// The cloneable per-request context stored in request extensions.
///
/// Cloning shares the same slots — that is the entire point: the middleware and
/// every extractor hold independent handles onto one value.
#[derive(Clone, Default)]
pub struct AuditRequestContext {
    inner: Arc<ContextInner>,
}

impl AuditRequestContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install this context into a request's extensions.
    ///
    /// The identity slot is inserted alongside it so
    /// [`crate::audit::identity::record_identity`] keeps working unchanged for
    /// every site that already proves an identity; both handles point at the
    /// same cell.
    pub fn install(&self, extensions: &mut Extensions) {
        extensions.insert(self.inner.identity.clone());
        extensions.insert(self.clone());
    }

    /// The context installed on a request, if any.
    pub fn from_extensions(extensions: &Extensions) -> Option<Self> {
        extensions.get::<Self>().cloned()
    }

    /// The verified initiating identity. Delegates to the shared slot so both
    /// entry points agree.
    pub fn record_identity(&self, identity: AuditIdentity) {
        self.inner.identity.record(identity);
    }

    pub fn record_arguments(
        &self,
        values: serde_json::Map<String, serde_json::Value>,
        status: ArgumentsParseStatus,
    ) {
        self.inner
            .arguments
            .record("arguments", AuditArguments { values, status });
    }

    /// A stable snake_case application code. Never error text: the event
    /// contract rejects anything else, and a message could carry request data.
    pub fn record_error_code(&self, code: impl Into<String>) {
        self.inner.error_code.record("error_code", code.into());
    }

    pub fn record_session_id(&self, session_id: impl Into<String>) {
        self.inner
            .session_id
            .record("session_id", session_id.into());
    }

    /// `owner/name`.
    pub fn record_repo_full_name(&self, repo: impl Into<String>) {
        self.inner
            .repo_full_name
            .record("repo_full_name", repo.into());
    }

    pub fn record_installation_id(&self, installation_id: i64) {
        self.inner
            .installation_id
            .record("installation_id", installation_id);
    }

    pub fn record_trigger_issue(&self, issue_number: i64) {
        self.inner
            .trigger_issue
            .record("trigger_issue", issue_number);
    }

    /// GitHub's `X-GitHub-Delivery` UUID, recorded only after the signature over
    /// the raw body verified.
    pub fn record_webhook_delivery_id(&self, delivery_id: impl Into<String>) {
        self.inner
            .webhook_delivery_id
            .record("webhook_delivery_id", delivery_id.into());
    }

    /// Take an immutable snapshot for the terminal record.
    ///
    /// Uses the neutral `not_applicable` default; the middleware calls
    /// [`Self::freeze_with_default`] with the operation's own declared default so
    /// a request rejected before its safe parse could run is classified as
    /// `unavailable` rather than as an operation that has no arguments.
    pub fn freeze(&self) -> FrozenRequestContext {
        self.freeze_with_default(ArgumentsParseStatus::NotApplicable)
    }

    /// Take an immutable snapshot, classifying "nothing was recorded" as
    /// `default_status`.
    pub fn freeze_with_default(
        &self,
        default_status: ArgumentsParseStatus,
    ) -> FrozenRequestContext {
        let arguments = self.inner.arguments.get().unwrap_or(AuditArguments {
            values: serde_json::Map::new(),
            status: default_status,
        });
        FrozenRequestContext {
            identity: self
                .inner
                .identity
                .get()
                .unwrap_or_else(AuditIdentity::anonymous),
            arguments: arguments.values,
            arguments_parse_status: arguments.status,
            error_code: self.inner.error_code.get(),
            correlation: Correlation {
                session_id: self.inner.session_id.get(),
                repo_full_name: self.inner.repo_full_name.get(),
                installation_id: self.inner.installation_id.get(),
                trigger_issue: self.inner.trigger_issue.get(),
                webhook_delivery_id: self.inner.webhook_delivery_id.get(),
            },
            conflicts: self.conflicts(),
        }
    }

    /// Total conflicting writes observed across every slot.
    pub fn conflicts(&self) -> u32 {
        let inner = &self.inner;
        inner
            .identity
            .conflicts()
            .saturating_add(inner.arguments.conflicts())
            .saturating_add(inner.error_code.conflicts())
            .saturating_add(inner.session_id.conflicts())
            .saturating_add(inner.repo_full_name.conflicts())
            .saturating_add(inner.installation_id.conflicts())
            .saturating_add(inner.trigger_issue.conflicts())
            .saturating_add(inner.webhook_delivery_id.conflicts())
    }
}

impl std::fmt::Debug for AuditRequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Which slots are filled, never what is in them: a `{:?}` of a request
        // must not dump a login, a session id, or a repository name into a log
        // line nobody asked to contain one.
        let inner = &self.inner;
        f.debug_struct("AuditRequestContext")
            .field("identity", &inner.identity)
            .field("arguments", &inner.arguments.is_filled())
            .field("error_code", &inner.error_code.is_filled())
            .field("session_id", &inner.session_id.is_filled())
            .field("repo_full_name", &inner.repo_full_name.is_filled())
            .field("installation_id", &inner.installation_id.is_filled())
            .field("trigger_issue", &inner.trigger_issue.is_filled())
            .field(
                "webhook_delivery_id",
                &inner.webhook_delivery_id.is_filled(),
            )
            .field("conflicts", &self.conflicts())
            .finish()
    }
}

/// An immutable snapshot of the context, taken once the response exists.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenRequestContext {
    pub identity: AuditIdentity,
    pub arguments: serde_json::Map<String, serde_json::Value>,
    pub arguments_parse_status: ArgumentsParseStatus,
    pub error_code: Option<String>,
    pub correlation: Correlation,
    pub conflicts: u32,
}

/// Record a fact on the context installed for `extensions`, if one is installed.
///
/// A no-op without a context, so any call site can use it unconditionally.
pub fn with_context(extensions: &Extensions, record: impl FnOnce(&AuditRequestContext)) {
    if let Some(context) = extensions.get::<AuditRequestContext>() {
        record(context);
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
