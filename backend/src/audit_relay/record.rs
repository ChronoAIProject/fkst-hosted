//! The relay's closed record vocabulary: what a stored row IS, and how far
//! through delivery it has got.
//!
//! ```text
//! started ──(completion)──> complete ──┐
//!    │                                 ├─> posthog_accepted ─> posthog_verified ─> purge
//!    └─(deadline + grace)─> incomplete ─┘        │
//!                                               └─> dead_letter   (retained)
//! ```
//!
//! Two rules the rest of the relay leans on:
//!
//! - **`posthog_accepted` is not `posthog_verified`.** A capture `2xx` means the
//!   payload was accepted, never that it is query-visible; the two are separate
//!   states with separate transitions precisely so nobody can rename one into the
//!   other (epic `AUD-07`).
//! - **`incomplete` is a real terminal state, not an error.** A process that died
//!   before producing a response has no status, and no system may invent one; the
//!   record stays queryable with `status_code = NULL` and the stable
//!   `request_incomplete` code.
//!
//! Both enums are closed and their wire strings are stable: they are column
//! values, metric label values, and part of the read API's response, so a
//! renamed variant is a breaking change in three places at once.

/// Which audit contract a stored row belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayRecordKind {
    /// One API request: a start, later a terminal completion or incomplete.
    ApiRequest,
    /// One sandbox lifecycle transition: terminal on arrival.
    SandboxLifecycle,
}

impl RelayRecordKind {
    pub const ALL: [RelayRecordKind; 2] = [
        RelayRecordKind::ApiRequest,
        RelayRecordKind::SandboxLifecycle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RelayRecordKind::ApiRequest => "api_request",
            RelayRecordKind::SandboxLifecycle => "sandbox_lifecycle",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// The ingress path that created the row (`kind` column). Distinct from
    /// [`RelayRecordKind::as_str`] (the `record_kind` column) because the read
    /// API speaks the activity vocabulary while the writer speaks the protocol's.
    pub fn ingress(self) -> &'static str {
        match self {
            RelayRecordKind::ApiRequest => "request",
            RelayRecordKind::SandboxLifecycle => "lifecycle",
        }
    }
}

/// How far through the outbox a stored row has got.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordState {
    /// Registered before its handler ran; no terminal projection yet.
    Started,
    /// A terminal projection is committed and awaiting capture.
    Complete,
    /// The completion never arrived; the relay synthesized a terminal
    /// `incomplete` projection with a null status.
    Incomplete,
    /// PostHog capture returned `2xx`. Accepted, NOT proven query-visible.
    PosthogAccepted,
    /// A fixed query read the event id back: genuinely query-visible.
    PosthogVerified,
    /// Delivery gave up permanently. Retained for the audit retention window and
    /// never auto-deleted.
    DeadLetter,
}

impl RecordState {
    pub const ALL: [RecordState; 6] = [
        RecordState::Started,
        RecordState::Complete,
        RecordState::Incomplete,
        RecordState::PosthogAccepted,
        RecordState::PosthogVerified,
        RecordState::DeadLetter,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RecordState::Started => "started",
            RecordState::Complete => "complete",
            RecordState::Incomplete => "incomplete",
            RecordState::PosthogAccepted => "posthog_accepted",
            RecordState::PosthogVerified => "posthog_verified",
            RecordState::DeadLetter => "dead_letter",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }

    /// Whether a terminal projection is committed for this state.
    ///
    /// Only these rows can be rendered on a timeline: a `started` row has no
    /// outcome, and inventing one would be exactly the fabricated status the
    /// epic forbids.
    pub fn has_terminal(self) -> bool {
        !matches!(self, RecordState::Started)
    }

    /// Whether this row is still waiting to be handed to PostHog capture.
    pub fn awaits_capture(self) -> bool {
        matches!(self, RecordState::Complete | RecordState::Incomplete)
    }

    /// The delivery state the activity merge speaks. The mapping is total and
    /// one-way: the relay's storage vocabulary never leaks past the read API.
    pub fn delivery_state(self) -> &'static str {
        match self {
            // A start has no terminal projection, so it is never returned by the
            // read API; the mapping exists so the match stays exhaustive.
            RecordState::Started | RecordState::Complete => "queued",
            RecordState::Incomplete => "incomplete",
            RecordState::PosthogAccepted => "accepted_pending_verification",
            RecordState::PosthogVerified => "verified_in_posthog",
            RecordState::DeadLetter => "dead_letter",
        }
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
