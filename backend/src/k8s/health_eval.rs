//! Pure, package-AGNOSTIC session-health evaluation (session health, PR).
//!
//! fkst-hosted hosts arbitrary fkst packages and must NEVER encode any one
//! package's notion of "healthy" — a github-devloop and a codex-triage define
//! useful work completely differently. So this module derives "degraded" from ONLY
//! the two things EVERY package shares because they run on the same fkst framework
//! inside a pod we control:
//!
//!   (a) the Kubernetes POD STATUS — restarts, phase, a bad waiting reason; and
//!   (b) the framework's OWN structured LOG SEVERITY — an `error` line, or a `warn`
//!       line that RECURS. The package decides what is a warning; we only RELAY it
//!       VERBATIM. We never interpret the message text (never assert "no-op") — we
//!       quote the framework's own line and report how often it recurred.
//!
//! Everything here is pure (no I/O) so the decision is exhaustively unit-testable
//! without a cluster; the effectful scrape ([`crate::k8s::health_scrape`]) feeds it
//! a [`PodStatusSummary`] + the parsed logs and acts on the [`HealthVerdict`].

use std::collections::HashMap;

use k8s_openapi::api::core::v1::Pod;
use once_cell::sync::Lazy;
use regex::Regex;

/// A `warn`-level message must recur at least this many times in the scanned window
/// before it is treated as a degraded signal. A single warning is normal fkst
/// framework chatter; the same warning every cycle is the "green pod, no useful
/// work" smell this feature exists to catch. `error` needs no recurrence.
pub const WARN_RECUR_THRESHOLD: usize = 3;

/// The severity of one framework log line, classified case-insensitively from the
/// package's OWN level token. We only ever act on these two rungs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Warn,
    Error,
}

impl Severity {
    /// The lower-case label used in the relayed comment.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
}

/// One distinct (normalized) framework message: a VERBATIM sample line for display,
/// its severity, and how many raw lines collapsed into this bucket (the recurrence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStat {
    /// One raw log line, quoted exactly as the framework emitted it (never edited).
    pub sample_verbatim: String,
    pub level: Severity,
    pub count: usize,
}

/// The coarse, package-agnostic pod-status facts the evaluator reads. Kept as a
/// plain struct (not a raw `PodStatus`) so tests construct it trivially and the
/// evaluator never reaches into Kubernetes types.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PodStatusSummary {
    /// The pod `status.phase` (`Running`, `Pending`, `Failed`, …), if reported.
    pub phase: Option<String>,
    /// The greatest container `restartCount` across the pod (0 = never restarted).
    pub restart_count: i32,
    /// The first container's `state.waiting.reason`, if any (e.g. `CrashLoopBackOff`).
    pub waiting_reason: Option<String>,
}

/// The evaluator's verdict for one pod. `Degraded` carries the VERBATIM offender
/// (the framework's own line, or the raw status fact) plus a `detail` sentence
/// describing recurrence / context — never a fabricated diagnosis of the work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthVerdict {
    Healthy,
    Degraded {
        reason_verbatim: String,
        detail: String,
    },
}

// --- Log parsing -------------------------------------------------------------

/// Long hex/id runs (SHAs, object ids) → a stable placeholder so the same message
/// with a different id collapses into one bucket. Applied before [`RE_NUM`].
static RE_HEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{7,}\b").expect("valid regex"));

/// Any remaining digit run (counts, short ids, the digits inside a timestamp or a
/// `path/123`) → a placeholder, so numeric drift does not fragment a bucket.
static RE_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[0-9]+").expect("valid regex"));

/// Collapse runs of whitespace to a single space when building the grouping key.
static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("valid regex"));

/// Classify a raw level token to one of the two rungs we act on, case-insensitively.
/// `fatal`/`critical` are strictly worse than `error`, so they map to [`Severity::Error`]
/// (we read the SEVERITY token, never the message, so this is not package logic).
/// Anything else (info/debug/trace/unknown) is `None` and ignored.
fn classify_level(raw: &str) -> Option<Severity> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("error")
        || lower == "err"
        || lower == "fatal"
        || lower == "critical"
        || lower == "crit"
    {
        return Some(Severity::Error);
    }
    if lower.starts_with("warn") {
        return Some(Severity::Warn);
    }
    None
}

/// Extract the (severity, verbatim message) of a single log line in EITHER format,
/// or `None` if the line is not a warn/error severity line we recognize.
fn parse_line(line: &str) -> Option<(Severity, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') {
        if let Some(parsed) = parse_json_line(trimmed) {
            return Some(parsed);
        }
        // A `{`-leading line that is not the JSON severity shape still falls through
        // to the space-kv scan (defensive; costs nothing).
    }
    parse_kv_line(trimmed)
}

/// JSON line: `{"level":"WARN","fields":{"message":"…"}}` (message may also sit at
/// the top level). Returns `None` for a non-object, an unclassifiable level, or a
/// missing message.
fn parse_json_line(line: &str) -> Option<(Severity, String)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    let level = classify_level(obj.get("level")?.as_str()?)?;
    let message = obj
        .get("fields")
        .and_then(|f| f.get("message"))
        .and_then(|m| m.as_str())
        .or_else(|| obj.get("message").and_then(|m| m.as_str()))?;
    Some((level, message.to_string()))
}

/// Space-kv line: `TIMESTAMP=… LEVEL=warn … MSG=<message to end of line>`. The
/// `MSG=` value runs to the end of the line (it can contain spaces), so it must be
/// the last key — the framework format we relay. Returns `None` without a
/// classifiable `LEVEL=` or a `MSG=`.
fn parse_kv_line(line: &str) -> Option<(Severity, String)> {
    let level_at = line.find("LEVEL=")?;
    let level_val = line[level_at + "LEVEL=".len()..]
        .split_whitespace()
        .next()?;
    let level = classify_level(level_val)?;
    let msg_at = line.find("MSG=")?;
    let message = line[msg_at + "MSG=".len()..].trim();
    if message.is_empty() {
        return None;
    }
    Some((level, message.to_string()))
}

/// Normalize a message for GROUPING only: strip volatile ids/numbers/whitespace so
/// the same recurring warning collapses to one bucket. The verbatim sample shown to
/// the user is NEVER normalized — only this derived key is.
fn normalize_message(msg: &str) -> String {
    let hexless = RE_HEX.replace_all(msg, "<id>");
    let numless = RE_NUM.replace_all(&hexless, "<n>");
    let collapsed = RE_WS.replace_all(&numless, " ");
    collapsed.trim().to_ascii_lowercase()
}

/// Parse a whole log window into per-distinct-message stats: for each
/// `(severity, normalized-message)` bucket, keep the FIRST verbatim sample, its
/// level, and the recurrence count. First-seen order is preserved (stable output).
pub fn parse_severity_lines(logs: &str) -> Vec<MessageStat> {
    // First-seen order is preserved implicitly by the push order into `stats`; the
    // index maps a bucket key to its slot so recurrences just bump the count.
    let mut index: HashMap<(Severity, String), usize> = HashMap::new();
    let mut stats: Vec<MessageStat> = Vec::new();

    for line in logs.lines() {
        let Some((level, message)) = parse_line(line) else {
            continue;
        };
        let key = (level, normalize_message(&message));
        match index.get(&key) {
            Some(&i) => stats[i].count += 1,
            None => {
                index.insert(key, stats.len());
                stats.push(MessageStat {
                    sample_verbatim: message,
                    level,
                    count: 1,
                });
            }
        }
    }
    stats
}

// --- Pod status projection ---------------------------------------------------

/// Waiting reasons that are NORMAL during a healthy pod startup and must NOT be
/// mistaken for a degraded session. Anything else (CrashLoopBackOff, ImagePullBackOff,
/// CreateContainerError, …) is a genuine problem.
fn is_benign_waiting(reason: &str) -> bool {
    matches!(reason, "ContainerCreating" | "PodInitializing")
}

/// Project a live pod's status into the coarse [`PodStatusSummary`] the evaluator
/// reads. Pure over the pod object (no I/O), so it is unit-testable with a fixture.
pub fn summarize_pod_status(pod: &Pod) -> PodStatusSummary {
    let status = pod.status.as_ref();
    let phase = status.and_then(|s| s.phase.clone());
    let containers = status.and_then(|s| s.container_statuses.as_ref());
    let restart_count = containers
        .map(|cs| cs.iter().map(|c| c.restart_count).max().unwrap_or(0))
        .unwrap_or(0);
    let waiting_reason = containers.and_then(|cs| {
        cs.iter()
            .find_map(|c| c.state.as_ref()?.waiting.as_ref()?.reason.clone())
    });
    PodStatusSummary {
        phase,
        restart_count,
        waiting_reason,
    }
}

/// The status-only degraded offender (reason_verbatim, detail), if the pod status
/// alone shows trouble: a restart, a non-benign waiting reason, or a `Failed`/
/// `Unknown` phase. A `Running`/`Pending`/`Succeeded`/not-yet-observed pod is NOT
/// status-degraded (Pending = still starting; Succeeded = clean exit).
fn status_offender(status: &PodStatusSummary) -> Option<(String, String)> {
    let phase = status.phase.as_deref().unwrap_or("unknown");
    if status.restart_count > 0 {
        return Some((
            format!("the session pod container has restarted {}×", status.restart_count),
            format!(
                "restartCount={} (phase {phase}); the pod is flapping rather than running steadily.",
                status.restart_count
            ),
        ));
    }
    if let Some(reason) = status.waiting_reason.as_deref() {
        if !is_benign_waiting(reason) {
            return Some((
                format!("the session pod container is stuck waiting: {reason}"),
                format!(
                    "container state is Waiting with reason {reason} (phase {phase}); the pod is not running its work."
                ),
            ));
        }
    }
    match status.phase.as_deref() {
        Some("Failed") | Some("Unknown") => Some((
            format!("the session pod is in phase {phase}"),
            format!("pod phase {phase} (expected Running); the session is not running."),
        )),
        _ => None,
    }
}

/// The most significant stat of a given severity: the highest recurrence count,
/// ties broken by first-seen (so output is deterministic). `None` if none match.
fn most_significant(parsed: &[MessageStat], level: Severity) -> Option<&MessageStat> {
    let mut best: Option<&MessageStat> = None;
    for stat in parsed.iter().filter(|s| s.level == level) {
        match best {
            Some(b) if b.count >= stat.count => {}
            _ => best = Some(stat),
        }
    }
    best
}

/// Detail sentence for an error-level offender (verbatim relay, no diagnosis).
fn error_detail(stat: &MessageStat) -> String {
    if stat.count > 1 {
        format!(
            "the session's own framework logged this at error level {}× in the recent window; \
             the pod is up but is reporting a failure.",
            stat.count
        )
    } else {
        "the session's own framework logged this at error level; the pod is up but is \
         reporting a failure."
            .to_string()
    }
}

/// Detail sentence for a recurring warn offender (verbatim relay, no diagnosis).
fn warn_recur_detail(stat: &MessageStat) -> String {
    format!(
        "logged {}× in the recent window; the pod is up but its own framework keeps reporting \
         this, so it may be running without doing useful work.",
        stat.count
    )
}

/// Evaluate one pod's health from its status + parsed framework logs. Priority when
/// several signals fire at once: an ERROR line outranks a RECURRING WARN, which
/// outranks a pod-STATUS problem — the offender chosen for `reason_verbatim` is the
/// most significant one. `Healthy` only when NONE fire.
pub fn evaluate_health(status: &PodStatusSummary, parsed: &[MessageStat]) -> HealthVerdict {
    // 1. Any error-level framework line (no recurrence needed).
    if let Some(stat) = most_significant(parsed, Severity::Error) {
        return HealthVerdict::Degraded {
            reason_verbatim: stat.sample_verbatim.clone(),
            detail: error_detail(stat),
        };
    }
    // 2. A warn-level line that RECURS at/above the threshold.
    if let Some(stat) = most_significant(parsed, Severity::Warn) {
        if stat.count >= WARN_RECUR_THRESHOLD {
            return HealthVerdict::Degraded {
                reason_verbatim: stat.sample_verbatim.clone(),
                detail: warn_recur_detail(stat),
            };
        }
    }
    // 3. Pod status alone (restart / crashloop / Failed) — even with clean logs.
    if let Some((reason_verbatim, detail)) = status_offender(status) {
        return HealthVerdict::Degraded {
            reason_verbatim,
            detail,
        };
    }
    HealthVerdict::Healthy
}

#[cfg(test)]
#[path = "health_eval_tests.rs"]
mod tests;
