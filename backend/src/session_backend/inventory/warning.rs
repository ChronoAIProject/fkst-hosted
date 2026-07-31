//! Bounded, closed-code inventory warnings.
//!
//! A warning exists so that "this runtime's data is incomplete" is VISIBLE rather
//! than silently smoothed over. It therefore carries no free text and no backend
//! error message — only a closed code plus the correlation ids #5675 needs to
//! attach it to an already-authorized row (attaching it to an UNauthorized row
//! would leak the existence of a hidden runtime, so the code/ids are the entire
//! payload and the caller decides what a viewer may see).

/// What went wrong with one runtime's data, or with the snapshot as a whole.
///
/// Closed by design: these values become bounded metric/response codes downstream,
/// so a new variant is a deliberate contract change, never an interpolated string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryWarningCode {
    /// A managed runtime carries no session-id stamp — an orphan. It is RETAINED
    /// (global admins must be able to see it); regular-user exclusion is #5675's.
    MissingSessionId,
    /// A correlation value is present but does not parse (a non-integer
    /// installation id or trigger issue).
    MalformedCorrelation,
    /// The attribution stamp holds a non-integer id.
    MalformedIdentity,
    /// The runtime reports no creation timestamp, so age/expiry/idle cannot be
    /// derived. Substituting `now` is forbidden — it would make an old runtime
    /// look new.
    MissingCreatedAt,
    /// A creation timestamp is present but unparseable.
    MalformedCreatedAt,
    /// The last-pending marker is present but unparseable; idle timing falls back
    /// to the creation time exactly as the reconciler does.
    MalformedLastPending,
    /// A timestamp lies in the future relative to the snapshot instant. The
    /// derived duration is clamped to zero rather than reported negative.
    ClockSkew,
    /// `created_at + max_lifetime` overflowed the representable range, so expiry
    /// and remaining time are null. The configured maximum is still reported.
    LifetimeOverflow,
    /// The backend reported a state this build does not map, preserved verbatim in
    /// `raw_status`.
    UnknownStatus,
    /// The backend page walk stopped at its safety cap, so the item list may be
    /// short. Never silent.
    SourceTruncated,
    /// More warnings occurred than one snapshot may carry. Emitted exactly once,
    /// as the LAST warning, so a caller knows the list is not exhaustive.
    WarningsTruncated,
}

impl InventoryWarningCode {
    pub fn as_str(self) -> &'static str {
        match self {
            InventoryWarningCode::MissingSessionId => "missing_session_id",
            InventoryWarningCode::MalformedCorrelation => "malformed_correlation",
            InventoryWarningCode::MalformedIdentity => "malformed_identity",
            InventoryWarningCode::MissingCreatedAt => "missing_created_at",
            InventoryWarningCode::MalformedCreatedAt => "malformed_created_at",
            InventoryWarningCode::MalformedLastPending => "malformed_last_pending",
            InventoryWarningCode::ClockSkew => "clock_skew",
            InventoryWarningCode::LifetimeOverflow => "lifetime_overflow",
            InventoryWarningCode::UnknownStatus => "unknown_status",
            InventoryWarningCode::SourceTruncated => "source_truncated",
            InventoryWarningCode::WarningsTruncated => "warnings_truncated",
        }
    }
}

impl std::fmt::Display for InventoryWarningCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One warning, correlated to the runtime it concerns.
///
/// Both correlation fields are optional: a snapshot-wide warning (a truncated page
/// walk) concerns no single runtime, and an orphan has no session id to name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedInventoryWarning {
    pub code: InventoryWarningCode,
    /// The backend handle of the runtime this warning is about.
    pub runtime_id: Option<String>,
    /// The session the runtime belongs to, when it has a usable stamp.
    pub session_id: Option<String>,
}

impl BoundedInventoryWarning {
    /// A warning about the whole snapshot rather than one runtime.
    pub fn snapshot(code: InventoryWarningCode) -> Self {
        Self {
            code,
            runtime_id: None,
            session_id: None,
        }
    }
}

/// The defensive ceiling on warnings per snapshot.
///
/// Warnings are per-runtime, so a fleet-wide metadata regression could otherwise
/// produce one allocation per runtime per code. The cap keeps a snapshot's memory
/// bounded independently of the item ceiling, and overflow is announced with
/// [`InventoryWarningCode::WarningsTruncated`] rather than dropped.
pub const MAX_WARNINGS: usize = 256;

/// A bounded warning collector.
///
/// The last slot is reserved for the truncation marker, so a full sink always ends
/// with an explicit statement that it is full — a caller can never mistake a
/// clipped list for a complete one.
#[derive(Debug)]
pub struct WarningSink {
    warnings: Vec<BoundedInventoryWarning>,
    max: usize,
    truncated: bool,
}

impl Default for WarningSink {
    fn default() -> Self {
        Self::new(MAX_WARNINGS)
    }
}

impl WarningSink {
    /// A sink holding at most `max` warnings (including the truncation marker). A
    /// `max` of zero is coerced to one so the marker always fits.
    pub fn new(max: usize) -> Self {
        Self {
            warnings: Vec::new(),
            max: max.max(1),
            truncated: false,
        }
    }

    /// Record a runtime-scoped warning. Once the sink is full every further push
    /// is folded into the single truncation marker.
    pub fn push(
        &mut self,
        code: InventoryWarningCode,
        runtime_id: Option<&str>,
        session_id: Option<&str>,
    ) {
        if self.warnings.len() + 1 >= self.max {
            self.mark_truncated();
            return;
        }
        self.warnings.push(BoundedInventoryWarning {
            code,
            runtime_id: runtime_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
        });
    }

    /// Record a snapshot-wide warning.
    pub fn push_snapshot(&mut self, code: InventoryWarningCode) {
        self.push(code, None, None);
    }

    /// Append the truncation marker exactly once.
    fn mark_truncated(&mut self) {
        if self.truncated {
            return;
        }
        self.truncated = true;
        self.warnings.push(BoundedInventoryWarning::snapshot(
            InventoryWarningCode::WarningsTruncated,
        ));
        tracing::warn!(
            max = self.max,
            "runtime inventory: warning ceiling reached; further warnings folded into one marker"
        );
    }

    /// Consume the sink into its warnings, in the order they were recorded.
    pub fn into_warnings(self) -> Vec<BoundedInventoryWarning> {
        self.warnings
    }

    /// How many warnings are currently held (including the marker).
    pub fn len(&self) -> usize {
        self.warnings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }
}

#[cfg(test)]
#[path = "warning_tests.rs"]
mod tests;
