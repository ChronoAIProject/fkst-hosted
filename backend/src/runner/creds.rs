//! Credential-file readers for the `run-session` pod.
//!
//! The per-session Kubernetes Secret is mounted as a 0400 file volume described
//! by [`crate::session_spec::CredsLayout`]. [`read_required_secret`] reads a
//! credential file, trimming the trailing newline a `kubectl`/Secret write
//! leaves behind, and keeps the secret value inside [`SecretString`] so a caller
//! cannot accidentally log it. Values are NEVER logged here — only the path
//! (which is non-secret) appears in error context.

use std::path::Path;

use secrecy::SecretString;

/// Read a REQUIRED credential file into a [`SecretString`].
///
/// Fails (with a non-secret, path-only message) when the file is missing,
/// unreadable, or empty after trimming — an empty installation token must abort
/// the session loudly rather than spawn an engine that cannot authenticate.
pub fn read_required_secret(path: &Path) -> Result<SecretString, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read credential file {}: {error}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("credential file {} is empty", path.display()));
    }
    Ok(SecretString::from(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn required_secret_trims_trailing_newline() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("github-token");
        std::fs::write(&path, "ghs_abc123\n").expect("write");
        let secret = read_required_secret(&path).expect("present token");
        assert_eq!(secret.expose_secret(), "ghs_abc123");
    }

    #[test]
    fn required_secret_missing_file_is_an_error() {
        let dir = tempfile::tempdir().expect("dir");
        let err = read_required_secret(&dir.path().join("absent")).expect_err("missing must fail");
        assert!(err.contains("read credential file"), "{err}");
    }

    #[test]
    fn required_secret_empty_file_is_an_error() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("github-token");
        std::fs::write(&path, "   \n").expect("write");
        let err = read_required_secret(&path).expect_err("empty must fail");
        assert!(err.contains("is empty"), "{err}");
    }

    #[test]
    fn required_secret_reads_the_llm_api_key() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("llm-api-key");
        std::fs::write(&path, "sk-test-key\n").expect("write");
        let secret = read_required_secret(&path).expect("present key");
        assert_eq!(secret.expose_secret(), "sk-test-key");
    }
}
