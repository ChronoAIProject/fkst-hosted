//! Unit tests for the safe-argument bounds and validators.

use super::*;

#[test]
fn owner_and_repo_accept_the_validated_shape_and_nothing_else() {
    assert_eq!(
        safe_owner("ChronoAIProject").as_deref(),
        Some("ChronoAIProject")
    );
    assert_eq!(
        safe_repo("fkst-hosted.v2_x").as_deref(),
        Some("fkst-hosted.v2_x")
    );
    // Anything that could forge a path segment, a field, or a log line is not a
    // validated owner and is therefore dropped, never truncated.
    for hostile in [
        "acme/evil",
        "acme evil",
        "acme\nevil",
        "acme\"evil",
        "",
        "   ",
    ] {
        assert_eq!(safe_owner(hostile), None, "{hostile:?} must be dropped");
    }
    assert_eq!(safe_owner(&"a".repeat(MAX_OWNER_LEN + 1)), None);
    assert_eq!(safe_repo(&"a".repeat(MAX_REPO_LEN + 1)), None);
}

#[test]
fn a_repo_full_name_needs_both_halves() {
    assert_eq!(
        safe_repo_full_name("acme", "site").as_deref(),
        Some("acme/site")
    );
    assert_eq!(safe_repo_full_name("acme", "si te"), None);
    assert_eq!(safe_repo_full_name("ac me", "site"), None);
}

#[test]
fn session_ids_accept_a_uuid_and_reject_a_probe_string() {
    let uuid = "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e";
    assert_eq!(safe_session_id(uuid).as_deref(), Some(uuid));
    assert_eq!(safe_session_id("../../etc/passwd"), None);
    assert_eq!(safe_session_id(&"s".repeat(MAX_SESSION_ID_LEN + 1)), None);
}

/// The `?run=` selector normalizes three "give me the authoritative bundle"
/// spellings onto one recorded value, and drops anything that is not a run id.
#[test]
fn run_selectors_normalize_to_latest_or_a_validated_run_id() {
    for absent in [None, Some(""), Some("   "), Some("latest")] {
        assert_eq!(safe_run_id(absent).as_deref(), Some(RUN_LATEST));
    }
    assert_eq!(safe_run_id(Some("run-42")).as_deref(), Some("run-42"));
    assert_eq!(safe_run_id(Some("run 42")), None);
    assert_eq!(safe_run_id(Some(&"r".repeat(MAX_RUN_ID_LEN + 1))), None);
}

#[test]
fn blob_shas_must_be_hex_within_the_git_object_bound() {
    assert_eq!(safe_blob_sha("deadBEEF01").as_deref(), Some("deadBEEF01"));
    assert!(safe_blob_sha(&"a".repeat(MAX_BLOB_SHA_LEN)).is_some());
    assert_eq!(safe_blob_sha(&"a".repeat(MAX_BLOB_SHA_LEN + 1)), None);
    assert_eq!(safe_blob_sha("../../secret"), None);
    assert_eq!(safe_blob_sha(""), None);
}

/// Branches go through the product's OWN validator, so the record can never
/// describe a branch the request could not have used.
#[test]
fn branches_are_accepted_only_by_the_product_validator() {
    assert_eq!(
        safe_branch("feat/audit-args").as_deref(),
        Some("feat/audit-args")
    );
    assert_eq!(safe_branch(""), None);
    assert_eq!(safe_branch("has space"), None);
    assert_eq!(safe_branch(&"b".repeat(MAX_BRANCH_LEN + 1)), None);
}

#[test]
fn work_labels_are_bounded_and_separator_free() {
    assert_eq!(safe_work_label("fkst:work").as_deref(), Some("fkst:work"));
    assert_eq!(
        safe_work_label("a,b"),
        None,
        "a comma could forge two labels"
    );
    assert_eq!(safe_work_label("a\"b"), None);
    assert_eq!(safe_work_label("a\nb"), None);
    assert_eq!(safe_work_label(&"l".repeat(MAX_WORK_LABEL_LEN + 1)), None);
    assert_eq!(safe_work_label("  "), None);
}

#[test]
fn environment_names_and_locales_use_their_validated_forms() {
    assert_eq!(safe_environment_name("node-20").as_deref(), Some("node-20"));
    assert_eq!(
        safe_environment_name("Node20"),
        None,
        "upper case is not a valid environment name"
    );
    assert_eq!(safe_output_lang("zh-CN").as_deref(), Some("zh-CN"));
    assert_eq!(safe_output_lang("zh CN"), None);
}

/// The media type is normalized and its PARAMETERS are dropped: a `boundary=`
/// value is caller-chosen free text, which malformed-input metadata forbids.
#[test]
fn content_types_normalize_and_shed_their_parameters() {
    assert_eq!(
        safe_content_type("Application/JSON; charset=UTF-8").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        safe_content_type("multipart/form-data; boundary=canary-boundary").as_deref(),
        Some("multipart/form-data")
    );
    assert_eq!(safe_content_type(""), None);
    assert_eq!(
        safe_content_type(&"x".repeat(MAX_CONTENT_TYPE_LEN + 1)),
        None
    );
}

#[test]
fn a_bounded_list_keeps_a_prefix_the_true_count_and_the_marker() {
    let values: Vec<String> = (0..5).map(|i| format!("item-{i}")).collect();
    let bounded = bounded_list(values.iter().map(String::as_str), 3, |v| {
        Some(v.to_string())
    });
    assert_eq!(bounded.items, vec!["item-0", "item-1", "item-2"]);
    assert_eq!(bounded.count, 5);
    assert!(bounded.truncated);
}

#[test]
fn a_short_list_is_not_marked_truncated() {
    let bounded = bounded_list(["a", "b"], 10, |v| Some(v.to_string()));
    assert_eq!(bounded.count, 2);
    assert!(!bounded.truncated);
}

/// A rejected entry is dropped from `items` but still counted, so the record
/// describes the request the caller made rather than the subset we could keep.
#[test]
fn rejected_entries_are_counted_but_never_rendered() {
    let bounded = bounded_list(["good", "bad"], 10, |v| {
        (v == "good").then(|| v.to_string())
    });
    assert_eq!(bounded.items, vec!["good"]);
    assert_eq!(bounded.count, 2);
    assert!(
        bounded.truncated,
        "a dropped entry means the rendered list is partial"
    );
}

#[test]
fn byte_len_measures_utf8_bytes_not_characters() {
    assert_eq!(byte_len("abc"), 3);
    assert_eq!(byte_len("héllo"), 6);
    assert_eq!(byte_len(""), 0);
}
