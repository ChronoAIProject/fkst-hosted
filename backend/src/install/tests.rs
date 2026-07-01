//! Unit tests for the shared install-command runner + `validate-env` core.
//!
//! Cover both success AND failure paths: all-pass counting, first-failure
//! short-circuit (proved via an absent side-effect file), env-variable
//! injection, stderr-tail truncation, deadline timeout, the exact verdict-frame
//! JSON for both variants, and the `validate-env` spec-driven paths (ok, failed,
//! unreadable). Each filesystem test uses its own unique temp dir.

use super::*;

use std::collections::BTreeMap;
use std::time::Duration;

#[tokio::test]
async fn all_commands_succeed_counts_them() {
    let cmds = vec![
        "true".to_string(),
        "echo hi".to_string(),
        "true".to_string(),
    ];
    let verdict = run_ordered(&cmds, &BTreeMap::new(), Duration::from_secs(30), 4096).await;
    assert_eq!(verdict, Verdict::Ok { commands: 3 });
}

#[tokio::test]
async fn failing_command_short_circuits_the_sequence() {
    // The 3rd command would create `sentinel`; because the 2nd (`false`) fails,
    // the 3rd must never run, so the file must be absent afterwards.
    let dir = tempfile::tempdir().expect("temp dir");
    let sentinel = dir.path().join("should-not-exist");
    let cmds = vec![
        "true".to_string(),
        "false".to_string(),
        format!("touch \"{}\"", sentinel.display()),
    ];

    let verdict = run_ordered(&cmds, &BTreeMap::new(), Duration::from_secs(30), 4096).await;

    match verdict {
        Verdict::Failed {
            index,
            exit_code,
            timed_out,
            ..
        } => {
            assert_eq!(index, 2, "second command is the one that failed");
            assert_eq!(exit_code, 1, "`false` exits 1");
            assert!(!timed_out);
        }
        other => panic!("expected a failure verdict, got {other:?}"),
    }
    assert!(
        !sentinel.exists(),
        "the command after the failure must not have run"
    );
}

#[tokio::test]
async fn injected_variables_reach_the_command() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("out.txt");
    let mut vars = BTreeMap::new();
    vars.insert("VALIDATE_TEST_FOO".to_string(), "hello-42".to_string());
    let cmds = vec![format!(
        "printf '%s' \"$VALIDATE_TEST_FOO\" > \"{}\"",
        out.display()
    )];

    let verdict = run_ordered(&cmds, &vars, Duration::from_secs(30), 4096).await;

    assert_eq!(verdict, Verdict::Ok { commands: 1 });
    let contents = std::fs::read_to_string(&out).expect("read output");
    assert_eq!(
        contents, "hello-42",
        "the injected variable must be visible"
    );
}

#[tokio::test]
async fn stderr_tail_is_truncated_to_the_cap() {
    // Emit 1000 bytes of stderr then fail; only the last 10 must survive.
    let cmds = vec!["head -c 1000 /dev/zero | tr '\\0' 'X' >&2; exit 3".to_string()];

    let verdict = run_ordered(&cmds, &BTreeMap::new(), Duration::from_secs(30), 10).await;

    match verdict {
        Verdict::Failed {
            index,
            exit_code,
            timed_out,
            stderr_tail,
            ..
        } => {
            assert_eq!(index, 1);
            assert_eq!(exit_code, 3);
            assert!(!timed_out);
            assert_eq!(stderr_tail.len(), 10, "tail must be capped to 10 bytes");
            assert_eq!(stderr_tail, "XXXXXXXXXX");
        }
        other => panic!("expected a failure verdict, got {other:?}"),
    }
}

#[tokio::test]
async fn command_exceeding_deadline_times_out() {
    let cmds = vec!["sleep 5".to_string()];

    let verdict = run_ordered(&cmds, &BTreeMap::new(), Duration::from_millis(50), 4096).await;

    match verdict {
        Verdict::Failed {
            index,
            exit_code,
            timed_out,
            ..
        } => {
            assert_eq!(index, 1);
            assert_eq!(exit_code, -1);
            assert!(timed_out, "the command outran the deadline");
        }
        other => panic!("expected a timeout verdict, got {other:?}"),
    }
}

#[test]
fn verdict_frame_ok_is_exact_json() {
    let frame = verdict_frame(&Verdict::Ok { commands: 3 });
    assert_eq!(frame, r#"{"status":"ok","commands":3}"#);

    let parsed: serde_json::Value = serde_json::from_str(&frame).expect("valid json");
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["commands"], 3);
}

#[test]
fn verdict_frame_failed_is_exact_json_and_escapes() {
    // Embedded quotes prove serde_json escaping is applied.
    let verdict = Verdict::Failed {
        index: 2,
        command: "echo \"hi\"".to_string(),
        exit_code: 7,
        timed_out: false,
        stderr_tail: "he said \"boom\"".to_string(),
    };
    let frame = verdict_frame(&verdict);
    assert_eq!(
        frame,
        r#"{"status":"failed","index":2,"command":"echo \"hi\"","exit_code":7,"timed_out":false,"stderr_tail":"he said \"boom\""}"#
    );

    let parsed: serde_json::Value = serde_json::from_str(&frame).expect("valid json");
    assert_eq!(parsed["status"], "failed");
    assert_eq!(parsed["index"], 2);
    assert_eq!(parsed["command"], "echo \"hi\"");
    assert_eq!(parsed["exit_code"], 7);
    assert_eq!(parsed["timed_out"], false);
    assert_eq!(parsed["stderr_tail"], "he said \"boom\"");
}

#[tokio::test]
async fn validate_spec_success_emits_ok_frame() {
    let dir = tempfile::tempdir().expect("temp dir");
    let spec = dir.path().join("validate-spec.json");
    std::fs::write(
        &spec,
        r#"{"install":["true","echo hi"],"variables":{"A":"b"},"deadline_secs":30}"#,
    )
    .expect("write spec");

    let (frame, success) = run_validate_spec_at(&spec, 4096).await;

    assert!(success);
    assert_eq!(frame, r#"{"status":"ok","commands":2}"#);
}

#[tokio::test]
async fn validate_spec_failure_emits_failed_frame() {
    let dir = tempfile::tempdir().expect("temp dir");
    let spec = dir.path().join("validate-spec.json");
    std::fs::write(
        &spec,
        r#"{"install":["false"],"variables":{},"deadline_secs":30}"#,
    )
    .expect("write spec");

    let (frame, success) = run_validate_spec_at(&spec, 4096).await;

    assert!(!success);
    let parsed: serde_json::Value = serde_json::from_str(&frame).expect("valid json");
    assert_eq!(parsed["status"], "failed");
    assert_eq!(parsed["index"], 1);
    assert_eq!(parsed["exit_code"], 1);
    assert_eq!(parsed["timed_out"], false);
}

#[tokio::test]
async fn validate_spec_unreadable_emits_index_zero_frame() {
    let path = Path::new("/no/such/fkst/validate-spec.json");

    let (frame, success) = run_validate_spec_at(path, 4096).await;

    assert!(!success);
    let parsed: serde_json::Value = serde_json::from_str(&frame).expect("valid json");
    assert_eq!(parsed["status"], "failed");
    assert_eq!(parsed["index"], 0);
    assert_eq!(parsed["command"], "");
    assert_eq!(parsed["exit_code"], -1);
    assert_eq!(parsed["timed_out"], false);
    assert!(
        parsed["stderr_tail"]
            .as_str()
            .expect("stderr_tail is a string")
            .contains("could not read validate spec"),
        "read-failure frame must explain the failure: {frame}"
    );
}
