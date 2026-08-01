//! Milestone acceptance: the canary sweep across every TEST-OWNED output at
//! once, plus the things a per-surface sweep cannot do.
//!
//! `audit_redaction_canary.rs` proves each surface individually. What it cannot
//! prove is the union: that no single canary escapes through ANY of the outputs
//! a milestone reviewer actually reads — the records, their exact PostHog
//! payloads, the metrics exposition, the durable relay's own SQLite file and
//! WAL, the relay's read responses, an authorized operations API answer, the
//! checked-in alert labels and annotations, and the generated evidence artifact.
//! A value that is scrubbed from seven surfaces and present in the eighth is
//! still a leak, and a per-surface suite has no place to notice it.
//!
//! ## Every surface here can actually carry a canary
//!
//! That is not a given, and an earlier version of this suite got it wrong twice:
//! it scanned a sandbox response built from clean fixtures (no canary existed to
//! find) and an evidence artifact read off disk with `unwrap_or_default()` (an
//! empty string whenever the suite that writes it had not run first, which test
//! -binary ordering never guarantees). Both assertions were vacuous. Now the
//! sandbox fleet carries a canary in a HIDDEN row — so a pass means authorization
//! removed it, not that nothing was there — and the artifact is rendered in this
//! process from the same matrix the gate reads.
//!
//! The suite also carries two assertions that only make sense at this level:
//!
//! - POSITIVE controls, so an implementation that recorded nothing at all could
//!   not pass by being empty. They assert VALUES, not field names: a `Debug`
//!   rendering always prints its field names, so "the output contains
//!   `operation_id`" holds for a completely blank record.
//! - a cardinality scan of the RENDERED exposition, rather than of the Rust
//!   label constants. The constants being bounded is necessary; it is not
//!   sufficient, because a metric family could still interpolate a value into a
//!   label at render time.

#[path = "acceptance/mod.rs"]
mod acceptance;
mod audit_canary;
#[path = "audit_relay_harness/mod.rs"]
mod relay;
mod sandbox_harness;

use audit_canary::{plant_every_canary, rendered, Canary, CANARIES};
use sandbox_harness::{fleet, harness_with};

/// A canary planted in a runtime row the viewer is NOT authorized to see.
const HIDDEN_RUNTIME_CANARY: &str = "canary-hidden-session-name";

/// The relay credentials, which are canaries by construction (see the relay
/// harness). They belong in the union corpus because the relay's storage and its
/// read responses are surfaces the per-request corpus never touches.
const RELAY_CANARIES: [&str; 2] = [relay::WRITE_TOKEN, relay::READ_TOKEN];

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
    surfaces.push(("operations sandbox api", hidden_row_sandbox_json().await));
    surfaces.push(("acceptance evidence artifact", evidence_artifact()));
    surfaces.push(("alert labels and annotations", alert_rule_text()));

    let (storage, read_response) = relay_surfaces().await;
    surfaces.push(("relay sqlite file and wal", storage));
    surfaces.push(("relay scoped read response", read_response));

    let mut escapes = Vec::new();
    for (surface, text) in &surfaces {
        for planted in CANARIES.iter().chain(RELAY_CANARIES.iter()) {
            if text.contains(planted) {
                escapes.push(format!("{planted} reached the {surface}"));
            }
        }
        if text.contains(HIDDEN_RUNTIME_CANARY) {
            escapes.push(format!("{HIDDEN_RUNTIME_CANARY} reached the {surface}"));
        }
    }
    assert!(escapes.is_empty(), "{escapes:#?}");
}

/// The positive control: the safe identifiers, counts, and flags the epic
/// deliberately KEEPS are still there — asserted as VALUES.
///
/// Without this, an implementation that recorded an empty argument map for every
/// operation would sail through the sweep above. And without the value-level
/// form, an implementation that recorded a record of entirely empty fields would
/// sail through this one: `format!("{event:#?}")` prints `operation_id: ""`,
/// which contains the string `operation_id`.
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

    // The correlation vocabulary, asserted by the VALUES it must carry.
    // `operation_id` and `route_template` are the normalized names the audited
    // surface actually uses, so naming three of them pins the projection rather
    // than the struct's field list.
    for operation_id in ["canvas_overview", "create_repo", "github_app_webhook"] {
        assert!(
            events
                .iter()
                .any(|event| event.operation_id == operation_id),
            "no record carries the operation id {operation_id}"
        );
    }
    assert!(
        events
            .iter()
            .any(|event| event.route_template == "/api/v1/logs/{session_id}/file"),
        "no record carries a normalized route template"
    );
    // Every record carries a non-empty request id and a real event id — the two
    // correlation handles the operations API pages and de-duplicates on.
    for event in &events {
        assert!(
            !event.request_id.trim().is_empty(),
            "a {} record carries an empty request id",
            event.operation_id
        );
        assert_ne!(
            event.event_id,
            uuid::Uuid::nil(),
            "a {} record carries a nil event id",
            event.operation_id
        );
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

/// One operations sandbox response, served to a viewer who may NOT see the row
/// that carries the canary.
///
/// This is the only test-owned output that is SERVED to a browser, and the
/// canary is in the hidden row on purpose: a clean fleet would make the scan
/// vacuous, while a canary in a VISIBLE row would make it wrong (the API is
/// supposed to return that row's fields). What the sweep proves here is that
/// authorization removed the hidden row before serialization — the same property
/// `operations_sandboxes_isolation.rs` proves byte-for-byte, restated as part of
/// the union.
async fn hidden_row_sandbox_json() -> String {
    let harness = harness_with(vec![
        fleet::item("rt-alice", Some(sandbox_harness::SESSION)),
        // A row belonging to a session in nobody's fixture, whose identity text
        // is the canary.
        fleet::Item {
            session_id: Some(HIDDEN_RUNTIME_CANARY.to_string()),
            creator_login: Some(HIDDEN_RUNTIME_CANARY.to_string()),
            raw_status: HIDDEN_RUNTIME_CANARY.to_string(),
            ..fleet::item("rt-hidden", Some(sandbox_harness::OTHER_SESSION))
        },
    ])
    .await;
    // Alice is a regular user: she is authorized for her own session and for
    // nothing else, so the hidden row must not appear at all.
    let bytes = harness.snapshot_bytes(sandbox_harness::ALICE, "").await;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A real relay's stored bytes and one scoped read response.
///
/// The relay's own write and read credentials are canaries, so a token that
/// reached a row, the WAL, or an answer shows up here. The spec names "relay
/// SQLite rows/WAL-aware logical export" as a hostile location precisely because
/// it is the one durable store this deployment owns.
async fn relay_surfaces() -> (String, String) {
    let node = relay::Relay::start().await;
    node.seed_cross_user_fixture().await;
    let stored = String::from_utf8_lossy(&node.database_bytes()).into_owned();
    let rows = node.read_all().await;
    let response = serde_json::to_string(&rows)
        .unwrap_or_else(|error| panic!("the relay's rows must serialize for the scan: {error}"));
    (stored, response)
}

/// The evidence artifact, rendered HERE from the same matrix the gate reads.
///
/// Rendering rather than reading is deliberate: reading the file makes the scan
/// depend on whether `acceptance_matrix` happened to run first, which nothing
/// guarantees across test binaries — and an absent file read as an empty string
/// is a scan that asserts nothing while looking like it passed.
fn evidence_artifact() -> String {
    let root = acceptance::repo_root();
    let matrix = acceptance::model::Matrix::load(&root).expect("the checked-in matrix parses");
    acceptance::report::render(&matrix, &acceptance::report::build_commit(&root))
}

/// The checked-in alert rules, whose labels and annotations reach an operator's
/// pager and an incident channel.
fn alert_rule_text() -> String {
    let monitoring = acceptance::repo_root().join("deploy/kubernetes/monitoring");
    let mut text = String::new();
    for name in ["audit-prometheus-rules.yaml", "prometheus-rules.yaml"] {
        let path = monitoring.join(name);
        text.push_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        );
        text.push('\n');
    }
    assert!(
        text.contains("annotations:"),
        "the alert rules carry no annotations; the scan would prove nothing"
    );
    text
}
