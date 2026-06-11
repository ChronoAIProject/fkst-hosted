//! `RAISED:` stdout line parsing and the canonical identity projection.
//!
//! The engine emits progress signals as `RAISED: <base64-json>` lines on its
//! stdout. This module owns the robust decode path (multiple base64
//! alphabets, lossy UTF-8, an oversize cap) and the deterministic identity
//! derivation (`canonical_event_identity` / `canonical_json`) that the
//! content-addressed `idem_key` is built from. Everything here is pure: no
//! I/O, no logging side effects beyond what callers do with the outcome.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;

/// Maximum byte length of any logged/journaled payload excerpt. Excerpts are
/// truncated to this length at a char boundary so a hostile payload can never
/// flood logs or journal documents.
pub const EXCERPT_MAX_BYTES: usize = 512;

/// The fixed sentinel contributed by a missing/`null` identity pointer, so
/// presence vs absence is itself part of identity (spec: `"\u{0}"`).
const ABSENT_SENTINEL: char = '\u{0}';

/// ASCII Unit Separator joining identity-projection parts (never appears in
/// JSON pointers or canonical JSON text as a raw byte).
const US: char = '\u{1f}';

/// Outcome of parsing one engine stdout line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLine {
    /// A well-formed `RAISED: <b64-json>` line: the decoded event envelope.
    Raised { event_json: serde_json::Value },
    /// A `RAISED:` line whose payload could not be decoded (bad base64 in
    /// every attempted alphabet, valid base64 but non-JSON, or an oversized
    /// line truncated past usefulness). `excerpt` is capped at
    /// [`EXCERPT_MAX_BYTES`]; `oversize` is true when the line exceeded the
    /// configured cap.
    Malformed { excerpt: String, oversize: bool },
    /// Non-RAISED engine chatter (forwarded only to debug logging).
    Other { excerpt: String },
}

/// Parse one raw stdout line (without its trailing `\n`).
///
/// - The line is capped at `max_line_bytes`: longer input is truncated for
///   parsing and, when it carries the `RAISED:` prefix, treated as
///   `Malformed { oversize: true }` (a truncated base64 payload can never be
///   trusted). Oversized non-RAISED chatter stays `Other`.
/// - Invalid UTF-8 is replaced lossily before prefix matching; a RAISED
///   payload containing the invalid bytes fails base64 decode downstream and
///   surfaces as `Malformed` — never a panic.
pub fn parse_raised_line(raw: &[u8], max_line_bytes: usize) -> ParsedLine {
    let oversize = raw.len() > max_line_bytes;
    let bounded = if oversize {
        &raw[..max_line_bytes]
    } else {
        raw
    };
    let text = String::from_utf8_lossy(bounded);
    let trimmed = text.trim_end();

    let Some(rest) = trimmed.strip_prefix("RAISED:") else {
        return ParsedLine::Other {
            excerpt: truncate_excerpt(trimmed),
        };
    };
    let payload = rest.trim();
    if oversize {
        // A truncated base64 payload is unusable by construction; declare it
        // malformed without attempting a decode that could "succeed" on a
        // prefix and journal a wrong event.
        return ParsedLine::Malformed {
            excerpt: truncate_excerpt(payload),
            oversize: true,
        };
    }

    let Some(bytes) = decode_any_alphabet(payload) else {
        return ParsedLine::Malformed {
            excerpt: truncate_excerpt(payload),
            oversize: false,
        };
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(event_json) => ParsedLine::Raised { event_json },
        Err(_) => ParsedLine::Malformed {
            excerpt: truncate_excerpt(payload),
            oversize: false,
        },
    }
}

/// Try every accepted base64 alphabet in order: STANDARD, STANDARD_NO_PAD,
/// URL_SAFE, URL_SAFE_NO_PAD. First success wins; `None` when all fail.
fn decode_any_alphabet(payload: &str) -> Option<Vec<u8>> {
    STANDARD
        .decode(payload)
        .or_else(|_| STANDARD_NO_PAD.decode(payload))
        .or_else(|_| URL_SAFE.decode(payload))
        .or_else(|_| URL_SAFE_NO_PAD.decode(payload))
        .ok()
}

/// Truncate `text` to [`EXCERPT_MAX_BYTES`] at a char boundary.
pub fn truncate_excerpt(text: &str) -> String {
    if text.len() <= EXCERPT_MAX_BYTES {
        return text.to_string();
    }
    let mut end = EXCERPT_MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Recursively canonical JSON: object keys sorted lexicographically, no
/// whitespace, numbers in serde_json's canonical display form (documented as
/// best-effort for non-integer floats, which the engine envelope is not
/// expected to use). Two structurally-equal values always serialize to the
/// same string regardless of original key order.
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        leaf => leaf.to_string(),
    }
}

/// Deterministic identity string for an event envelope, built from the
/// configured JSON-pointer projection.
///
/// For each pointer: `pointer || US || canonical_value(resolved)`, joined by
/// `US`. A missing or `null` pointer contributes the fixed `\u{0}` sentinel
/// instead of a value, so two different absence patterns never collide.
/// When EVERY pointer resolves to missing/null (or the pointer list is
/// empty), the fallback is `canonical_json(event_json)` — a non-degenerate
/// key even before the envelope shape is pinned.
pub fn canonical_event_identity(event_json: &serde_json::Value, pointers: &[String]) -> String {
    let mut any_present = false;
    let mut parts = Vec::with_capacity(pointers.len());
    for pointer in pointers {
        match event_json.pointer(pointer) {
            Some(value) if !value.is_null() => {
                any_present = true;
                parts.push(format!("{pointer}{US}{}", canonical_json(value)));
            }
            _ => parts.push(format!("{pointer}{US}{ABSENT_SENTINEL}")),
        }
    }
    if !any_present {
        return canonical_json(event_json);
    }
    parts.join(&US.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const CAP: usize = 1_048_576;

    fn pointers(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    fn b64(engine: &impl Engine, payload: &str) -> String {
        engine.encode(payload)
    }

    // ---- parse_raised_line ------------------------------------------------

    #[test]
    fn valid_standard_base64_line_parses_to_the_decoded_json() {
        let payload = r#"{"department":"hello","name":"tick"}"#;
        let line = format!("RAISED: {}", b64(&STANDARD, payload));
        match parse_raised_line(line.as_bytes(), CAP) {
            ParsedLine::Raised { event_json } => {
                assert_eq!(event_json, json!({"department": "hello", "name": "tick"}));
            }
            other => panic!("expected Raised, got {other:?}"),
        }
    }

    #[test]
    fn all_base64_alphabets_are_accepted() {
        // Payload chosen so the standard alphabet emits both '+' and '/'
        // characters AND padding, exercising every fallback distinctly.
        let payload = r#"{"name":"ûÿ?>xy"}"#;
        let value: serde_json::Value = serde_json::from_str(payload).expect("payload json");
        let standard = b64(&STANDARD, payload);
        assert!(standard.contains('='), "fixture must exercise padding");
        for encoded in [
            standard,
            b64(&STANDARD_NO_PAD, payload),
            b64(&URL_SAFE, payload),
            b64(&URL_SAFE_NO_PAD, payload),
        ] {
            let line = format!("RAISED: {encoded}");
            match parse_raised_line(line.as_bytes(), CAP) {
                ParsedLine::Raised { event_json } => assert_eq!(event_json, value),
                other => panic!("alphabet {encoded:?} not accepted: {other:?}"),
            }
        }
    }

    #[test]
    fn crlf_and_trailing_whitespace_are_stripped() {
        let line = format!("RAISED: {} \r", b64(&STANDARD, r#"{"a":1}"#));
        assert!(matches!(
            parse_raised_line(line.as_bytes(), CAP),
            ParsedLine::Raised { .. }
        ));
    }

    #[test]
    fn non_raised_lines_are_other() {
        match parse_raised_line(b"INFO consumer started dept=hello", CAP) {
            ParsedLine::Other { excerpt } => {
                assert_eq!(excerpt, "INFO consumer started dept=hello");
            }
            other => panic!("expected Other, got {other:?}"),
        }
        // Prefix must be exact: a mid-line RAISED is chatter, not a signal.
        assert!(matches!(
            parse_raised_line(b"note: RAISED: abc", CAP),
            ParsedLine::Other { .. }
        ));
    }

    #[test]
    fn bad_base64_in_every_alphabet_is_malformed() {
        match parse_raised_line(b"RAISED: !!!not-base64!!!", CAP) {
            ParsedLine::Malformed { excerpt, oversize } => {
                assert!(!oversize);
                assert_eq!(excerpt, "!!!not-base64!!!");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn valid_base64_but_non_json_is_malformed() {
        let line = format!("RAISED: {}", b64(&STANDARD, "not json at all"));
        assert!(matches!(
            parse_raised_line(line.as_bytes(), CAP),
            ParsedLine::Malformed {
                oversize: false,
                ..
            }
        ));
    }

    #[test]
    fn oversized_raised_line_is_truncated_and_malformed_without_panic() {
        let huge = format!("RAISED: {}", "A".repeat(2 * CAP));
        match parse_raised_line(huge.as_bytes(), CAP) {
            ParsedLine::Malformed { excerpt, oversize } => {
                assert!(oversize, "oversize flag must be set");
                assert!(excerpt.len() <= EXCERPT_MAX_BYTES);
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn oversized_non_raised_chatter_stays_other() {
        let huge = "x".repeat(2 * CAP);
        assert!(matches!(
            parse_raised_line(huge.as_bytes(), CAP),
            ParsedLine::Other { .. }
        ));
    }

    #[test]
    fn invalid_utf8_is_replaced_lossily_and_never_panics() {
        // Invalid bytes inside a RAISED payload: lossy replacement corrupts
        // the base64 -> Malformed, no panic.
        let mut line = b"RAISED: ".to_vec();
        line.extend_from_slice(&[0xff, 0xfe, 0xfd]);
        assert!(matches!(
            parse_raised_line(&line, CAP),
            ParsedLine::Malformed { .. }
        ));
        // Invalid bytes in plain chatter: Other, no panic.
        assert!(matches!(
            parse_raised_line(&[0xff, 0x20, 0xff], CAP),
            ParsedLine::Other { .. }
        ));
    }

    #[test]
    fn malformed_excerpt_is_capped_at_512_bytes_on_a_char_boundary() {
        // 600 two-byte chars: the 512-byte cap lands mid-char and must back
        // up to a boundary.
        let payload = "α".repeat(600);
        let line = format!("RAISED: {payload}");
        match parse_raised_line(line.as_bytes(), CAP) {
            ParsedLine::Malformed { excerpt, .. } => {
                assert!(excerpt.len() <= EXCERPT_MAX_BYTES);
                assert!(excerpt.chars().all(|c| c == 'α'));
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    // ---- canonical_json -----------------------------------------------------

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a: serde_json::Value =
            serde_json::from_str(r#"{"b":2,"a":1,"c":{"y":0,"x":[1,2]}}"#).expect("fixture a");
        let b: serde_json::Value =
            serde_json::from_str(r#"{"c":{"x":[1,2],"y":0},"a":1,"b":2}"#).expect("fixture b");
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(canonical_json(&a), r#"{"a":1,"b":2,"c":{"x":[1,2],"y":0}}"#);
    }

    #[test]
    fn canonical_json_strips_whitespace_and_preserves_arrays() {
        let value: serde_json::Value =
            serde_json::from_str("[ 1 , 2 , { \"k\" : \"v\" } ]").expect("fixture");
        assert_eq!(canonical_json(&value), r#"[1,2,{"k":"v"}]"#);
    }

    // ---- canonical_event_identity ------------------------------------------

    #[test]
    fn identity_projection_uses_the_configured_pointers() {
        let event = json!({"department":"d","source":"s","name":"n","corr":"c","extra":"x"});
        let id_a = canonical_event_identity(&event, &pointers(&["/department", "/name"]));
        let id_b = canonical_event_identity(&event, &pointers(&["/department", "/name"]));
        assert_eq!(id_a, id_b, "deterministic across calls");
        // The non-projected fields do not participate.
        let other = json!({"department":"d","source":"DIFFERENT","name":"n","corr":"c"});
        assert_eq!(
            id_a,
            canonical_event_identity(&other, &pointers(&["/department", "/name"]))
        );
    }

    #[test]
    fn missing_and_null_pointers_use_the_sentinel_and_stay_distinct() {
        let with_null = json!({"department":"d","source":null});
        let without = json!({"department":"d"});
        let ptrs = pointers(&["/department", "/source"]);
        // Missing and explicit null collapse to the same identity (both are
        // "absent"); a present value differs.
        assert_eq!(
            canonical_event_identity(&with_null, &ptrs),
            canonical_event_identity(&without, &ptrs)
        );
        let present = json!({"department":"d","source":"s"});
        assert_ne!(
            canonical_event_identity(&present, &ptrs),
            canonical_event_identity(&without, &ptrs)
        );
    }

    #[test]
    fn different_absence_patterns_never_collide() {
        let only_a = json!({"a":"v"});
        let only_b = json!({"b":"v"});
        let ptrs = pointers(&["/a", "/b"]);
        assert_ne!(
            canonical_event_identity(&only_a, &ptrs),
            canonical_event_identity(&only_b, &ptrs)
        );
    }

    #[test]
    fn all_missing_pointers_fall_back_to_canonical_json() {
        let event = json!({"unrelated": {"deep": [1, 2, 3]}});
        let ptrs = pointers(&["/department", "/source", "/name", "/corr"]);
        assert_eq!(
            canonical_event_identity(&event, &ptrs),
            canonical_json(&event)
        );
        // Stable and non-degenerate: two different envelopes differ.
        let other = json!({"unrelated": {"deep": [1, 2, 4]}});
        assert_ne!(
            canonical_event_identity(&event, &ptrs),
            canonical_event_identity(&other, &ptrs)
        );
    }

    #[test]
    fn empty_pointer_list_falls_back_to_canonical_json() {
        let event = json!({"a":1});
        assert_eq!(
            canonical_event_identity(&event, &[]),
            canonical_json(&event)
        );
    }
}
