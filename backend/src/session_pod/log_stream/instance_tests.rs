//! Tests for the instance id + branch-root document builders. Split into a sibling
//! file so `instance.rs` stays under the 500-line module cap.

use super::*;

use k8s_openapi::chrono::TimeZone;

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 2, 14, 15, 30)
        .single()
        .expect("valid instant")
}

#[test]
fn instance_id_uses_the_basic_utc_stamp_and_uid_prefix() {
    let id = compute_instance_id(fixed_now(), "a1b2c3d4-e5f6-7890-abcd-ef0123456789", "pod-x");
    assert_eq!(id, "20260702T141530Z-a1b2c3d4");
}

#[test]
fn instance_id_stamp_is_separatorless_and_z_suffixed() {
    let id = compute_instance_id(fixed_now(), "deadbeef1234", "pod-x");
    let (stamp, uid) = id.split_once('-').expect("dash between stamp and uid");
    assert_eq!(stamp, "20260702T141530Z");
    assert_eq!(uid, "deadbeef");
    assert_eq!(uid.len(), 8, "the pod short id is always 8 chars");
}

#[test]
fn instance_id_falls_back_to_a_name_hash_without_a_uid() {
    let id = compute_instance_id(fixed_now(), "  ", "fkst-sess-abc123");
    let (_, uid) = id.split_once('-').expect("dash");
    assert_eq!(uid.len(), 8, "hash fallback still yields an 8-char id");
    assert!(
        uid.chars().all(|c| c.is_ascii_hexdigit()),
        "hash id is hex: {uid}"
    );
    // Deterministic per pod name.
    let again = compute_instance_id(fixed_now(), "", "fkst-sess-abc123");
    assert_eq!(id, again);
    // Different name → different id.
    let other = compute_instance_id(fixed_now(), "", "fkst-sess-zzz");
    assert_ne!(id, other);
}

#[test]
fn meta_json_carries_the_expected_shape() {
    let meta = InstanceMeta::new(
        "20260702T141530Z-a1b2c3d4".to_string(),
        fixed_now(),
        "a1b2c3d4-e5f6".to_string(),
        "main".to_string(),
        "cfg-deadbeef".to_string(),
        7,
        "acme/site".to_string(),
    );
    let json = meta.to_json();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed["instance"], "20260702T141530Z-a1b2c3d4");
    assert_eq!(parsed["start_time"], "2026-07-02T14:15:30Z");
    assert_eq!(parsed["pod_uid"], "a1b2c3d4-e5f6");
    assert_eq!(parsed["engine_ref"], "main");
    assert_eq!(parsed["config_hash"], "cfg-deadbeef");
    assert_eq!(parsed["trigger_issue"], 7);
    assert_eq!(parsed["repo"], "acme/site");
    assert!(json.ends_with('\n'), "meta.json ends with a newline");
}

#[test]
fn readme_links_the_trigger_issue_and_flags_redaction() {
    let md = readme_markdown("acme/site", 7, "fkst-logs/issue-7");
    assert!(
        md.contains("https://github.com/acme/site/issues/7"),
        "issue link: {md}"
    );
    assert!(md.contains("fkst-logs/issue-7"), "branch name: {md}");
    assert!(
        md.to_lowercase().contains("redacted"),
        "redaction notice: {md}"
    );
    assert!(
        md.to_lowercase().contains("auto-generated"),
        "generated notice: {md}"
    );
}
