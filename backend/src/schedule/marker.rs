//! The `fkst-cron-run:v1` run record: the durable, stateless history of a
//! scheduled workflow.
//!
//! This marker is the ONLY place a run's outcome is stored. There is no control
//! plane datastore: a schedule's cursor, its in-flight run, and its whole history
//! are recovered by re-reading these hidden comments on the definition issue. That
//! is what lets any replica take over on leader failover, and what makes the
//! history survive a control-plane rebuild.
//!
//! **The wire format is a cross-repository contract.** The control plane writes
//! `Dispatched` and the terminal records it decides itself (timeout, overlap skip);
//! the session pod's workflow-runner package writes the terminal record for a run it
//! executed. Any drift silently breaks completion detection — a run would look
//! in-flight until the watchdog released it — so the literal rendering is pinned by
//! test rather than merely described.
//!
//! Compatibility rule: parsing tolerates unknown attributes and any field order, so
//! a newer writer can add an attribute without breaking an older reader. Adding a
//! new `status` VALUE is a breaking change for readers and needs a version bump.

use std::collections::BTreeMap;

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};

use crate::error::AppError;

const MARKER_PREFIX: &str = "<!-- fkst-cron-run:v1";

/// How long a `detail` may be. It is free text from a failing step, so it is
/// truncated rather than trusted: a marker is an HTML comment on an issue, and an
/// unbounded one would push the human part of the comment out of view.
const MAX_DETAIL_CHARS: usize = 200;

/// The outcome of one scheduled run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    /// The control plane created the run issue; the run is in flight. Written by
    /// the control plane only, and the marker whose absence or presence decides
    /// whether a slot still counts as running.
    Dispatched,
    /// Every step completed.
    Ok,
    /// A step failed, or the run issue was closed without a terminal record.
    Failed,
    /// The run outlived its budget and the watchdog released the schedule.
    Timeout,
    /// The slot came due while the previous run was still in flight. Recorded so
    /// the history explains the gap, and deliberately NOT queued for later: a
    /// backlog of catch-up runs is never what an operator wants from a cron.
    SkippedOverlap,
}

impl RunStatus {
    /// The wire token. Kept beside [`Self::parse`] so the two cannot drift.
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Dispatched => "dispatched",
            RunStatus::Ok => "ok",
            RunStatus::Failed => "failed",
            RunStatus::Timeout => "timeout",
            RunStatus::SkippedOverlap => "skipped-overlap",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "dispatched" => Ok(RunStatus::Dispatched),
            "ok" => Ok(RunStatus::Ok),
            "failed" => Ok(RunStatus::Failed),
            "timeout" => Ok(RunStatus::Timeout),
            "skipped-overlap" => Ok(RunStatus::SkippedOverlap),
            other => Err(invalid_marker(&format!(
                "field `status` must be one of dispatched|ok|failed|timeout|skipped-overlap, \
                 got {other:?}"
            ))),
        }
    }

    /// True when this record ENDS a slot. `Dispatched` is the only non-terminal
    /// status, which is what makes "in flight" a question about run records rather
    /// than about a label alone.
    pub fn is_terminal(self) -> bool {
        !matches!(self, RunStatus::Dispatched)
    }
}

/// The outcome of one step within a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunStep {
    /// 1-based position in the workflow definition.
    pub index: u32,
    /// The step id from the definition.
    pub id: String,
    pub status: StepStatus,
    /// Wall-clock seconds, absent for a step that never ran.
    pub duration_s: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepStatus {
    Ok,
    Failed,
    /// A later step that never ran because an earlier one failed.
    Skipped,
}

impl StepStatus {
    fn as_str(self) -> &'static str {
        match self {
            StepStatus::Ok => "ok",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(StepStatus::Ok),
            "failed" => Some(StepStatus::Failed),
            "skipped" => Some(StepStatus::Skipped),
            _ => None,
        }
    }
}

/// A durable scheduled-run record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub slot: DateTime<Utc>,
    /// True for a run started from the dashboard's run-now action rather than by
    /// the clock. Carried so a manual run is visible in the history but never
    /// mistaken for evidence that the cadence fired.
    pub manual: bool,
    pub status: RunStatus,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    /// The run issue this slot produced.
    pub issue: Option<u64>,
    /// Short human explanation, e.g. a failing step's reason.
    pub detail: Option<String>,
    /// Per-step outcomes, so the API and the dashboard can render a stepper without
    /// re-deriving anything from the run issue.
    pub steps: Vec<RunStep>,
}

impl RunRecord {
    /// A control-plane-authored record with no step detail.
    pub fn new(slot: DateTime<Utc>, status: RunStatus, at: DateTime<Utc>) -> Self {
        Self {
            slot,
            manual: false,
            status,
            started: at,
            ended: status.is_terminal().then_some(at),
            issue: None,
            detail: None,
            steps: Vec::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_issue(mut self, issue: u64) -> Self {
        self.issue = Some(issue);
        self
    }

    pub fn manual(mut self) -> Self {
        self.manual = true;
        self
    }

    /// The run's wall-clock duration, derived rather than stored so it can never
    /// disagree with the timestamps.
    pub fn duration_s(&self) -> Option<u64> {
        self.ended
            .map(|ended| (ended - self.started).num_seconds().max(0) as u64)
    }
}

/// Render the `fkst-cron-run:v1` hidden marker.
///
/// Absent optional attributes are OMITTED rather than emitted empty, so a reader
/// never has to distinguish "absent" from "present but blank".
pub fn render_marker(record: &RunRecord) -> String {
    let mut fields = vec![
        format!("slot=\"{}\"", timestamp(record.slot)),
        format!("manual=\"{}\"", record.manual),
        format!("status=\"{}\"", record.status.as_str()),
        format!("started=\"{}\"", timestamp(record.started)),
    ];
    if let Some(ended) = record.ended {
        fields.push(format!("ended=\"{}\"", timestamp(ended)));
    }
    if let Some(issue) = record.issue {
        fields.push(format!("issue=\"{issue}\""));
    }
    if let Some(detail) = &record.detail {
        let sanitized = sanitize_detail(detail);
        if !sanitized.is_empty() {
            fields.push(format!("detail=\"{sanitized}\""));
        }
    }
    if !record.steps.is_empty() {
        fields.push(format!("steps=\"{}\"", render_steps(&record.steps)));
    }
    format!("{MARKER_PREFIX} {} -->", fields.join(" "))
}

/// `index:id:status:duration` tuples joined by `;`, with an empty duration for a
/// step that never ran.
///
/// A separator-delimited scalar rather than embedded JSON: the value lives inside
/// a double-quoted HTML-comment attribute, and step ids are already restricted to
/// the path-safe token set, so neither separator can occur in the data and no
/// escaping layer is needed on either side of the contract.
fn render_steps(steps: &[RunStep]) -> String {
    steps
        .iter()
        .map(|step| {
            format!(
                "{}:{}:{}:{}",
                step.index,
                step.id,
                step.status.as_str(),
                step.duration_s.map(|s| s.to_string()).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Strip everything that would terminate the attribute or the comment, and bound
/// the length. A detail is free text produced by a failing step, so it is treated
/// as hostile to the enclosing format rather than trusted to be well-behaved.
fn sanitize_detail(detail: &str) -> String {
    detail
        .chars()
        .map(|c| {
            if c == '"' || c == '<' || c == '>' {
                '\''
            } else {
                c
            }
        })
        .filter(|c| !c.is_control())
        .take(MAX_DETAIL_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Parse one marker, tolerating field order and unrecognized extra fields.
pub fn parse_marker(marker: &str) -> Result<RunRecord, AppError> {
    let marker = marker.trim();
    let attributes = marker
        .strip_prefix(MARKER_PREFIX)
        .and_then(|rest| rest.strip_suffix("-->"))
        .ok_or_else(|| invalid_marker("expected an `fkst-cron-run:v1` HTML marker"))?;
    let fields = parse_attributes(attributes.trim())?;

    Ok(RunRecord {
        slot: parse_timestamp(required(&fields, "slot")?, "slot")?,
        manual: parse_bool(required(&fields, "manual")?, "manual")?,
        status: RunStatus::parse(required(&fields, "status")?)?,
        started: parse_timestamp(required(&fields, "started")?, "started")?,
        ended: fields
            .get("ended")
            .map(|value| parse_timestamp(value, "ended"))
            .transpose()?,
        issue: fields
            .get("issue")
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| invalid_marker("field `issue` must be an unsigned issue number"))
            })
            .transpose()?,
        detail: fields.get("detail").cloned(),
        steps: fields
            .get("steps")
            .map(|value| parse_steps(value))
            .unwrap_or_default(),
    })
}

/// Parse the step tuples, skipping any malformed entry.
///
/// Lenient on purpose: the step list is diagnostic detail written by the pod side,
/// and losing the whole record — including its authoritative status — because one
/// step tuple was malformed would turn a display glitch into a stuck schedule.
fn parse_steps(value: &str) -> Vec<RunStep> {
    value
        .split(';')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let mut parts = entry.split(':');
            let index = parts.next()?.parse::<u32>().ok()?;
            let id = parts.next()?;
            let status = StepStatus::parse(parts.next()?)?;
            let duration = parts.next().unwrap_or_default();
            if id.is_empty() {
                return None;
            }
            Some(RunStep {
                index,
                id: id.to_string(),
                status,
                duration_s: duration.parse::<u64>().ok(),
            })
        })
        .collect()
}

fn parse_attributes(input: &str) -> Result<BTreeMap<String, String>, AppError> {
    let mut fields = BTreeMap::new();
    let mut rest = input;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let equals = rest
            .find('=')
            .ok_or_else(|| invalid_marker("expected a marker field assignment"))?;
        let key = rest[..equals].trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(invalid_marker("marker field name is invalid"));
        }
        rest = &rest[equals + 1..];
        let quoted = rest
            .strip_prefix('"')
            .ok_or_else(|| invalid_marker("marker field values must be quoted"))?;
        let quote = quoted
            .find('"')
            .ok_or_else(|| invalid_marker("marker field has an unterminated value"))?;
        let value = &quoted[..quote];
        rest = &quoted[quote + 1..];
        if fields.insert(key.to_string(), value.to_string()).is_some() {
            return Err(invalid_marker(&format!("duplicate marker field `{key}`")));
        }
    }
    Ok(fields)
}

/// Every `fkst-cron-run:v1` record in `comments`, newest LAST.
///
/// Malformed markers are skipped rather than propagated: a single hand-edited
/// comment must not be able to hide a schedule's entire history and strand it.
pub fn collect_records(comments: &[String]) -> Vec<RunRecord> {
    comments
        .iter()
        .flat_map(|body| body.lines())
        .filter(|line| line.trim_start().starts_with(MARKER_PREFIX))
        .filter_map(|line| parse_marker(line).ok())
        .collect()
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| invalid_marker(&format!("missing required marker field `{key}`")))
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| invalid_marker(&format!("field `{field}` must be an RFC 3339 timestamp")))
}

fn parse_bool(value: &str, field: &str) -> Result<bool, AppError> {
    value
        .parse::<bool>()
        .map_err(|_| invalid_marker(&format!("field `{field}` must be `true` or `false`")))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn invalid_marker(detail: &str) -> AppError {
    AppError::Unprocessable(format!("invalid fkst-cron-run:v1 marker: {detail}"))
}

#[cfg(test)]
#[path = "marker_tests.rs"]
mod tests;
