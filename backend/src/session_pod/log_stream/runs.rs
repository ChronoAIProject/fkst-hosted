//! The per-RUN (per-pod-incarnation) log-bundle model + the run-index transforms.
//!
//! A substrate SESSION (one trigger issue) is served by a SEQUENCE of pods over its
//! lifetime (idle-reap → auto-revive); each pod incarnation is ONE run, identified by
//! the collector's existing instance id. The whole-session `latest.tar.gz` is
//! overwritten by every pod, so only the newest run survives there. Per-run separation
//! is ADDITIVE: each run ALSO uploads its own immutable object at
//! [`run_bundle_key`], and a single per-session index object ([`runs_index_key`])
//! enumerates every run — chrono-storage has no list-by-prefix API, so the index (one
//! object, read-modify-written by the sole live pod) is how runs are discovered.
//!
//! This module is PURE + exhaustively unit-tested: the key builders and the three
//! index transforms ([`upsert_run`] / [`finalize_run`] / [`parse_runs`]) are the
//! stable machinery both the in-pod collector and the download route depend on. Every
//! transform is lenient — malformed / absent index bytes degrade to an empty list
//! rather than ever failing — because the collector is best-effort and MUST NOT crash
//! the session.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

/// The chrono-storage object key a single RUN's immutable bundle uploads to:
/// `logs/{session_id}/runs/{run_id}.tar.gz`. One object per pod incarnation, so a
/// revived pod never clobbers a prior run's logs (unlike the shared `latest.tar.gz`).
pub fn run_bundle_key(session_id: &str, run_id: &str) -> String {
    format!("logs/{session_id}/runs/{run_id}.tar.gz")
}

/// The chrono-storage object key of a session's run INDEX: `logs/{session_id}/runs.json`.
/// A single object listing every run — the only way to enumerate runs, since
/// chrono-storage exposes no list-by-prefix endpoint.
pub fn runs_index_key(session_id: &str) -> String {
    format!("logs/{session_id}/runs.json")
}

/// One RUN (== one pod incarnation) in a session's run index. Non-secret metadata
/// only; timestamps are RFC3339 UTC strings. Deriving `ToSchema` here is fine — the
/// backend is ONE crate, so the download route can reference it directly in the spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct LogRun {
    /// The run id (== the collector instance id: `<UTC-basic-stamp>Z-<pod8>`).
    pub run_id: String,
    /// When the run's pod started (RFC3339, UTC). MAY be empty for the legacy
    /// synthetic `latest` run the `/runs` endpoint fabricates for a pre-#568
    /// bundle (no run index exists, so the original start time is unknown) — an
    /// empty value there is a documented contract, not a violation.
    pub started_at: String,
    /// When the run ended (RFC3339, UTC); absent while the run is still live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

/// Render a UTC instant as the RFC3339 string form used for every run timestamp.
pub fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Leniently parse the run-index bytes into a run list: malformed / non-array / empty
/// bytes all degrade to an empty `Vec` (the best-effort contract — a corrupt index is
/// never a hard failure). Used by the download route AND by the read-modify-write
/// transforms below.
pub fn parse_runs(bytes: &[u8]) -> Vec<LogRun> {
    serde_json::from_slice(bytes).unwrap_or_default()
}

/// Parse the OPTIONAL existing index bytes (`None`/garbage → empty list).
fn parse_existing(existing: Option<&[u8]>) -> Vec<LogRun> {
    existing.map(parse_runs).unwrap_or_default()
}

/// Serialize a run list as the index document (pretty + trailing newline). A Vec of
/// plain structs cannot fail to serialize; the defensive fallback keeps this
/// infallible for the best-effort collector.
fn serialize_index(runs: &[LogRun]) -> String {
    match serde_json::to_string_pretty(runs) {
        Ok(mut json) => {
            json.push('\n');
            json
        }
        Err(_) => "[]\n".to_string(),
    }
}

/// Add `run` to the index (parsing the optional existing bytes first), appending it
/// only when no entry already carries its `run_id` — so re-running against an index
/// that already lists the run is a no-op (IDEMPOTENT). Returns the serialized index.
pub fn upsert_run(existing: Option<&[u8]>, run: &LogRun) -> String {
    let mut runs = parse_existing(existing);
    if !runs.iter().any(|r| r.run_id == run.run_id) {
        runs.push(run.clone());
    }
    serialize_index(&runs)
}

/// UPSERT-then-stamp `run` (which carries the run identity + its `ended_at`) into
/// the index: when an entry already lists its `run_id`, stamp that entry's end time
/// while PRESERVING its recorded `started_at`; otherwise push `run` as-is. The
/// upsert is the shutdown RECOVERY path — a run whose bundle uploaded fine but whose
/// one-shot `upsert_run` was lost to a transient index error is re-added here rather
/// than staying permanently absent. Returns the serialized index.
pub fn finalize_run(existing: Option<&[u8]>, run: &LogRun) -> String {
    let mut runs = parse_existing(existing);
    match runs.iter_mut().find(|r| r.run_id == run.run_id) {
        // Known run: stamp its end time, keeping the start time it was added with.
        Some(existing) => existing.ended_at = run.ended_at.clone(),
        // Unknown run (a lost add): recover it in full so shutdown never drops a
        // run whose bundle already uploaded.
        None => runs.push(run.clone()),
    }
    serialize_index(&runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, started: &str, ended: Option<&str>) -> LogRun {
        LogRun {
            run_id: id.to_string(),
            started_at: started.to_string(),
            ended_at: ended.map(|s| s.to_string()),
        }
    }

    #[test]
    fn keys_have_the_stable_per_session_layout() {
        assert_eq!(
            run_bundle_key("sess-1", "20260720T101010Z-abcd1234"),
            "logs/sess-1/runs/20260720T101010Z-abcd1234.tar.gz"
        );
        assert_eq!(runs_index_key("sess-1"), "logs/sess-1/runs.json");
    }

    #[test]
    fn upsert_into_empty_index_yields_one_run() {
        let out = upsert_run(None, &run("r1", "2026-07-20T10:00:00Z", None));
        let runs = parse_runs(out.as_bytes());
        assert_eq!(runs, vec![run("r1", "2026-07-20T10:00:00Z", None)]);
        // Pretty-printed with a trailing newline.
        assert!(out.ends_with("\n"));
        assert!(out.contains("  \"run_id\": \"r1\""), "pretty: {out}");
        // A live run omits `ended_at` entirely (skip_serializing_if).
        assert!(!out.contains("ended_at"), "live run has no ended_at: {out}");
    }

    #[test]
    fn upsert_is_idempotent_on_the_same_run_id() {
        let first = upsert_run(None, &run("r1", "t1", None));
        // Re-upsert the SAME run id (even with different fields) does not duplicate.
        let second = upsert_run(Some(first.as_bytes()), &run("r1", "t1", None));
        assert_eq!(parse_runs(second.as_bytes()).len(), 1);
        // A DIFFERENT run id appends.
        let third = upsert_run(Some(second.as_bytes()), &run("r2", "t2", None));
        let runs = parse_runs(third.as_bytes());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "r1");
        assert_eq!(runs[1].run_id, "r2");
    }

    #[test]
    fn finalize_stamps_an_existing_run_preserving_its_start_time() {
        let idx = upsert_run(None, &run("r1", "2026-07-20T10:00:00Z", None));
        let out = finalize_run(
            Some(idx.as_bytes()),
            &run("r1", "IGNORED-START", Some("2026-07-20T11:00:00Z")),
        );
        let runs = parse_runs(out.as_bytes());
        assert_eq!(runs.len(), 1, "no duplicate is added for a known run");
        assert_eq!(runs[0].ended_at.as_deref(), Some("2026-07-20T11:00:00Z"));
        // The existing entry's start time is preserved, NOT overwritten.
        assert_eq!(runs[0].started_at, "2026-07-20T10:00:00Z");
    }

    #[test]
    fn finalize_recovers_a_missing_run_with_its_start_and_end() {
        // The one-shot add was lost (index missing this run); finalize UPSERTS it
        // in full so a run whose bundle uploaded is never permanently absent.
        let idx = upsert_run(None, &run("r1", "t1", None));
        let out = finalize_run(
            Some(idx.as_bytes()),
            &run("r2", "2026-07-20T12:00:00Z", Some("2026-07-20T12:30:00Z")),
        );
        let runs = parse_runs(out.as_bytes());
        assert_eq!(runs.len(), 2, "the missing run was added, not dropped");
        let recovered = runs.iter().find(|r| r.run_id == "r2").expect("r2 added");
        assert_eq!(recovered.started_at, "2026-07-20T12:00:00Z");
        assert_eq!(recovered.ended_at.as_deref(), Some("2026-07-20T12:30:00Z"));
    }

    #[test]
    fn finalize_on_empty_index_adds_the_run() {
        // Nothing to stamp → the recovery path adds the run outright.
        let out = finalize_run(None, &run("r1", "2026-07-20T09:00:00Z", Some("t-end")));
        let runs = parse_runs(out.as_bytes());
        assert_eq!(runs, vec![run("r1", "2026-07-20T09:00:00Z", Some("t-end"))]);
    }

    #[test]
    fn parse_tolerates_garbage_and_absence() {
        assert!(parse_runs(b"").is_empty());
        assert!(parse_runs(b"not json at all").is_empty());
        // Valid JSON that is not an array of runs also degrades to empty.
        assert!(parse_runs(b"{\"unexpected\": true}").is_empty());
        // A well-formed index round-trips.
        let idx = upsert_run(None, &run("r1", "t1", Some("t-end")));
        let runs = parse_runs(idx.as_bytes());
        assert_eq!(runs, vec![run("r1", "t1", Some("t-end"))]);
    }

    #[test]
    fn rfc3339_renders_seconds_precision_utc() {
        let dt = DateTime::parse_from_rfc3339("2026-07-20T10:00:00.123456Z")
            .expect("parse")
            .with_timezone(&Utc);
        assert_eq!(rfc3339(dt), "2026-07-20T10:00:00Z");
    }
}
