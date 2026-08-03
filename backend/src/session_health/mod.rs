//! The **v1 session health report contract**: the on-disk artifact an in-session
//! package writes, and the only thing the control plane knows about business-aware
//! session health.
//!
//! # Why this module is a contract and not a feature
//!
//! Health reports are produced by a package on the `packages` branch and consumed by
//! the control plane here. Those are two independently released streams, so the format
//! between them is the load-bearing artifact: it lands first and both sides build
//! against it.
//!
//! # The package-agnostic relay constraint
//!
//! fkst-hosted hosts arbitrary fkst packages and must NEVER encode any one package's
//! notion of "healthy" — the same constraint [`crate::k8s::health_scrape`] states for
//! the infra-layer signal. This module therefore serves **"health reports conforming
//! to schema v1"**, never "the `fkst-health` package's output":
//!
//! * [`report::HealthReport::status`] and [`report::HealthReport::headline`] are
//!   relayed **verbatim**. The control plane never second-guesses, recomputes, or
//!   overrides a producer's verdict.
//! * [`report::HealthReport::body_markdown`] is **opaque**. It is never parsed,
//!   searched, summarized, or interpreted anywhere in this crate — it is bytes that
//!   travel from a producer to a browser.
//! * [`report::HealthReport::expected_interval_secs`] is the **producer's** declared
//!   cadence. The staleness watchdog reads it from the report rather than hardcoding a
//!   package's tick, so a producer with a different rhythm is judged by its own clock.
//!
//! Any package emitting this schema is served identically. That is what keeps the
//! infra layer's package-agnosticism intact while adding a business layer on top.
//!
//! # Failure posture: one bad report is not an outage
//!
//! Every entry point here is total and lenient by design, because the producer ships
//! in the default manifest and therefore rides *every* session — a strict parser would
//! turn one producer defect into a fleet-wide outage.
//!
//! * Unknown front-matter keys are ignored, so a newer producer never breaks an older
//!   control plane.
//! * An unrecognized `status` degrades to [`report::HealthStatus::Unknown`] with the
//!   raw string preserved, rather than failing.
//! * Malformed **optional** structure (a non-array `evidence`, a junk `work_items`
//!   entry) is dropped, never fatal.
//! * Only a genuinely unusable file — no front matter, a wrong schema version, a
//!   missing identity field — yields a typed [`report::ReportParseError`], and the
//!   documented caller behaviour for that is to **skip that one file with a warning**
//!   and carry on.
//!
//! # Layout in the session
//!
//! Reports live in [`HEALTH_DIR_NAME`] under the injected `FKST_RUNTIME_ROOT`, so the
//! location is backend-neutral (OpenSandbox and Kubernetes set that root differently
//! and neither literal appears here). Filenames are generated *and* parsed by
//! [`naming`], so the two can never drift.

pub mod index;
pub mod naming;
pub mod report;

pub use index::{
    health_index_key, health_report_key, index_entry, parse_index, upsert_report, HealthIndex,
    HealthIndexEntry,
};
pub use naming::{parse_report_filename, report_filename, ReportName};
pub use report::{
    parse_report, EvidenceEntry, HealthReport, HealthStatus, ReportParseError, WorkItemProgress,
};

/// Directory, relative to `FKST_RUNTIME_ROOT`, that a producer writes reports into.
///
/// The single source of truth for every consumer: the in-pod collector discovers files
/// here, and no other module may re-declare the literal.
pub const HEALTH_DIR_NAME: &str = "health";

/// The schema version this control plane understands.
///
/// A report whose `fkst_health_report` differs is skipped rather than guessed at — see
/// [`report::ReportParseError::UnsupportedSchema`].
pub const SCHEMA_VERSION: u32 = 1;
