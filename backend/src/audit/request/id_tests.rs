//! Unit tests for `X-Request-Id` acceptance and generation.

use super::*;

#[test]
fn accepts_a_well_formed_client_value_verbatim() {
    for value in [
        "0f5b9a41-7d2e-4c8b-9c31-6b0a2f7751d4",
        "req_123",
        "trace.42:1",
        "A",
    ] {
        let normalized = normalize_request_id(Some(value));
        assert_eq!(normalized.value, value, "{value} must be propagated as-is");
        assert!(!normalized.generated);
    }
}

#[test]
fn replaces_a_missing_value_with_a_generated_uuid() {
    let normalized = normalize_request_id(None);
    assert!(normalized.generated);
    assert!(is_acceptable(&normalized.value));
    assert_eq!(normalized.value.len(), 36, "{}", normalized.value);
}

#[test]
fn replaces_every_unsafe_or_oversized_value() {
    let too_long = "a".repeat(MAX_REQUEST_ID_LEN + 1);
    for hostile in [
        "",
        " ",
        "has space",
        "new\nline",
        "semi;colon",
        "quote\"d",
        "sl/ash",
        "unicode-é",
        "null\0byte",
        too_long.as_str(),
    ] {
        let normalized = normalize_request_id(Some(hostile));
        assert!(
            normalized.generated,
            "{hostile:?} must be rejected and replaced"
        );
        assert_ne!(normalized.value, hostile);
        assert!(is_acceptable(&normalized.value));
    }
}

#[test]
fn a_value_at_the_length_limit_is_still_accepted() {
    let exact = "b".repeat(MAX_REQUEST_ID_LEN);
    let normalized = normalize_request_id(Some(&exact));
    assert!(!normalized.generated);
    assert_eq!(normalized.value, exact);
}

#[test]
fn generated_ids_are_unique_per_call() {
    let first = generate_request_id();
    let second = generate_request_id();
    assert_ne!(first, second);
}
