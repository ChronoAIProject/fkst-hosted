//! Tests for [`super`] (the `### Engine Config` allowlist parser/validator).

use super::*;

fn err_message(block: &str) -> String {
    match parse_engine_config(block) {
        Err(AppError::Unprocessable(msg)) => msg,
        other => panic!("expected 422, got {other:?}"),
    }
}

#[test]
fn empty_blank_and_comment_only_sections_are_an_empty_map() {
    assert!(parse_engine_config("").expect("empty").is_empty());
    assert!(parse_engine_config("  \n\n").expect("blank").is_empty());
    // The PRISTINE template shape: an explanatory HTML comment and no values.
    assert!(
        parse_engine_config("<!--\nOptional. One KEY=value per line.\n-->\n")
            .expect("pristine template")
            .is_empty()
    );
}

#[test]
fn every_allowlisted_key_accepts_a_valid_value() {
    let config = parse_engine_config(
        "FKST_CODEX_PERMIT_SLOTS=8\n\
         FKST_QUEUE_CAPACITY=32\n\
         FKST_MAX_IN_FLIGHT_PER_DEPT=4\n\
         FKST_DURABLE_ADMISSION_BURST_PER_DEPT=2\n\
         FKST_RETRY_DEFAULT_MAX_ATTEMPTS=3\n\
         FKST_RETRY_DEFAULT_BASE=30s\n\
         FKST_RETRY_DEFAULT_CAP=10m\n\
         FKST_DEPARTMENT_DEFAULT_STALL_WINDOW=45s\n\
         FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=72h\n\
         FKST_RATE_POOL_GH=10,10\n",
    )
    .expect("all allowlisted keys parse");
    assert_eq!(config.len(), 10);
    assert_eq!(config["FKST_CODEX_PERMIT_SLOTS"], "8");
    assert_eq!(config["FKST_RATE_POOL_GH"], "10,10");
}

#[test]
fn comment_plus_values_parses_the_values() {
    let config = parse_engine_config("<!--\nprose here\n-->\nFKST_CODEX_PERMIT_SLOTS=4\n")
        .expect("comment + value parses");
    assert_eq!(config["FKST_CODEX_PERMIT_SLOTS"], "4");
}

#[test]
fn unsupported_keys_are_422_naming_the_key() {
    for key in [
        "FKST_RUNTIME_ROOT",
        "FKST_DURABLE_ROOT",
        "FKST_GITHUB_TOKEN_FILE",
        "FKST_RATE_POOL_ROOT",
        "PATH",
        "LLM_API_KEY",
    ] {
        let msg = err_message(&format!("{key}=x\n"));
        assert!(msg.contains(key), "{key}: names the key: {msg}");
    }
}

#[test]
fn output_lang_key_points_to_its_own_section() {
    let msg = err_message("FKST_OUTPUT_LANG=zh\n");
    assert!(
        msg.contains("### Output Language"),
        "points at the dedicated section: {msg}"
    );
}

#[test]
fn malformed_lines_and_duplicates_are_422() {
    let msg = err_message("FKST_CODEX_PERMIT_SLOTS\n");
    assert!(msg.contains("KEY=value"), "{msg}");
    let msg = err_message("FKST_CODEX_PERMIT_SLOTS=4\nFKST_CODEX_PERMIT_SLOTS=8\n");
    assert!(msg.contains("more than once"), "{msg}");
}

#[test]
fn integer_keys_enforce_their_bounds_as_u64() {
    // Permit slots: 1..=32 (static cap — see MAX_CODEX_PERMIT_SLOTS).
    assert!(parse_engine_config("FKST_CODEX_PERMIT_SLOTS=32").is_ok());
    for bad in ["0", "33", "-1", "4.5", "18446744073709551616"] {
        let msg = err_message(&format!("FKST_CODEX_PERMIT_SLOTS={bad}"));
        assert!(msg.contains("FKST_CODEX_PERMIT_SLOTS"), "{bad}: {msg}");
    }
    // Queue shape: 1..=1024.
    assert!(parse_engine_config("FKST_QUEUE_CAPACITY=1024").is_ok());
    assert!(parse_engine_config("FKST_QUEUE_CAPACITY=1025").is_err());
    // Attempts: 1..=100.
    assert!(parse_engine_config("FKST_RETRY_DEFAULT_MAX_ATTEMPTS=100").is_ok());
    assert!(parse_engine_config("FKST_RETRY_DEFAULT_MAX_ATTEMPTS=101").is_err());
}

#[test]
fn durations_must_be_fully_shaped_and_bounded_after_normalization() {
    // The engine checks only the suffix; the FULL shape check here prevents the
    // supervisor's expect()-panic on a non-numeric prefix.
    // NOTE: " 5m" is absent — section lines and values are trimmed by design,
    // so leading/trailing whitespace around a valid duration is accepted.
    for bad in ["30", "s", "3.5m", "1d", "5 m", "-5m"] {
        let msg = err_message(&format!("FKST_DEPARTMENT_DEFAULT_STALL_WINDOW={bad}"));
        assert!(
            msg.contains("FKST_DEPARTMENT_DEFAULT_STALL_WINDOW"),
            "{bad:?}: {msg}"
        );
    }
    // Bound applies AFTER normalization: 168h (= 7 days) is the max; 169h and a
    // suffix-blind large-second value both fail.
    assert!(parse_engine_config("FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=168h").is_ok());
    assert!(parse_engine_config("FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=169h").is_err());
    assert!(parse_engine_config("FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=604801s").is_err());
}

#[test]
fn the_retry_pair_is_validated_against_effective_values() {
    // Both set, coherent.
    assert!(parse_engine_config("FKST_RETRY_DEFAULT_BASE=30s\nFKST_RETRY_DEFAULT_CAP=5m").is_ok());
    // Base-only raise above the DEFAULT cap (30m): the engine would refuse to
    // start — must 422 here, naming both keys.
    let msg = err_message("FKST_RETRY_DEFAULT_BASE=1h");
    assert!(msg.contains("FKST_RETRY_DEFAULT_BASE"), "{msg}");
    assert!(msg.contains("FKST_RETRY_DEFAULT_CAP"), "{msg}");
    // Cap-only drop below the DEFAULT base (60s) fails the same way.
    assert!(parse_engine_config("FKST_RETRY_DEFAULT_CAP=30s").is_err());
    // Base-only raise UP TO the default cap is fine (30m >= 30m).
    assert!(parse_engine_config("FKST_RETRY_DEFAULT_BASE=30m").is_ok());
}

#[test]
fn rate_pools_validate_name_and_shape() {
    assert!(parse_engine_config("FKST_RATE_POOL_MY_TOOL2=5,60").is_ok());
    for (bad, needle) in [
        ("FKST_RATE_POOL_gh=5,5", "NAME"),
        ("FKST_RATE_POOL_=5,5", "NAME"),
        ("FKST_RATE_POOL_GH=5", "burst"),
        ("FKST_RATE_POOL_GH=0,5", "burst"),
        ("FKST_RATE_POOL_GH=5,0", "refill"),
        ("FKST_RATE_POOL_GH=a,5", "burst"),
    ] {
        let msg = err_message(bad);
        assert!(msg.contains(needle), "{bad}: expected {needle} in: {msg}");
    }
}
