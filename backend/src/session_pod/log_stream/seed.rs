//! Seed the fail-closed [`super::redact::Redactor`] from the pod's mounted creds.
//!
//! The redactor's strongest layer masks the EXACT known secrets (and their derived
//! encodings). The pod already holds every one of those secrets on disk — the
//! rotating GitHub token, the static LLM key, and each per-user env value — so this
//! reads them into a `(label, value)` table the collector feeds to the redactor.
//! It NEVER logs a value; only labels and counts are ever surfaced. The GitHub token
//! is re-read on a timer so a control-plane rotation is picked up mid-session.

use std::path::Path;

use crate::session_spec::creds::CredsLayout;

/// The label the GitHub App token is masked under.
pub const LABEL_GITHUB_TOKEN: &str = "github-token";
/// The label the LLM API key is masked under.
pub const LABEL_LLM_KEY: &str = "llm-key";
/// Prefix for a per-user env value's mask label (`userenv:<KEY>`).
pub const LABEL_USER_ENV_PREFIX: &str = "userenv:";

/// The initial known-secret table read from the creds dir: `(label, value)` pairs.
/// Empty values are dropped by the redactor, so a missing/blank file is harmless.
///
/// Best-effort: an unreadable creds dir yields an empty table (the redactor's
/// pattern + entropy layers still defend the stream) rather than aborting streaming.
pub fn seed_secrets(creds: &CredsLayout) -> Vec<(String, String)> {
    let mut secrets = Vec::new();
    if let Some(token) = read_github_token(&creds.github_token()) {
        secrets.push((LABEL_GITHUB_TOKEN.to_string(), token));
    }
    if let Some(key) = read_trimmed(&creds.llm_api_key()) {
        secrets.push((LABEL_LLM_KEY.to_string(), key));
    }
    match creds.user_env_files() {
        Ok(files) => {
            for (key, path) in files {
                if let Some(value) = read_trimmed(&path) {
                    secrets.push((format!("{LABEL_USER_ENV_PREFIX}{key}"), value));
                }
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "log-stream: could not list user env for redactor seeding");
        }
    }
    secrets
}

/// Read + parse the rotating `github-token` file, returning the bare `ghs_…` token.
/// The file is the `{"token": "...", "expires_at": "..."}` JSON the git helper
/// reads; a malformed file falls back to its trimmed raw contents so a token is
/// masked even if the shape ever changes. `None` when the file is absent/blank.
pub fn read_github_token(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => value
            .get("token")
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .filter(|t| !t.is_empty())
            // Defensive: a valid JSON object with no `token` still shouldn't leak, so
            // fall back to masking the whole document.
            .or_else(|| Some(trimmed.to_string())),
        Err(_) => Some(trimmed.to_string()),
    }
}

/// Read a plaintext credential file, trimming the trailing newline a Secret write
/// leaves. `None` on a missing/blank file.
fn read_trimmed(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
#[path = "seed_tests.rs"]
mod tests;
