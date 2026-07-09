//! Pure unit tests for the session <-> sandbox correlation (no network). Cover the
//! stamp -> recover / to_live_pod round-trip, the state map, the config-hash split +
//! work-label hex encoding round-trips, and the loud rejection of label-unsafe values.

use std::collections::BTreeMap;

use super::super::backend_test_support::spec;
use super::*;

/// A `SandboxView` carrying `metadata` (+ state + createdAt), built directly since the
/// view is normally deserialize-only.
fn view_with(
    metadata: BTreeMap<String, String>,
    state: SandboxState,
    created_at: Option<&str>,
) -> SandboxView {
    SandboxView {
        id: "sbx-test".to_string(),
        state,
        reason: None,
        message: None,
        metadata,
        extensions: BTreeMap::new(),
        created_at: created_at.map(str::to_string),
    }
}

#[test]
fn stamp_then_recover_and_to_live_pod_round_trip() {
    let spec = spec();
    let meta = stamp(&spec).expect("stamp emits only label-safe values");

    // Identity + drift metadata, all label-safe.
    assert_eq!(meta[KEY_MANAGED], "true");
    assert_eq!(meta[KEY_SESSION_ID], spec.session_id);
    assert_eq!(meta[KEY_INSTALLATION], "42");
    assert_eq!(meta[KEY_TRIGGER_ISSUE], "7");
    assert!(meta.contains_key(KEY_LAST_PENDING));
    assert_eq!(meta[KEY_OWNER], "acme");
    assert_eq!(meta[KEY_REPO], "site");
    // The 64-hex config hash is split into two 32-char halves that reassemble exactly.
    assert_eq!(meta[KEY_CONFIG_HASH].len(), 32);
    assert_eq!(meta[KEY_CONFIG_HASH_2].len(), 32);
    assert_eq!(
        format!("{}{}", meta[KEY_CONFIG_HASH], meta[KEY_CONFIG_HASH_2]),
        spec.config_hash
    );

    let view = view_with(meta, SandboxState::Running, Some("2026-07-09T00:00:00Z"));

    let handle = recover(&view).expect("recover a well-formed sandbox");
    assert_eq!(handle.session_id, spec.session_id);
    assert_eq!(handle.installation_id, 42);
    assert_eq!(handle.repo, spec.repo);
    assert_eq!(handle.trigger_issue, Some(7));

    let pod = to_live_pod(&view).expect("project a well-formed sandbox");
    assert_eq!(pod.session_id, spec.session_id);
    assert_eq!(pod.trigger_issue, 7);
    assert_eq!(pod.liveness, PodLiveness::Live);
    // The FULL canonical hash is reconstructed (drift is compared against it).
    assert_eq!(pod.config_hash.as_deref(), Some(spec.config_hash.as_str()));
    assert_eq!(pod.work_label.as_deref(), Some("fkst-work"));
    assert!(pod.last_pending_at.is_some());
    let expected_created = DateTime::parse_from_rfc3339("2026-07-09T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(pod.created_at, expected_created);
}

#[test]
fn state_to_liveness_maps_every_state() {
    use PodLiveness::*;
    let cases = [
        (SandboxState::Running, Live),
        (SandboxState::Pending, Starting),
        (SandboxState::Failed, Terminal),
        (SandboxState::Terminated, Terminal),
        (SandboxState::Pausing, Starting),
        (SandboxState::Paused, Starting),
        (SandboxState::Resuming, Starting),
        (SandboxState::Stopping, Starting),
        (
            SandboxState::Unknown("SomeFutureState".to_string()),
            Starting,
        ),
    ];
    for (state, expected) in cases {
        assert_eq!(state_to_liveness(&state), expected, "state {state:?}");
    }
}

#[test]
fn work_label_round_trips_spaces_and_emoji() {
    let mut spec = spec();
    spec.work_label = "fix 🚀 now".to_string();
    let meta = stamp(&spec).expect("short unicode label is label-safe once hex-encoded");
    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    assert_eq!(pod.work_label.as_deref(), Some("fix 🚀 now"));
}

#[test]
fn stamp_rejects_label_unsafe_values_loudly() {
    // Each mutation makes exactly one stamped value violate the label-value contract;
    // stamp must fail loudly (never emit a create the server would reject).
    let long_label = {
        let mut s = spec();
        s.work_label = "w".repeat(40); // 40 bytes -> 80 hex chars -> exceeds 63
        s
    };
    let bad_repo_charset = {
        let mut s = spec();
        s.repo.name = "bad name!".to_string(); // space + '!' are not label-safe
        s
    };
    let long_repo = {
        let mut s = spec();
        s.repo.name = "r".repeat(64); // exceeds 63
        s
    };
    for (case, s) in [
        ("too-long work label", long_label),
        ("invalid repo charset", bad_repo_charset),
        ("too-long repo", long_repo),
    ] {
        let err = stamp(&s).expect_err(case);
        assert!(matches!(err, BackendError::Other(_)), "{case}: {err:?}");
    }
}

#[test]
fn to_live_pod_and_recover_skip_a_sandbox_with_no_session_id() {
    let view = view_with(BTreeMap::new(), SandboxState::Running, None);
    assert!(to_live_pod(&view).is_none());
    assert!(recover(&view).is_none());
}

#[test]
fn to_live_pod_reports_no_config_hash_when_a_half_is_missing() {
    let mut meta = stamp(&spec()).expect("stamp");
    meta.remove(KEY_CONFIG_HASH_2);
    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    // An incomplete hash yields no drift decision (matches K8s "annotation absent").
    assert_eq!(pod.config_hash, None);
}

#[test]
fn to_live_pod_defaults_created_at_to_now_on_absent_or_malformed_timestamp() {
    let meta = stamp(&spec()).expect("stamp");
    for created in [None, Some("not-a-timestamp")] {
        let view = view_with(meta.clone(), SandboxState::Running, created);
        let pod = to_live_pod(&view).expect("pod");
        // Falls back to ~now (shielded from idle-kill), never epoch 0.
        assert!(
            (Utc::now() - pod.created_at).num_seconds().abs() < 60,
            "created_at should default to ~now for {created:?}"
        );
    }
}
