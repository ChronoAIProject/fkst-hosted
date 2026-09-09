//! The v1 report document: markdown with **TOML front matter**, and its parser.
//!
//! # Why TOML and not YAML
//!
//! `toml = "0.8"` is already a dependency, TOML is this repository's native
//! configuration language (`fkst.toml`, the codex `config.toml`, `Cargo.toml`), and
//! the obvious YAML crate — `serde_yaml` — was deprecated and archived upstream in
//! 2024. Adding an unmaintained dependency for a flat key/value block would be the
//! wrong trade in both directions.
//!
//! # Strictness policy
//!
//! Deliberately asymmetric, because the producer rides every session and a strict
//! parser turns one producer defect into a fleet-wide outage:
//!
//! * **Required identity fields are strict** (`session_id`, `producer`,
//!   `generated_at`, `headline`, and the presence of `status`). Without them the file
//!   cannot become a usable index entry, so it is skipped and the next tick recovers.
//! * **Everything optional is lenient.** A non-array `evidence`, a junk `work_items`
//!   entry, a `status` string nobody recognizes, an unparseable
//!   `expected_interval_secs` — each degrades locally and never sinks the document.
//!
//! # TOML ordering requirement
//!
//! TOML requires scalar keys to precede any table or array-of-tables, so a producer
//! must render `[[evidence]]` / `[[work_items]]` **last**. The parser does not care,
//! but a hand-written example that violates it is not valid TOML and will not parse.

use k8s_openapi::chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use super::SCHEMA_VERSION;

/// Delimiter line opening and closing the front-matter block.
pub const FRONT_MATTER_FENCE: &str = "+++";

/// Cadence assumed when a producer does not declare `expected_interval_secs`.
///
/// Matches the `fkst-health` package's 10-minute cron raiser, but is only a fallback:
/// a producer that declares its own value is judged by that value instead.
pub const DEFAULT_EXPECTED_INTERVAL_SECS: u64 = 600;

/// Maximum `headline` length in characters. Longer headlines are truncated with an
/// ellipsis rather than rejected — a verbose headline is a cosmetic problem, and
/// dropping the report over it would cost a heartbeat.
pub const HEADLINE_MAX_CHARS: usize = 200;

/// Maximum number of `evidence` entries retained.
pub const EVIDENCE_MAX_ENTRIES: usize = 32;
/// Maximum `evidence.key` length in characters.
pub const EVIDENCE_KEY_MAX_CHARS: usize = 64;
/// Maximum `evidence.value` length in characters.
pub const EVIDENCE_VALUE_MAX_CHARS: usize = 256;
/// Maximum number of `work_items` entries retained.
pub const WORK_ITEMS_MAX_ENTRIES: usize = 64;
/// Maximum length of a `work_items` `state` / `progress` string.
///
/// Not in the published field table, but these cross the same trust boundary as
/// `evidence` and are bounded on the same principle: nothing a session writes may be
/// unbounded by the time it reaches an API response.
pub const WORK_ITEM_FIELD_MAX_CHARS: usize = 64;
/// Maximum `confidence` length in characters. Bounded for the same reason.
pub const CONFIDENCE_MAX_CHARS: usize = 32;

/// The producer's verdict for the observed window.
///
/// Relayed verbatim by the control plane, which never derives, overrides, or
/// second-guesses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Measurable progress in the window.
    Working,
    /// The pod is alive but has no open work items.
    ///
    /// Deliberately **rare**: a session with no pending work is reaped after
    /// `session_idle_grace_secs`, so a tick can only observe this inside that grace
    /// window. The normal "nothing to do" case is represented control-plane side by
    /// the absence of a live runtime, not by a report carrying this status.
    Idle,
    /// Producing output, but repeatedly failing on the same obstacle.
    Blocked,
    /// Work pending, no progress, no new output.
    Stalled,
    /// The framework or engine is erroring out.
    Failing,
    /// Insufficient evidence to judge — also what an unrecognized status maps to.
    Unknown,
}

impl HealthStatus {
    /// Map a producer's raw status string onto the taxonomy.
    ///
    /// Case-insensitive and whitespace-tolerant. Anything unrecognized becomes
    /// [`HealthStatus::Unknown`]; the caller keeps the raw string for display so a
    /// newer producer's vocabulary is still visible to a user.
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "working" => Self::Working,
            "idle" => Self::Idle,
            "blocked" => Self::Blocked,
            "stalled" => Self::Stalled,
            "failing" => Self::Failing,
            _ => Self::Unknown,
        }
    }
}

/// One `key = value` observation the producer grounded its verdict in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct EvidenceEntry {
    /// Short machine-ish name of the observation, e.g. `deliveries_completed_delta`.
    pub key: String,
    /// The observed value, always rendered as a string.
    pub value: String,
}

/// A work item's state as the producer observed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkItemProgress {
    /// The GitHub issue number.
    pub number: i64,
    /// Issue state as the producer saw it, e.g. `open`.
    pub state: String,
    /// Producer-defined progress descriptor, e.g. `none`.
    pub progress: String,
}

/// A parsed v1 health report: the machine-readable verdict plus the opaque body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct HealthReport {
    /// Always [`SCHEMA_VERSION`] — a report with any other value is never parsed.
    pub schema_version: u32,
    /// The deterministic session id the producer stamped in.
    pub session_id: String,
    /// The fkst **work-label** namespace, when the session has one.
    ///
    /// Display and provenance only. Absent means the session's labels are
    /// unnamespaced — never a placeholder.
    pub namespace: Option<String>,
    /// `<name>@<version>` of the producing package, rendered so a user can see which
    /// package spoke.
    pub producer: String,
    /// When the producer generated the report.
    #[schema(value_type = String, example = "2026-07-30T14:15:00Z")]
    pub generated_at: DateTime<Utc>,
    /// Start of the window the producer observed, when it declared one.
    #[schema(value_type = String, example = "2026-07-30T14:05:00Z")]
    pub window_start: Option<DateTime<Utc>>,
    /// The producer's **own** declared cadence, in seconds.
    ///
    /// The staleness watchdog reads this from the report rather than hardcoding a
    /// package's tick. A declared `0` is nonsensical and falls back to
    /// [`DEFAULT_EXPECTED_INTERVAL_SECS`]; there is deliberately **no upper clamp**,
    /// because an absurdly large value only suppresses a producer's own alarm, and
    /// suppressing an alarm is the fail-open direction.
    pub expected_interval_secs: u64,
    /// The verdict, mapped onto the taxonomy.
    pub status: HealthStatus,
    /// The verdict exactly as the producer wrote it, preserved even when it did not
    /// map onto a known variant.
    pub status_raw: String,
    /// One-line human summary, truncated to [`HEADLINE_MAX_CHARS`].
    pub headline: String,
    /// Producer-declared confidence, conventionally `high` / `medium` / `low`.
    pub confidence: Option<String>,
    /// Observations backing the verdict, capped at [`EVIDENCE_MAX_ENTRIES`].
    pub evidence: Vec<EvidenceEntry>,
    /// Work items the producer observed, capped at [`WORK_ITEMS_MAX_ENTRIES`].
    pub work_items: Vec<WorkItemProgress>,
    /// The producer's narrative, verbatim.
    ///
    /// **Opaque.** Never parsed, searched, or interpreted anywhere in this crate, and
    /// untrusted input at the rendering boundary — it is authored by an LLM inside a
    /// session pod.
    pub body_markdown: String,
}

/// Why one report file could not be turned into a [`HealthReport`].
///
/// The documented caller behaviour for every variant is the same: log it and **skip
/// that one file**. One malformed report must never break a listing.
///
/// > **Redaction:** [`ReportParseError::FrontMatterSyntax`] may quote the offending
/// > input, which originates inside a session pod and can therefore contain a
/// > credential. Callers MUST pass it through the log redactor before logging it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReportParseError {
    /// The file does not open with a `+++` fence.
    #[error("report has no TOML front matter")]
    MissingFrontMatter,
    /// The opening fence is never closed.
    #[error("report front matter is not terminated")]
    UnterminatedFrontMatter,
    /// The front matter is not valid TOML.
    #[error("report front matter is not valid TOML: {0}")]
    FrontMatterSyntax(String),
    /// `fkst_health_report` is absent, non-numeric, or a version this build does not
    /// understand.
    #[error("unsupported report schema version: {found:?} (expected {expected})")]
    UnsupportedSchema {
        /// The declared version, when one could be read as an integer at all.
        found: Option<u32>,
        /// The version this build understands.
        expected: u32,
    },
    /// A required field is absent or empty.
    #[error("report is missing required field `{0}`")]
    MissingField(&'static str),
    /// A timestamp field is present but not RFC3339.
    #[error("report field `{field}` is not an RFC3339 timestamp")]
    InvalidTimestamp {
        /// Which field failed.
        field: &'static str,
    },
}

/// Parse one report file into its verdict plus its opaque body.
///
/// See the module docs for the strictness policy; see [`ReportParseError`] for the
/// caller contract on failure (skip this file, keep going).
pub fn parse_report(text: &str) -> Result<HealthReport, ReportParseError> {
    let (front_matter, body) = split_front_matter(text)?;
    let raw: RawFrontMatter = toml::from_str(front_matter)
        .map_err(|err| ReportParseError::FrontMatterSyntax(err.to_string()))?;

    let declared = raw.fkst_health_report.as_ref().and_then(coerce_u32);
    if declared != Some(SCHEMA_VERSION) {
        return Err(ReportParseError::UnsupportedSchema {
            found: declared,
            expected: SCHEMA_VERSION,
        });
    }

    let session_id = required(raw.session_id, "session_id")?;
    let producer = required(raw.producer, "producer")?;
    let generated_at_raw = required(raw.generated_at, "generated_at")?;
    let headline_raw = required(raw.headline, "headline")?;

    // `status` must be PRESENT, but its value is lenient: an unrecognized verdict is
    // a forward-compatibility case, not a malformed file.
    let status_raw = raw
        .status
        .ok_or(ReportParseError::MissingField("status"))?
        .trim()
        .to_string();

    Ok(HealthReport {
        schema_version: SCHEMA_VERSION,
        session_id,
        namespace: optional_text(raw.namespace, EVIDENCE_KEY_MAX_CHARS),
        producer,
        generated_at: parse_timestamp(&generated_at_raw, "generated_at")?,
        window_start: match raw.window_start {
            Some(value) if !value.trim().is_empty() => {
                Some(parse_timestamp(value.trim(), "window_start")?)
            }
            _ => None,
        },
        expected_interval_secs: raw
            .expected_interval_secs
            .as_ref()
            .and_then(coerce_u64)
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_EXPECTED_INTERVAL_SECS),
        status: HealthStatus::from_raw(&status_raw),
        status_raw,
        headline: truncate_chars(&headline_raw, HEADLINE_MAX_CHARS),
        confidence: optional_text(raw.confidence, CONFIDENCE_MAX_CHARS),
        evidence: evidence_entries(raw.evidence),
        work_items: work_item_entries(raw.work_items),
        body_markdown: body.to_string(),
    })
}

/// Split `+++`-fenced front matter from the body.
///
/// The body is everything after the line terminating the **first** closing fence, kept
/// byte-for-byte — a body containing further `+++` or `---` lines is unaffected,
/// because the scan stops at the first close and never looks again.
fn split_front_matter(text: &str) -> Result<(&str, &str), ReportParseError> {
    // A UTF-8 BOM is invisible to the author and would otherwise make the opening
    // fence unrecognizable.
    let src = text.strip_prefix('\u{feff}').unwrap_or(text);

    let opening = line_len(src);
    if !is_fence(&src[..opening]) {
        return Err(ReportParseError::MissingFrontMatter);
    }

    let mut cursor = opening;
    while cursor < src.len() {
        let len = line_len(&src[cursor..]);
        if is_fence(&src[cursor..cursor + len]) {
            return Ok((&src[opening..cursor], &src[cursor + len..]));
        }
        cursor += len;
    }
    Err(ReportParseError::UnterminatedFrontMatter)
}

/// Byte length of the first line of `rest`, **including** its trailing newline.
fn line_len(rest: &str) -> usize {
    match rest.find('\n') {
        Some(index) => index + 1,
        None => rest.len(),
    }
}

/// Is this line a fence? Trailing whitespace (including `\r\n`) is tolerated; leading
/// whitespace is not, so an indented `+++` inside a code block cannot close the block.
fn is_fence(line: &str) -> bool {
    line.trim_end() == FRONT_MATTER_FENCE
}

fn required(value: Option<String>, field: &'static str) -> Result<String, ReportParseError> {
    let value = value.ok_or(ReportParseError::MissingField(field))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ReportParseError::MissingField(field));
    }
    Ok(trimmed.to_string())
}

fn parse_timestamp(value: &str, field: &'static str) -> Result<DateTime<Utc>, ReportParseError> {
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| ReportParseError::InvalidTimestamp { field })
}

/// Trim, drop when empty, and bound the length of an optional display string.
fn optional_text(value: Option<String>, max_chars: usize) -> Option<String> {
    let value = truncate_chars(value.as_deref().unwrap_or_default(), max_chars);
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Trim and bound to `max_chars` **characters** (not bytes), marking a cut with an
/// ellipsis so a reader can see the value was shortened.
fn truncate_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut out: String = trimmed.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

fn coerce_u32(value: &toml::Value) -> Option<u32> {
    coerce_u64(value).and_then(|v| u32::try_from(v).ok())
}

/// Read an unsigned integer from a TOML scalar, tolerating a stringified number —
/// leniency that keeps a producer's quoting mistake from costing a whole report.
fn coerce_u64(value: &toml::Value) -> Option<u64> {
    match value {
        toml::Value::Integer(v) => u64::try_from(*v).ok(),
        toml::Value::String(v) => v.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// Render a TOML scalar as a string. Arrays and tables have no scalar rendering and
/// yield `None`, which drops the entry that contained them.
fn scalar_string(value: Option<&toml::Value>) -> Option<String> {
    match value? {
        toml::Value::String(v) => Some(v.clone()),
        toml::Value::Integer(v) => Some(v.to_string()),
        toml::Value::Float(v) => Some(v.to_string()),
        toml::Value::Boolean(v) => Some(v.to_string()),
        toml::Value::Datetime(v) => Some(v.to_string()),
        toml::Value::Array(_) | toml::Value::Table(_) => None,
    }
}

fn evidence_entries(value: Option<toml::Value>) -> Vec<EvidenceEntry> {
    let Some(toml::Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let table = item.as_table()?;
            let key = truncate_chars(&scalar_string(table.get("key"))?, EVIDENCE_KEY_MAX_CHARS);
            if key.is_empty() {
                return None;
            }
            Some(EvidenceEntry {
                key,
                value: truncate_chars(
                    &scalar_string(table.get("value")).unwrap_or_default(),
                    EVIDENCE_VALUE_MAX_CHARS,
                ),
            })
        })
        .take(EVIDENCE_MAX_ENTRIES)
        .collect()
}

fn work_item_entries(value: Option<toml::Value>) -> Vec<WorkItemProgress> {
    let Some(toml::Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| {
            let table = item.as_table()?;
            let number = match table.get("number")? {
                toml::Value::Integer(number) => *number,
                toml::Value::String(number) => number.trim().parse::<i64>().ok()?,
                _ => return None,
            };
            Some(WorkItemProgress {
                number,
                state: truncate_chars(
                    &scalar_string(table.get("state")).unwrap_or_default(),
                    WORK_ITEM_FIELD_MAX_CHARS,
                ),
                progress: truncate_chars(
                    &scalar_string(table.get("progress")).unwrap_or_default(),
                    WORK_ITEM_FIELD_MAX_CHARS,
                ),
            })
        })
        .take(WORK_ITEMS_MAX_ENTRIES)
        .collect()
}

/// The front matter exactly as written, before validation.
///
/// Unknown keys are **ignored** (no `deny_unknown_fields`), which is what lets a newer
/// producer add fields without breaking an older control plane. Optional structure is
/// held as `toml::Value` so a malformed optional field degrades locally instead of
/// failing the whole document.
#[derive(Debug, Deserialize)]
struct RawFrontMatter {
    #[serde(default)]
    fkst_health_report: Option<toml::Value>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    producer: Option<String>,
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    window_start: Option<String>,
    #[serde(default)]
    expected_interval_secs: Option<toml::Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    headline: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    evidence: Option<toml::Value>,
    #[serde(default)]
    work_items: Option<toml::Value>,
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
