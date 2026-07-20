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
    assert_eq!(pod.work_labels, vec!["fkst-work".to_string()]);
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
    assert_eq!(pod.work_labels, vec!["fix 🚀 now".to_string()]);
}

#[test]
fn comma_joined_work_label_set_round_trips_to_the_full_set() {
    // A multi-label session (epic #594 I4): the comma-joined effective set is hex-stamped
    // and split back into the individual labels on projection.
    let mut spec = spec();
    spec.work_label = "alpha,beta".to_string();
    let meta = stamp(&spec).expect("short comma-joined set is label-safe once hex-encoded");
    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    assert_eq!(
        pod.work_labels,
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

#[test]
fn stamp_rejects_label_unsafe_values_loudly() {
    // Each mutation makes exactly one stamped value violate the label-value contract;
    // stamp must fail loudly (never emit a create the server would reject). The work
    // label is NO LONGER a failure mode — it is hex-chunked across ≤63-char keys — so a
    // pathological value can now only come from the RAW owner/repo names.
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
        ("invalid repo charset", bad_repo_charset),
        ("too-long repo", long_repo),
    ] {
        let err = stamp(&s).expect_err(case);
        assert!(matches!(err, BackendError::Other(_)), "{case}: {err:?}");
    }
}

#[test]
fn default_three_label_manifest_set_spans_two_keys_and_round_trips() {
    // The default-workflows manifest's three labels: 36 UTF-8 bytes → 72 hex chars, which
    // exceeds the 63-char label-value cap in a single key. The N-way chunk split stamps it
    // across TWO keys (72 hex → 62 + 10) and recovers all three on projection. This exact
    // set previously failed `stamp` outright — the live opensandbox default.
    let mut spec = spec();
    spec.work_label = "fkst-dev,fkst-security,fkst-workflow".to_string();
    let meta = stamp(&spec).expect("the default 3-label set now stamps (no error)");

    let k1 = work_label_chunk_key(1);
    assert!(meta.contains_key(KEY_WORK_LABEL), "chunk 0 present");
    assert!(meta.contains_key(&k1), "chunk 1 (continuation) present");
    assert!(
        !meta.contains_key(&work_label_chunk_key(2)),
        "no third chunk for a 72-hex set"
    );
    assert!(meta[KEY_WORK_LABEL].len() <= 63);
    assert!(meta[&k1].len() <= 63);

    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    assert_eq!(
        pod.work_labels,
        vec![
            "fkst-dev".to_string(),
            "fkst-security".to_string(),
            "fkst-workflow".to_string(),
        ]
    );
}

#[test]
fn a_single_short_label_uses_exactly_one_work_label_key() {
    // A short single-label set is exactly ONE chunk, hex byte-identical to the pre-chunk
    // stamp (existing single-label round-trips stay green).
    let meta = stamp(&spec()).expect("stamp"); // spec work_label = "fkst-work"
    assert!(meta.contains_key(KEY_WORK_LABEL));
    assert!(
        !meta.contains_key(&work_label_chunk_key(1)),
        "a short single-label set needs no continuation key"
    );
    assert_eq!(meta[KEY_WORK_LABEL], "666b73742d776f726b"); // hex of "fkst-work"
}

#[test]
fn a_large_label_set_spanning_three_keys_round_trips() {
    // A set whose hex exceeds 2 chunks (> 62 UTF-8 bytes → > 124 hex) must span ≥3 keys
    // and still recover exactly.
    let labels = "aaaaaaaaaa,bbbbbbbbbb,cccccccccc,dddddddddd,eeeeeeeeee,ffffffffff,gggggggggg";
    assert!(
        labels.len() > 62,
        "the set must exceed one 62-hex chunk's worth of bytes"
    );
    let mut spec = spec();
    spec.work_label = labels.to_string();
    let meta = stamp(&spec).expect("a large set stamps across ≥3 keys");
    assert!(
        meta.contains_key(&work_label_chunk_key(2)),
        "a >124-hex set needs a third chunk"
    );
    // Every chunk stays within the label-value cap.
    for i in 0.. {
        match meta.get(&work_label_chunk_key(i)) {
            Some(chunk) => assert!(chunk.len() <= 63, "chunk {i} within the 63-char cap"),
            None => break,
        }
    }

    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    let expected: Vec<String> = labels.split(',').map(str::to_string).collect();
    assert_eq!(pod.work_labels, expected);
}

#[test]
fn to_live_pod_stops_at_a_missing_continuation_chunk() {
    // Chunk 0 present, chunk 1 (continuation) missing, a stray chunk 2 left by a partial
    // rewrite: reassembly stops at the gap and ignores the stray (robust, never a crash).
    let mut spec = spec();
    spec.work_label = "alpha".to_string(); // 5 bytes → 10 hex → chunk 0 only
    let mut meta = stamp(&spec).expect("stamp");
    meta.insert(work_label_chunk_key(2), "deadbeef".to_string()); // stray, no chunk 1
    let view = view_with(meta, SandboxState::Running, None);
    let pod = to_live_pod(&view).expect("pod");
    assert_eq!(
        pod.work_labels,
        vec!["alpha".to_string()],
        "a stray continuation past a gap is ignored"
    );
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
