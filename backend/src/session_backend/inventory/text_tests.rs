//! The bounding + redaction gate, including the secret canaries the epic requires
//! never to reach an inventory DTO.

use super::*;

#[test]
fn an_absent_or_blank_message_is_none() {
    assert_eq!(bounded_operational_text(None, 128), None);
    assert_eq!(bounded_operational_text(Some(""), 128), None);
    assert_eq!(bounded_operational_text(Some("   \n\t "), 128), None);
}

#[test]
fn an_ordinary_reason_survives_intact() {
    assert_eq!(
        bounded_operational_text(Some("CrashLoopBackOff"), MAX_STATUS_REASON_BYTES),
        Some("CrashLoopBackOff".to_string())
    );
}

#[test]
fn newlines_and_control_characters_are_flattened() {
    // A multi-line backend message must not be able to forge a second log record
    // or a second UI row.
    let out = bounded_operational_text(
        Some("failed to pull\nimage\r\n\tretrying\u{7}"),
        MAX_STATUS_MESSAGE_BYTES,
    )
    .expect("text");
    assert_eq!(out, "failed to pull image retrying");
    assert!(!out.contains('\n') && !out.contains('\r') && !out.contains('\t'));
}

#[test]
fn url_userinfo_and_query_material_are_stripped() {
    let out = bounded_operational_text(
        Some("pull failed from https://bob:hunter2@registry.example.com/img?token=abc#frag"),
        MAX_STATUS_MESSAGE_BYTES,
    )
    .expect("text");
    assert!(!out.contains("hunter2"), "{out}");
    assert!(!out.contains("token=abc"), "{out}");
    assert!(!out.contains("#frag"), "{out}");
    // The operationally useful host + path survive.
    assert!(out.contains("registry.example.com/img"), "{out}");
}

#[test]
fn a_bare_user_password_at_host_is_masked_but_an_email_is_not() {
    let masked =
        bounded_operational_text(Some("creds root:s3cret@db.internal"), 512).expect("text");
    assert!(!masked.contains("s3cret"), "{masked}");
    assert!(masked.contains("db.internal"), "{masked}");

    let email = bounded_operational_text(Some("owner alice@example.com"), 512).expect("text");
    assert!(email.contains("alice@example.com"), "{email}");
}

#[test]
fn secret_canaries_never_survive_the_gate() {
    let canaries = [
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789AB",
        "sk-abcdefghijklmnopqrstuvwxyz0123",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        "password=hunter2",
        "Authorization: Bearer sometokenvalue",
    ];
    for canary in canaries {
        let out = bounded_operational_text(
            Some(&format!("backend said {canary} while pulling")),
            MAX_STATUS_MESSAGE_BYTES,
        )
        .expect("text");
        assert!(out.contains("«REDACTED:"), "no mask for {canary}: {out}");
        assert!(!out.contains("hunter2"), "{out}");
    }
}

#[test]
fn a_long_message_is_truncated_within_its_budget_and_marked() {
    let long = "x".repeat(4000);
    let out = bounded_operational_text(Some(&long), MAX_STATUS_MESSAGE_BYTES).expect("text");
    assert!(out.len() <= MAX_STATUS_MESSAGE_BYTES, "{}", out.len());
    assert!(out.ends_with('…'), "truncation must be visible");
}

#[test]
fn a_reason_is_bounded_more_tightly_than_a_message() {
    let long = "y".repeat(1000);
    let reason = bounded_operational_text(Some(&long), MAX_STATUS_REASON_BYTES).expect("reason");
    let message = bounded_operational_text(Some(&long), MAX_STATUS_MESSAGE_BYTES).expect("message");
    assert!(reason.len() <= MAX_STATUS_REASON_BYTES);
    assert!(message.len() <= MAX_STATUS_MESSAGE_BYTES);
    assert!(reason.len() < message.len());
}

#[test]
fn truncation_never_splits_a_multibyte_character() {
    // Every char is 4 bytes, so a naive byte cut would produce invalid UTF-8.
    let emoji = "🚀".repeat(200);
    let out = bounded_operational_text(Some(&emoji), 64).expect("text");
    assert!(out.len() <= 64, "{}", out.len());
    // Reaching this point at all proves the slice stayed on a char boundary.
    assert!(out.starts_with('🚀'));
}

#[test]
fn a_budget_smaller_than_the_marker_still_respects_the_bound() {
    let out = bounded_operational_text(Some("abcdef"), 2).expect("text");
    assert!(out.len() <= 2, "{out}");
}

#[test]
fn the_raw_status_budget_bounds_a_hostile_state_value() {
    let hostile = "State".repeat(100);
    let out = bounded_operational_text(Some(&hostile), MAX_RAW_STATUS_BYTES).expect("text");
    assert!(out.len() <= MAX_RAW_STATUS_BYTES);
}

#[test]
fn a_message_reduced_to_nothing_reports_none_rather_than_an_empty_string() {
    // Control characters only: the backend said nothing meaningful, and an empty
    // string in the DTO would imply it did.
    assert_eq!(bounded_operational_text(Some("\u{0}\u{1}\u{2}"), 128), None);
}
