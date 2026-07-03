//! Tests for redactor seeding from the mounted creds dir. Split into a sibling file
//! so `seed.rs` stays under the 500-line module cap.

use super::*;

use crate::session_spec::creds::{GITHUB_TOKEN_FILE, LLM_API_KEY_FILE};

/// Materialize a creds dir with the given files and return its layout.
fn creds_with(files: &[(&str, &str)]) -> (tempfile::TempDir, CredsLayout) {
    let dir = tempfile::tempdir().expect("dir");
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).expect("write cred");
    }
    let layout = CredsLayout::new(dir.path());
    (dir, layout)
}

#[test]
fn read_github_token_extracts_the_json_token_field() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("github-token");
    std::fs::write(
        &path,
        r#"{"token":"ghs_realtokenvalue","expires_at":"2999-01-01T00:00:00Z"}"#,
    )
    .expect("write");
    assert_eq!(
        read_github_token(&path),
        Some("ghs_realtokenvalue".to_string())
    );
}

#[test]
fn read_github_token_falls_back_to_raw_on_non_json() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("github-token");
    std::fs::write(&path, "ghs_plain_token\n").expect("write");
    assert_eq!(
        read_github_token(&path),
        Some("ghs_plain_token".to_string())
    );
}

#[test]
fn read_github_token_is_none_when_absent_or_blank() {
    let dir = tempfile::tempdir().expect("dir");
    assert_eq!(read_github_token(&dir.path().join("missing")), None);
    let blank = dir.path().join("blank");
    std::fs::write(&blank, "   \n").expect("write");
    assert_eq!(read_github_token(&blank), None);
}

#[test]
fn seed_secrets_builds_the_full_known_secret_table() {
    let (_dir, creds) = creds_with(&[
        (
            GITHUB_TOKEN_FILE,
            r#"{"token":"ghs_tok","expires_at":"2999-01-01T00:00:00Z"}"#,
        ),
        (LLM_API_KEY_FILE, "sk-llmkey\n"),
        ("userenv.API_TOKEN", "user-secret-123\n"),
        ("userenv.FOO", "foo-val"),
    ]);

    let mut secrets = seed_secrets(&creds);
    secrets.sort();

    assert!(secrets.contains(&("github-token".to_string(), "ghs_tok".to_string())));
    assert!(secrets.contains(&("llm-key".to_string(), "sk-llmkey".to_string())));
    assert!(secrets.contains(&(
        "userenv:API_TOKEN".to_string(),
        "user-secret-123".to_string()
    )));
    assert!(secrets.contains(&("userenv:FOO".to_string(), "foo-val".to_string())));
    assert_eq!(
        secrets.len(),
        4,
        "exactly the four seeded secrets: {secrets:?}"
    );
}

#[test]
fn seed_secrets_skips_missing_files() {
    // Only the LLM key present; no token, no user env.
    let (_dir, creds) = creds_with(&[(LLM_API_KEY_FILE, "sk-only\n")]);
    let secrets = seed_secrets(&creds);
    assert_eq!(
        secrets,
        vec![("llm-key".to_string(), "sk-only".to_string())]
    );
}

#[test]
fn seed_secrets_on_empty_dir_is_empty() {
    let dir = tempfile::tempdir().expect("dir");
    let creds = CredsLayout::new(dir.path());
    assert!(seed_secrets(&creds).is_empty());
}
