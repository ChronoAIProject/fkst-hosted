//! Internal inventory warnings, projected onto bounded PUBLIC codes.
//!
//! [`crate::session_backend::inventory::BoundedInventoryWarning`] is already a
//! closed code, but it is an INTERNAL value carrying the runtime and session id
//! of whatever it concerns — including runtimes the caller may not see. Handing
//! that list to a client verbatim would announce the existence of hidden rows,
//! which is precisely what epic `AUTH-06` forbids. So the projection is:
//!
//! ```text
//! each AUTHORIZED, filter-matching row's OWN codes
//!        -> attach the public codes to THAT item
//! snapshot-scope warnings (naming no runtime)
//!        -> global scope only
//! ```
//!
//! The per-row half deliberately reads the ROW's codes
//! ([`crate::session_backend::inventory::RuntimeInventoryItem::warnings`]) rather
//! than filtering the snapshot's list by runtime id: that list is capped FIFO
//! across the whole fleet, so a row's codes would otherwise depend on how many
//! warning-emitting runtimes — visible or hidden — happened to precede it.
//!
//! ## Why snapshot-scope warnings are administrator-only
//!
//! A snapshot-scope warning is a statement about the whole fleet. If a regular
//! caller's response carried one, then adding a malformed runtime they cannot see
//! would change their response — and "a hidden row cannot change my answer" is the
//! isolation property this endpoint is tested for byte-for-byte. Deployment-wide
//! inventory health remains fully visible to a global administrator here, and to
//! every operator on `/metrics`.
//!
//! ## Truncated SOURCE reads are not a warning at all
//!
//! A clipped page walk means the fleet read was INCOMPLETE, so no authorized
//! answer derived from it can honestly claim to be the complete matching set.
//! [`super::service`] turns that into `503 sandbox_inventory_too_large` before any
//! projection runs; it deliberately has no public warning code.

use crate::session_backend::inventory::InventoryWarningCode;

/// A bounded, stable warning code a client may act on.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SandboxWarningCode {
    /// The runtime carries no session-id stamp (an orphan). Global scope only —
    /// such a row can never be authorized for a regular caller.
    MissingSessionId,
    /// A correlation value is present but does not parse.
    MalformedCorrelation,
    /// An attribution id is present but does not parse.
    MalformedIdentity,
    /// The runtime's stamped attribution was observed to disagree with its
    /// trigger. The stamp is reported verbatim; this is how the disputed row is
    /// findable.
    AttributionConflict,
    /// No creation timestamp, so age/expiry/idle cannot be derived.
    MissingCreatedAt,
    /// A creation timestamp is present but unparseable.
    MalformedCreatedAt,
    /// The last-pending marker is present but unparseable; idle timing fell back
    /// to the creation time.
    MalformedLastPending,
    /// A timestamp lies in the future relative to the snapshot instant; the
    /// derived duration was clamped to zero rather than reported negative.
    ClockSkew,
    /// `created_at + max_lifetime` overflowed, so expiry and remaining are null.
    LifetimeOverflow,
    /// The backend reported a state this build does not map; `raw_status` carries
    /// it verbatim.
    UnknownStatus,
    /// The snapshot's operator-facing warning list hit its ceiling, so the
    /// deployment-wide diagnostic is not exhaustive. Returned rows keep their own
    /// complete codes. Snapshot scope: global administrators only.
    WarningsIncomplete,
}

impl SandboxWarningCode {
    /// Every variant, in the fixed order a response renders them.
    pub const ALL: [SandboxWarningCode; 11] = [
        SandboxWarningCode::MissingSessionId,
        SandboxWarningCode::MalformedCorrelation,
        SandboxWarningCode::MalformedIdentity,
        SandboxWarningCode::AttributionConflict,
        SandboxWarningCode::MissingCreatedAt,
        SandboxWarningCode::MalformedCreatedAt,
        SandboxWarningCode::MalformedLastPending,
        SandboxWarningCode::ClockSkew,
        SandboxWarningCode::LifetimeOverflow,
        SandboxWarningCode::UnknownStatus,
        SandboxWarningCode::WarningsIncomplete,
    ];

    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxWarningCode::MissingSessionId => "missing_session_id",
            SandboxWarningCode::MalformedCorrelation => "malformed_correlation",
            SandboxWarningCode::MalformedIdentity => "malformed_identity",
            SandboxWarningCode::AttributionConflict => "attribution_conflict",
            SandboxWarningCode::MissingCreatedAt => "missing_created_at",
            SandboxWarningCode::MalformedCreatedAt => "malformed_created_at",
            SandboxWarningCode::MalformedLastPending => "malformed_last_pending",
            SandboxWarningCode::ClockSkew => "clock_skew",
            SandboxWarningCode::LifetimeOverflow => "lifetime_overflow",
            SandboxWarningCode::UnknownStatus => "unknown_status",
            SandboxWarningCode::WarningsIncomplete => "warnings_incomplete",
        }
    }
}

impl std::fmt::Display for SandboxWarningCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Project one internal code onto its public code.
///
/// `None` means "not a public warning": [`InventoryWarningCode::SourceTruncated`]
/// is an incompleteness FAILURE the service answers with `503` before this
/// projection ever runs, so it has no code to render.
pub fn public_code(code: InventoryWarningCode) -> Option<SandboxWarningCode> {
    match code {
        InventoryWarningCode::MissingSessionId => Some(SandboxWarningCode::MissingSessionId),
        InventoryWarningCode::MalformedCorrelation => {
            Some(SandboxWarningCode::MalformedCorrelation)
        }
        InventoryWarningCode::MalformedIdentity => Some(SandboxWarningCode::MalformedIdentity),
        InventoryWarningCode::AttributionConflict => Some(SandboxWarningCode::AttributionConflict),
        InventoryWarningCode::MissingCreatedAt => Some(SandboxWarningCode::MissingCreatedAt),
        InventoryWarningCode::MalformedCreatedAt => Some(SandboxWarningCode::MalformedCreatedAt),
        InventoryWarningCode::MalformedLastPending => {
            Some(SandboxWarningCode::MalformedLastPending)
        }
        InventoryWarningCode::ClockSkew => Some(SandboxWarningCode::ClockSkew),
        InventoryWarningCode::LifetimeOverflow => Some(SandboxWarningCode::LifetimeOverflow),
        InventoryWarningCode::UnknownStatus => Some(SandboxWarningCode::UnknownStatus),
        InventoryWarningCode::WarningsTruncated => Some(SandboxWarningCode::WarningsIncomplete),
        InventoryWarningCode::SourceTruncated => None,
    }
}

/// Sort and de-duplicate a set of codes into the fixed rendering order.
pub fn normalize(mut codes: Vec<SandboxWarningCode>) -> Vec<SandboxWarningCode> {
    codes.sort_unstable();
    codes.dedup();
    codes
}

#[cfg(test)]
#[path = "warning_tests.rs"]
mod tests;
