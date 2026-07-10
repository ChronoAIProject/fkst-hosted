//! Tests for the shared env-validation verdict parsing (relocated verbatim from
//! `k8s/validation_tests.rs`). These pin the pure, cluster-free pieces every backend
//! shares: verdict parsing, last-line selection, and the two conservative-`Failed`
//! builders that guarantee a timed-out / unparseable run yields a byte-identical
//! verdict in either backend.

use super::{last_non_empty_line, parse_verdict_line, verdict_timed_out, verdict_unparseable};
use crate::session_backend::ValidationOutcome;

#[test]
fn parse_verdict_line_reads_the_ok_frame() {
    let outcome = parse_verdict_line(r#"{"status":"ok","commands":3}"#).expect("ok parses");
    assert_eq!(outcome, ValidationOutcome::Passed { commands: 3 });
}

#[test]
fn parse_verdict_line_reads_the_failed_frame_with_every_field() {
    let line = r#"{"status":"failed","index":2,"command":"pip install x","exit_code":1,"timed_out":false,"stderr_tail":"boom"}"#;
    let outcome = parse_verdict_line(line).expect("failed parses");
    assert_eq!(
        outcome,
        ValidationOutcome::Failed {
            failed_command_index: 2,
            failed_command: "pip install x".to_string(),
            exit_code: 1,
            timed_out: false,
            stderr_tail: "boom".to_string(),
        }
    );
}

#[test]
fn parse_verdict_line_reads_a_timed_out_failed_frame() {
    let line = r#"{"status":"failed","index":1,"command":"sleep 999","exit_code":-1,"timed_out":true,"stderr_tail":""}"#;
    let outcome = parse_verdict_line(line).expect("failed parses");
    assert_eq!(
        outcome,
        ValidationOutcome::Failed {
            failed_command_index: 1,
            failed_command: "sleep 999".to_string(),
            exit_code: -1,
            timed_out: true,
            stderr_tail: String::new(),
        }
    );
}

#[test]
fn parse_verdict_line_rejects_non_json_and_empty_lines() {
    assert!(parse_verdict_line("not json at all").is_none());
    assert!(parse_verdict_line("").is_none());
    assert!(parse_verdict_line("   ").is_none());
    // Recognized JSON but an unknown status is not a verdict.
    assert!(parse_verdict_line(r#"{"status":"weird"}"#).is_none());
    // `ok` without a command count is incomplete → not a verdict.
    assert!(parse_verdict_line(r#"{"status":"ok"}"#).is_none());
    // `failed` without an index is incomplete → not a verdict.
    assert!(parse_verdict_line(r#"{"status":"failed","command":"x"}"#).is_none());
}

#[test]
fn parse_verdict_line_tolerates_surrounding_whitespace() {
    let outcome =
        parse_verdict_line("  {\"status\":\"ok\",\"commands\":1}  \n").expect("trimmed parses");
    assert_eq!(outcome, ValidationOutcome::Passed { commands: 1 });
}

#[test]
fn last_non_empty_line_picks_the_final_frame_ignoring_prior_chatter() {
    let logs = "starting up\n{\"status\":\"noise\"}\n\n{\"status\":\"ok\",\"commands\":2}\n";
    let last = last_non_empty_line(logs).expect("has a last line");
    assert_eq!(last, r#"{"status":"ok","commands":2}"#);
    // And it composes with the parser to yield the real verdict.
    assert_eq!(
        last_non_empty_line(logs).and_then(parse_verdict_line),
        Some(ValidationOutcome::Passed { commands: 2 })
    );
}

#[test]
fn last_non_empty_line_skips_trailing_blank_lines() {
    let logs = "{\"status\":\"ok\",\"commands\":5}\n\n   \n";
    assert_eq!(
        last_non_empty_line(logs),
        Some(r#"{"status":"ok","commands":5}"#)
    );
}

#[test]
fn last_non_empty_line_is_none_for_all_blank_input() {
    assert!(last_non_empty_line("").is_none());
    assert!(last_non_empty_line("\n\n   \n\t\n").is_none());
}

#[test]
fn conservative_builders_are_the_shared_failed_verdicts() {
    // A deadline elapse → timed_out Failed with the shared detail sentence.
    assert_eq!(
        verdict_timed_out(),
        ValidationOutcome::Failed {
            failed_command_index: 0,
            failed_command: String::new(),
            exit_code: -1,
            timed_out: true,
            stderr_tail: "validation pod did not complete before the deadline".to_string(),
        }
    );
    // A readable-but-unparseable run → non-timed-out Failed with the shared sentence.
    assert_eq!(
        verdict_unparseable(),
        ValidationOutcome::Failed {
            failed_command_index: 0,
            failed_command: String::new(),
            exit_code: -1,
            timed_out: false,
            stderr_tail: "validation pod exceeded its limits".to_string(),
        }
    );
}
