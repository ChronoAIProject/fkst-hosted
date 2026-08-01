//! Milestone acceptance: the canary sweep across every TEST-OWNED output at
//! once, plus the two things a per-surface sweep cannot do.
//!
//! `audit_redaction_canary.rs` proves each surface individually. What it cannot
//! prove is the union: that no single canary escapes through ANY of the outputs
//! a milestone reviewer actually reads — the records, their exact PostHog
//! payloads, the metrics exposition, the operations API's own JSON, and the
//! generated evidence artifact. A value that is scrubbed from four surfaces and
//! present in the fifth is still a leak, and a per-surface suite has no place to
//! notice it.
//!
//! The suite also carries two assertions that only make sense at this level:
//!
//! - a POSITIVE control, so an implementation that recorded nothing at all could
//!   not pass by being empty;
//! - a cardinality scan of the RENDERED exposition, rather than of the Rust
//!   label constants. The constants being bounded is necessary; it is not
//!   sufficient, because a metric family could still interpolate a value into a
//!   label at render time.

mod audit_canary;
mod sandbox_harness;

use audit_canary::{plant_every_canary, rendered, Canary, CANARIES};
use sandbox_harness::{fleet, harness_with};

/// One canary escaping through ANY test-owned output fails here.
#[tokio::test]
async fn no_canary_survives_into_any_test_owned_output() {
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;

    let events = canary.events();
    assert!(!events.is_empty(), "the harness recorded nothing at all");

    // Every output this process owns, concatenated into one haystack. Reading
    // them together is the point: a per-surface loop would report the first
    // failure and stop, and the union is what the acceptance claim is about.
    let mut surfaces: Vec<(&str, String)> = Vec::new();
    for event in &events {
        surfaces.push(("audit record + posthog payload", rendered(event)));
    }
    surfaces.push(("metrics exposition", canary.metrics_text().await));
    surfaces.push(("operations sandbox api", operations_sandbox_json().await));
    surfaces.push((
        "acceptance evidence artifact",
        std::fs::read_to_string(evidence_path()).unwrap_or_default(),
    ));

    let mut escapes = Vec::new();
    for (surface, text) in &surfaces {
        for planted in CANARIES {
            if text.contains(planted) {
                escapes.push(format!("{planted} reached the {surface}"));
            }
        }
    }
    assert!(escapes.is_empty(), "{escapes:#?}");
}

/// The positive control: the safe identifiers, counts, and flags the epic
/// deliberately KEEPS are still there.
///
/// Without this, an implementation that recorded an empty argument map for every
/// operation would sail through the sweep above.
#[tokio::test]
async fn the_intended_safe_identifiers_are_still_present() {
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;

    let events = canary.events();
    let all = events.iter().map(rendered).collect::<Vec<_>>().join("\n");

    // The verified numeric id — the one identity fact the whole authorization
    // model rests on — must be recorded, not scrubbed along with the payload.
    assert!(
        all.contains(&audit_canary::USER_ID.to_string()),
        "the verified actor id is absent from every record"
    );
    // Route templates and operation ids are the correlation vocabulary.
    for expected in ["operation_id", "route_template", "request_id", "event_id"] {
        assert!(all.contains(expected), "no record carries {expected}");
    }
    // At least one record must carry a non-empty safe-argument map, or the
    // allowlist is doing nothing at all.
    let with_arguments = events
        .iter()
        .filter(|event| !event.arguments.is_empty())
        .count();
    assert!(
        with_arguments > 0,
        "no recorded operation kept a single safe argument"
    );
    // And the outcome vocabulary is present, so the records are terminal rather
    // than blank.
    assert!(
        events.iter().any(|event| event.status_code.is_some()),
        "no record carries a status at all"
    );
}

/// Every label VALUE in the rendered exposition is drawn from a bounded
/// vocabulary — no numeric ids, no logins, no session or runtime identifiers.
///
/// Epic `OPS-04` is about the exposition, not about the constants: a family that
/// formatted an id into a label would satisfy every constant-level test and
/// still blow up a Prometheus instance.
#[tokio::test]
async fn the_whole_exposition_carries_no_unbounded_label_value() {
    let canary = Canary::start().await;
    plant_every_canary(&canary).await;
    let exposition = canary.metrics_text().await;

    let mut offenders = Vec::new();
    let mut inspected = 0usize;
    for line in exposition.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let Some(open) = line.find('{') else {
            continue;
        };
        let Some(close) = line[open..].find('}') else {
            continue;
        };
        for pair in line[open + 1..open + close].split(',') {
            let Some((name, raw)) = pair.split_once('=') else {
                continue;
            };
            let value = raw.trim().trim_matches('"');
            inspected += 1;
            if !is_bounded_label_value(value) {
                offenders.push(format!(
                    "{}: {name}={value}",
                    line.split('{').next().unwrap_or(line)
                ));
            }
        }
    }
    assert!(
        inspected > 20,
        "the exposition produced almost no labels to inspect; the scan is not \
         doing what it claims ({inspected} inspected)"
    );
    assert!(
        offenders.is_empty(),
        "these labels are not drawn from a closed vocabulary: {offenders:#?}"
    );
}

/// A bounded label value is a short, lower-case, snake/dot/dash token — the
/// shape every closed enum in this codebase renders.
///
/// A purely numeric value is rejected outright: that is the shape of an id, and
/// no reviewed label in this deployment is a number. `le` bucket bounds are the
/// one legitimate numeric label a histogram carries, so they are named.
fn is_bounded_label_value(value: &str) -> bool {
    const NUMERIC_LABELS_ALLOWED: [&str; 1] = ["+Inf"];
    if NUMERIC_LABELS_ALLOWED.contains(&value) {
        return true;
    }
    if value.is_empty() || value.len() > 48 {
        return false;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '/'))
}

/// One authorized operations sandbox response, as JSON text.
///
/// Included in the sweep because it is the only test-owned output that is
/// SERVED to a browser: everything else here is internal telemetry.
async fn operations_sandbox_json() -> String {
    let harness = harness_with(vec![
        fleet::item("rt-alice", Some(sandbox_harness::SESSION)),
        fleet::orphan("rt-orphan"),
    ])
    .await;
    let bytes = harness
        .snapshot_bytes(sandbox_harness::GRACE, "?scope=all")
        .await;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Where `acceptance_matrix` writes the evidence artifact. Absent until that
/// suite has run, which is fine — an absent artifact carries no canary.
fn evidence_path() -> std::path::PathBuf {
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
    };
    target.join("acceptance").join("requirement-report.md")
}
