//! The on-disk credential file layout the run-substrate pod reads.
//!
//! The control plane creates a per-session Kubernetes Secret and mounts it into
//! the pod as a 0400 file volume. Keeping the layout typed (rather than
//! stringly-pathed at each call site) means the writer (the session-Pod/Secret
//! launcher) and the reader (the run-substrate subcommand) can never disagree on
//! a filename.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The GitHub App installation token (`ghs_…`): clone + git ops + log commits.
/// The in-pod git credential helper + `gh` shim read this file on every op.
pub const GITHUB_TOKEN_FILE: &str = "github-token";

/// The static LLM API key the engine's codex provider authenticates with. The
/// pod reads it and exports it under the `LLM_API_KEY` env var (the codex
/// `env_key`); the value comes from the control plane's `FKST_LLM_API_KEY`
/// config, not a per-session token, so it is written once and never rotated.
pub const LLM_API_KEY_FILE: &str = "llm-api-key";

/// Filename prefix for an injected per-user env entry (PR4b). The control plane
/// writes one Secret data key `userenv.<KEY>` per resolved user env var; mounted,
/// each surfaces as a file `userenv.<KEY>` under [`CredsLayout::base`]. The runner
/// globs these, strips this prefix to recover `KEY`, and folds the file's
/// contents into the engine `env_profile`. `KEY` is env-var-shaped
/// (`^[A-Za-z_][A-Za-z0-9_]*$`), so the composite `userenv.<KEY>` is a valid
/// Kubernetes Secret data key (`[-._a-zA-Z0-9]+`).
pub const USER_ENV_PREFIX: &str = "userenv.";

/// Default mount path of the per-session credential Secret volume inside the pod.
pub const DEFAULT_CREDS_DIR: &str = "/var/run/fkst/creds";

/// Assemble the credential data keys carried by a per-session Kubernetes Secret:
/// the rotating [`GITHUB_TOKEN_FILE`], the static [`LLM_API_KEY_FILE`], and one
/// [`USER_ENV_PREFIX`]`<KEY>` entry per injected per-user env var.
///
/// The session-Pod Secret builder builds on top of this map — the Model-B
/// session Secret carries only these creds — so the credential layout lives in
/// exactly one place. Callers expose their own secret values before calling,
/// which keeps this module free of a `secrecy` dependency.
pub fn credential_secret_data<'a>(
    github_token: &str,
    llm_api_key: &str,
    user_env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, String> {
    let mut data = BTreeMap::new();
    data.insert(GITHUB_TOKEN_FILE.to_string(), github_token.to_string());
    data.insert(LLM_API_KEY_FILE.to_string(), llm_api_key.to_string());
    for (key, value) in user_env {
        data.insert(format!("{USER_ENV_PREFIX}{key}"), value.to_string());
    }
    data
}

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

    /// List the mounted per-user env files (PR4b): every entry directly under
    /// [`base`](Self::base) whose filename starts with [`USER_ENV_PREFIX`],
    /// returned as `(KEY, path)` with the prefix stripped from `KEY`. The
    /// non-user files (the spec, the GitHub token, the LLM key) are skipped, as is
    /// a bare `userenv.` with no key. Order follows the filesystem and the caller
    /// must not depend on it. The recovered `KEY` is whatever followed the prefix;
    /// reserved-name filtering is the reader's job, not this lister's.
    pub fn user_env_files(&self) -> std::io::Result<Vec<(String, PathBuf)>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.base)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if let Some(key) = name.strip_prefix(USER_ENV_PREFIX) {
                if !key.is_empty() {
                    files.push((key.to_string(), entry.path()));
                }
            }
        }
        Ok(files)
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
    fn user_env_files_lists_only_prefixed_entries_with_the_key_stripped() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("userenv.FOO"), "foo").expect("write FOO");
        std::fs::write(dir.path().join("userenv.BAR_2"), "bar").expect("write BAR_2");
        // Non-user files must be ignored entirely.
        std::fs::write(dir.path().join(GITHUB_TOKEN_FILE), "ghs").expect("write token");
        std::fs::write(dir.path().join(LLM_API_KEY_FILE), "sk").expect("write llm");
        std::fs::write(dir.path().join("session-spec.json"), "{}").expect("write spec");
        // A bare prefix with no key must be skipped (no empty-named entry).
        std::fs::write(dir.path().join("userenv."), "x").expect("write bare");

        let layout = CredsLayout::new(dir.path());
        let mut found = layout.user_env_files().expect("list");
        found.sort();
        assert_eq!(
            found,
            vec![
                ("BAR_2".to_string(), dir.path().join("userenv.BAR_2")),
                ("FOO".to_string(), dir.path().join("userenv.FOO")),
            ]
        );
    }

    #[test]
    fn user_env_files_is_empty_when_no_entries_present() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join(GITHUB_TOKEN_FILE), "ghs").expect("write token");
        let layout = CredsLayout::new(dir.path());
        assert!(layout.user_env_files().expect("list").is_empty());
    }

    #[test]
    fn credential_secret_data_carries_the_base_creds_and_user_env() {
        let user_env = [("FOO", "foo-val"), ("API_TOKEN", "tok-val")];
        let data = credential_secret_data("ghs_json", "sk-key", user_env);
        assert_eq!(data["github-token"], "ghs_json");
        assert_eq!(data["llm-api-key"], "sk-key");
        assert_eq!(data["userenv.FOO"], "foo-val");
        assert_eq!(data["userenv.API_TOKEN"], "tok-val");
        // Exactly the two base keys plus the two user-env keys — nothing else.
        assert_eq!(data.len(), 4);
    }

    #[test]
    fn credential_secret_data_with_no_user_env_carries_only_the_base_creds() {
        let data = credential_secret_data("ghs_json", "sk-key", std::iter::empty());
        assert_eq!(data.len(), 2);
        assert!(data.contains_key("github-token"));
        assert!(data.contains_key("llm-api-key"));
    }
}
