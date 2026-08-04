//! Bounded telemetry for the sandbox inventory (`fkst_operations_sandbox_*`).
//!
//! Every label is a closed Rust enum, so the series count is decided at compile
//! time: backends × scopes × results, backends × results, backends × scopes, and
//! rejection reasons. No viewer, actor, session, runtime, repository, or filter
//! value is ever a label OR a value here (epic `OPS-04`).
//!
//! ## The item gauge is an aggregate, never a per-requester series
//!
//! `fkst_operations_sandbox_inventory_items{backend,scope}` records the size of
//! the last AUTHORIZED result under each closed scope category. It is deliberately
//! not keyed by viewer: a per-requester series would be both unbounded and a
//! standing disclosure of who looked at what, and a scrape endpoint is a much
//! weaker access boundary than the API it describes.
//!
//! ## `backend="none"` is a real state, not a missing label
//!
//! A deployment with no runtime backend still serves this route — and answers
//! `503 sandbox_inventory_disabled`. Recording that under a third closed backend
//! value keeps the label set complete without inventing a fake backend.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::runtime_identity::RuntimeBackendKind;

/// The runtime backend a request was served against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendLabel {
    Kubernetes,
    OpenSandbox,
    /// No runtime backend is configured in this deployment.
    None,
}

impl BackendLabel {
    pub const COUNT: usize = 3;
    pub const ALL: [BackendLabel; Self::COUNT] = [
        BackendLabel::Kubernetes,
        BackendLabel::OpenSandbox,
        BackendLabel::None,
    ];

    /// The label for a configured backend, or [`BackendLabel::None`].
    pub fn of(backend: Option<RuntimeBackendKind>) -> Self {
        match backend {
            Some(RuntimeBackendKind::Kubernetes) => BackendLabel::Kubernetes,
            Some(RuntimeBackendKind::OpenSandbox) => BackendLabel::OpenSandbox,
            None => BackendLabel::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BackendLabel::Kubernetes => "kubernetes",
            BackendLabel::OpenSandbox => "opensandbox",
            BackendLabel::None => "none",
        }
    }

    fn index(self) -> usize {
        match self {
            BackendLabel::Kubernetes => 0,
            BackendLabel::OpenSandbox => 1,
            BackendLabel::None => 2,
        }
    }
}

/// The effective scope a request ran under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeLabel {
    Accessible,
    All,
}

impl ScopeLabel {
    pub const COUNT: usize = 2;
    pub const ALL: [ScopeLabel; Self::COUNT] = [ScopeLabel::Accessible, ScopeLabel::All];

    pub fn as_str(self) -> &'static str {
        match self {
            ScopeLabel::Accessible => "accessible",
            ScopeLabel::All => "all",
        }
    }

    fn index(self) -> usize {
        match self {
            ScopeLabel::Accessible => 0,
            ScopeLabel::All => 1,
        }
    }
}

/// The terminal result of one inventory request. Mirrors the documented status
/// set exactly, so a counter and an HTTP response can never disagree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryResult {
    Success,
    InvalidRequest,
    Forbidden,
    NotFound,
    VisibilityUnavailable,
    Disabled,
    Unavailable,
    TooLarge,
}

impl InventoryResult {
    pub const COUNT: usize = 8;
    pub const ALL: [InventoryResult; Self::COUNT] = [
        InventoryResult::Success,
        InventoryResult::InvalidRequest,
        InventoryResult::Forbidden,
        InventoryResult::NotFound,
        InventoryResult::VisibilityUnavailable,
        InventoryResult::Disabled,
        InventoryResult::Unavailable,
        InventoryResult::TooLarge,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            InventoryResult::Success => "success",
            InventoryResult::InvalidRequest => "invalid_request",
            InventoryResult::Forbidden => "forbidden",
            InventoryResult::NotFound => "not_found",
            InventoryResult::VisibilityUnavailable => "visibility_unavailable",
            InventoryResult::Disabled => "disabled",
            InventoryResult::Unavailable => "unavailable",
            InventoryResult::TooLarge => "too_large",
        }
    }

    fn index(self) -> usize {
        match self {
            InventoryResult::Success => 0,
            InventoryResult::InvalidRequest => 1,
            InventoryResult::Forbidden => 2,
            InventoryResult::NotFound => 3,
            InventoryResult::VisibilityUnavailable => 4,
            InventoryResult::Disabled => 5,
            InventoryResult::Unavailable => 6,
            InventoryResult::TooLarge => 7,
        }
    }
}

/// Why a request was refused before the runtime backend was touched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxRejectionReason {
    /// A regular caller selected the global scope.
    GlobalScope,
    /// An exact session id was unknown OR unauthorized — one reason for both, so
    /// the counter cannot become the existence oracle the response refuses to be.
    SessionNotFound,
    /// The session-visibility projection was cold or incomplete.
    VisibilityUnavailable,
    /// A filter value failed its closed-vocabulary check.
    InvalidFilter,
}

impl SandboxRejectionReason {
    pub const COUNT: usize = 4;
    pub const ALL: [SandboxRejectionReason; Self::COUNT] = [
        SandboxRejectionReason::GlobalScope,
        SandboxRejectionReason::SessionNotFound,
        SandboxRejectionReason::VisibilityUnavailable,
        SandboxRejectionReason::InvalidFilter,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SandboxRejectionReason::GlobalScope => "global_scope_forbidden",
            SandboxRejectionReason::SessionNotFound => "session_not_found",
            SandboxRejectionReason::VisibilityUnavailable => "session_visibility_unavailable",
            SandboxRejectionReason::InvalidFilter => "invalid_filter",
        }
    }

    fn index(self) -> usize {
        match self {
            SandboxRejectionReason::GlobalScope => 0,
            SandboxRejectionReason::SessionNotFound => 1,
            SandboxRejectionReason::VisibilityUnavailable => 2,
            SandboxRejectionReason::InvalidFilter => 3,
        }
    }
}

const REQUEST_SERIES: usize = BackendLabel::COUNT * ScopeLabel::COUNT * InventoryResult::COUNT;
const DURATION_SERIES: usize = BackendLabel::COUNT * InventoryResult::COUNT;
const ITEM_SERIES: usize = BackendLabel::COUNT * ScopeLabel::COUNT;

/// Process-local inventory counters. Cheap to clone; every clone shares one
/// backing store.
#[derive(Clone)]
pub struct SandboxMetrics {
    requests: Arc<[AtomicU64; REQUEST_SERIES]>,
    duration_sum: Arc<[AtomicU64; DURATION_SERIES]>,
    duration_count: Arc<[AtomicU64; DURATION_SERIES]>,
    items: Arc<[AtomicU64; ITEM_SERIES]>,
    rejections: Arc<[AtomicU64; SandboxRejectionReason::COUNT]>,
}

/// `[AtomicU64; N]` has no blanket `Default` (std stops at 32 elements and the
/// request family has 48 series), so the arrays are built explicitly.
fn zeroed<const N: usize>() -> Arc<[AtomicU64; N]> {
    Arc::new(std::array::from_fn(|_| AtomicU64::new(0)))
}

impl Default for SandboxMetrics {
    fn default() -> Self {
        Self {
            requests: zeroed(),
            duration_sum: zeroed(),
            duration_count: zeroed(),
            items: zeroed(),
            rejections: zeroed(),
        }
    }
}

impl std::fmt::Debug for SandboxMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxMetrics")
            .field("request_series", &REQUEST_SERIES)
            .finish()
    }
}

fn request_index(backend: BackendLabel, scope: ScopeLabel, result: InventoryResult) -> usize {
    (backend.index() * ScopeLabel::COUNT + scope.index()) * InventoryResult::COUNT + result.index()
}

fn duration_index(backend: BackendLabel, result: InventoryResult) -> usize {
    backend.index() * InventoryResult::COUNT + result.index()
}

fn item_index(backend: BackendLabel, scope: ScopeLabel) -> usize {
    backend.index() * ScopeLabel::COUNT + scope.index()
}

impl SandboxMetrics {
    /// Fresh counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one terminal request and observe its duration.
    pub fn record_request(
        &self,
        backend: BackendLabel,
        scope: ScopeLabel,
        result: InventoryResult,
        duration_millis: u64,
    ) {
        self.requests[request_index(backend, scope, result)].fetch_add(1, Ordering::Relaxed);
        let index = duration_index(backend, result);
        self.duration_sum[index].fetch_add(duration_millis, Ordering::Relaxed);
        self.duration_count[index].fetch_add(1, Ordering::Relaxed);
    }

    /// Publish the size of one AUTHORIZED result under its closed scope category.
    pub fn record_items(&self, backend: BackendLabel, scope: ScopeLabel, items: u64) {
        self.items[item_index(backend, scope)].store(items, Ordering::Relaxed);
    }

    /// Count one refusal that happened before the runtime backend was touched.
    pub fn record_rejection(&self, reason: SandboxRejectionReason) {
        self.rejections[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// A consistent read projection for `/metrics`.
    pub fn snapshot(&self) -> SandboxMetricsSnapshot {
        let load = |slots: &[AtomicU64]| -> Vec<u64> {
            slots
                .iter()
                .map(|slot| slot.load(Ordering::Relaxed))
                .collect()
        };
        SandboxMetricsSnapshot {
            requests: load(&*self.requests),
            duration_sum: load(&*self.duration_sum),
            duration_count: load(&*self.duration_count),
            items: load(&*self.items),
            rejections: load(&*self.rejections),
        }
    }
}

/// An immutable copy of the inventory counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SandboxMetricsSnapshot {
    requests: Vec<u64>,
    duration_sum: Vec<u64>,
    duration_count: Vec<u64>,
    items: Vec<u64>,
    rejections: Vec<u64>,
}

impl SandboxMetricsSnapshot {
    /// Every `(backend, scope, result)` series, in exposition order.
    pub fn requests(
        &self,
    ) -> impl Iterator<Item = (&'static str, &'static str, &'static str, u64)> + '_ {
        BackendLabel::ALL.into_iter().flat_map(move |backend| {
            ScopeLabel::ALL.into_iter().flat_map(move |scope| {
                InventoryResult::ALL.into_iter().map(move |result| {
                    (
                        backend.as_str(),
                        scope.as_str(),
                        result.as_str(),
                        self.requests
                            .get(request_index(backend, scope, result))
                            .copied()
                            .unwrap_or_default(),
                    )
                })
            })
        })
    }

    /// Every `(backend, result)` duration summary, in exposition order.
    pub fn durations(&self) -> impl Iterator<Item = (&'static str, &'static str, u64, u64)> + '_ {
        BackendLabel::ALL.into_iter().flat_map(move |backend| {
            InventoryResult::ALL.into_iter().map(move |result| {
                let index = duration_index(backend, result);
                (
                    backend.as_str(),
                    result.as_str(),
                    self.duration_sum.get(index).copied().unwrap_or_default(),
                    self.duration_count.get(index).copied().unwrap_or_default(),
                )
            })
        })
    }

    /// Every `(backend, scope)` item gauge, in exposition order.
    pub fn items(&self) -> impl Iterator<Item = (&'static str, &'static str, u64)> + '_ {
        BackendLabel::ALL.into_iter().flat_map(move |backend| {
            ScopeLabel::ALL.into_iter().map(move |scope| {
                (
                    backend.as_str(),
                    scope.as_str(),
                    self.items
                        .get(item_index(backend, scope))
                        .copied()
                        .unwrap_or_default(),
                )
            })
        })
    }

    /// Rejection counts by bounded reason, in exposition order.
    pub fn rejections(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        SandboxRejectionReason::ALL.into_iter().map(move |reason| {
            (
                reason.as_str(),
                self.rejections
                    .get(reason.index())
                    .copied()
                    .unwrap_or_default(),
            )
        })
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
