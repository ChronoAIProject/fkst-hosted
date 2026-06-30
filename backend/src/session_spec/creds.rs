//! The on-disk credential file layout the run-session pod reads.
//!
//! The control plane creates a per-session Kubernetes Secret and mounts it into
//! the pod as a 0400 file volume. Keeping the layout typed (rather than
//! stringly-pathed at each call site) means the writer (the Job/Secret launcher)
//! and the reader (the run-session subcommand) can never disagree on a filename.

use std::path::{Path, PathBuf};

/// The GitHub App installation token (`ghs_…`): clone + git ops + log commits.
/// Matches the engine's runtime token filename so the pod copies it through
/// verbatim (`crate::engine::goal_token::TOKEN_FILE_NAME`).
pub const GITHUB_TOKEN_FILE: &str = "github-token";

/// The static LLM API key the engine's codex provider authenticates with. The
/// pod reads it and exports it under the `LLM_API_KEY` env var (the codex
/// `env_key`); the value comes from the control plane's `FKST_LLM_API_KEY`
/// config, not a per-session token, so it is written once and never rotated.
pub const LLM_API_KEY_FILE: &str = "llm-api-key";

/// Default mount path of the per-session credential Secret volume inside the pod.
pub const DEFAULT_CREDS_DIR: &str = "/var/run/fkst/creds";

/// Resolves credential file paths under a mounted Secret volume base directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredsLayout {
    base: PathBuf,
}

impl CredsLayout {
    /// Layout rooted at an explicit base (the Secret volume mount path).
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    /// Layout rooted at [`DEFAULT_CREDS_DIR`] — the conventional pod mount path.
    pub fn at_default_mount() -> Self {
        Self::new(DEFAULT_CREDS_DIR)
    }

    /// The volume base directory.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Path to the GitHub App installation token file.
    pub fn github_token(&self) -> PathBuf {
        self.base.join(GITHUB_TOKEN_FILE)
    }

    /// Path to the static LLM API key file.
    pub fn llm_api_key(&self) -> PathBuf {
        self.base.join(LLM_API_KEY_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mount_composes_the_expected_paths() {
        let layout = CredsLayout::at_default_mount();
        assert_eq!(layout.base(), Path::new("/var/run/fkst/creds"));
        assert_eq!(
            layout.github_token(),
            Path::new("/var/run/fkst/creds/github-token")
        );
        assert_eq!(
            layout.llm_api_key(),
            Path::new("/var/run/fkst/creds/llm-api-key")
        );
    }

    #[test]
    fn honors_a_custom_base() {
        let layout = CredsLayout::new("/mnt/creds");
        assert_eq!(layout.github_token(), Path::new("/mnt/creds/github-token"));
    }

    #[test]
    fn github_token_filename_matches_the_engine_runtime_convention() {
        // The pod copies this file through to the engine's runtime dir verbatim;
        // keep the two filenames identical so the convention is one constant.
        assert_eq!(
            GITHUB_TOKEN_FILE,
            crate::engine::goal_token::TOKEN_FILE_NAME
        );
    }
}
