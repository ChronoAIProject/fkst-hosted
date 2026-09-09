//! The normalized runtime status, and the two backend mapping tables.
//!
//! The two runtimes describe their lifecycles with different vocabularies and
//! different granularity — Kubernetes has no `Paused`, OpenSandbox has no
//! `deletionTimestamp` grace window. Rather than pretend they are the same, the
//! inventory carries BOTH: this stable enum for cross-backend comparison, and the
//! backend-native string verbatim in `raw_status`.
//!
//! The mappings live here (not in the adapters) so the two tables sit side by side
//! and can be diffed by a reader deciding whether a new backend state deserves a
//! new normalized variant. Both are pure `&str` functions, which keeps this module
//! kube-free and exhaustively unit-testable.

/// The stable, backend-neutral runtime state.
///
/// [`Unknown`](RuntimeInventoryStatus::Unknown) is a first-class outcome, not an
/// error: a backend that ships a new state must remain listable, with its unmapped
/// value preserved in `raw_status`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeInventoryStatus {
    Pending,
    Running,
    Paused,
    /// Mid-transition between two steady states (OpenSandbox `Pausing`/`Resuming`).
    Transitioning,
    Succeeded,
    Failed,
    /// Deletion has been requested and the runtime is draining.
    Terminating,
    /// The runtime is gone/stopped, with NO claim about whether its work
    /// succeeded — OpenSandbox's `Terminated` says nothing about exit status.
    Terminated,
    /// The default, because "we could not tell" is the only honest state to
    /// assume before a backend has said anything.
    #[default]
    Unknown,
}

impl RuntimeInventoryStatus {
    /// Every variant, so a downstream renderer or filter can enumerate the closed
    /// set without restating it.
    pub const ALL: [RuntimeInventoryStatus; 9] = [
        RuntimeInventoryStatus::Pending,
        RuntimeInventoryStatus::Running,
        RuntimeInventoryStatus::Paused,
        RuntimeInventoryStatus::Transitioning,
        RuntimeInventoryStatus::Succeeded,
        RuntimeInventoryStatus::Failed,
        RuntimeInventoryStatus::Terminating,
        RuntimeInventoryStatus::Terminated,
        RuntimeInventoryStatus::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeInventoryStatus::Pending => "pending",
            RuntimeInventoryStatus::Running => "running",
            RuntimeInventoryStatus::Paused => "paused",
            RuntimeInventoryStatus::Transitioning => "transitioning",
            RuntimeInventoryStatus::Succeeded => "succeeded",
            RuntimeInventoryStatus::Failed => "failed",
            RuntimeInventoryStatus::Terminating => "terminating",
            RuntimeInventoryStatus::Terminated => "terminated",
            RuntimeInventoryStatus::Unknown => "unknown",
        }
    }

    /// Parse the closed wire spelling back. `None` for anything else — used by the
    /// public filter layer, which must reject an unrecognized value rather than
    /// silently widening a query.
    pub fn parse(value: &str) -> Option<Self> {
        RuntimeInventoryStatus::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    /// The Kubernetes mapping.
    ///
    /// `deletionTimestamp` WINS over the phase: a Pod being drained still reports
    /// `Running`, and an operations view that showed it as healthy would hide the
    /// most operationally interesting fact about it. Everything else follows
    /// `status.phase`; an absent phase (the object exists but the kubelet has not
    /// reported yet), the literal `Unknown` phase, and any future value all map to
    /// [`RuntimeInventoryStatus::Unknown`] rather than being guessed at.
    pub fn from_kubernetes(phase: Option<&str>, terminating: bool) -> Self {
        if terminating {
            return RuntimeInventoryStatus::Terminating;
        }
        match phase {
            Some("Pending") => RuntimeInventoryStatus::Pending,
            Some("Running") => RuntimeInventoryStatus::Running,
            Some("Succeeded") => RuntimeInventoryStatus::Succeeded,
            Some("Failed") => RuntimeInventoryStatus::Failed,
            _ => RuntimeInventoryStatus::Unknown,
        }
    }

    /// The OpenSandbox mapping.
    ///
    /// `Terminated` maps to [`RuntimeInventoryStatus::Terminated`] and NEVER to
    /// `Succeeded`: the lifecycle API reports that a sandbox stopped existing, not
    /// that its work completed successfully, and inventing a success verdict would
    /// be the most damaging kind of wrong in an audit surface. `Stopping` is
    /// `Terminating` (deletion in progress), while `Pausing`/`Resuming` are
    /// `Transitioning` (the runtime is staying).
    pub fn from_opensandbox(state: &str) -> Self {
        match state {
            "Pending" => RuntimeInventoryStatus::Pending,
            "Running" => RuntimeInventoryStatus::Running,
            "Paused" => RuntimeInventoryStatus::Paused,
            "Pausing" | "Resuming" => RuntimeInventoryStatus::Transitioning,
            "Stopping" => RuntimeInventoryStatus::Terminating,
            "Terminated" => RuntimeInventoryStatus::Terminated,
            "Failed" => RuntimeInventoryStatus::Failed,
            _ => RuntimeInventoryStatus::Unknown,
        }
    }
}

impl std::fmt::Display for RuntimeInventoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
