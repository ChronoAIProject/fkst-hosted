//! Journaling configuration, per-session identity context, and the flush
//! outcome shape.

use std::fmt;
use std::time::Duration;

use secrecy::SecretString;

use crate::github::DEFAULT_API_BASE;

/// Journaling configuration (see the `FKST_JOURNAL_*` / `FKST_RAISED_*` env
/// table; constructed from the app `Config` in production and directly in
/// tests).
#[derive(Clone)]
pub struct JournalConfig {
    /// Max debounce before flushing buffered completions to GitHub.
    pub flush_interval: Duration,
    /// Flush early when this many new completions are buffered.
    pub flush_max_batch: usize,
    /// Master switch for GitHub journaling (the committed file is the SOLE
    /// machine-truth since #139; disabling it drops the durable floor).
    pub github_enabled: bool,
    /// Enable the optional issue-comment mirroring (dormant by default).
    pub issue_comments: bool,
    /// Enable the rolling activity comment on the flush cadence (#139).
    pub activity_comment_enabled: bool,
    /// Max optimistic-concurrency retries per flush.
    pub cas_max_retries: u32,
    /// Bootstrap eventual-consistency retries: how many times `load_skip_set`
    /// re-reads the committed file after a 404 before concluding "fresh run"
    /// (the just-committed file may not yet be visible on a fresh redo).
    pub bootstrap_read_retries: u32,
    /// Branch the journal file lives on.
    pub github_branch: String,
    /// `owner/name` of the journal repo; `None` disables GitHub journaling.
    pub github_repo: Option<String>,
    /// GitHub REST API base (tests point this at a mock server).
    pub github_api_base: String,
    /// JSON pointers forming event identity.
    pub identity_pointers: Vec<String>,
    /// Max stdout line length parsed; longer lines are malformed.
    pub max_line_bytes: usize,
    /// API token (env/secret-manager only; never logged).
    pub github_token: Option<SecretString>,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            flush_interval: Duration::from_millis(2000),
            flush_max_batch: 50,
            github_enabled: true,
            issue_comments: false,
            activity_comment_enabled: true,
            cas_max_retries: 5,
            bootstrap_read_retries: 3,
            github_branch: "main".to_string(),
            github_repo: None,
            github_api_base: DEFAULT_API_BASE.to_string(),
            identity_pointers: default_identity_pointers(),
            max_line_bytes: 1_048_576,
            github_token: None,
        }
    }
}

// Hand-written: the token must never appear in any Debug rendering.
impl fmt::Debug for JournalConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JournalConfig")
            .field("flush_interval", &self.flush_interval)
            .field("flush_max_batch", &self.flush_max_batch)
            .field("github_enabled", &self.github_enabled)
            .field("issue_comments", &self.issue_comments)
            .field("activity_comment_enabled", &self.activity_comment_enabled)
            .field("cas_max_retries", &self.cas_max_retries)
            .field("bootstrap_read_retries", &self.bootstrap_read_retries)
            .field("github_branch", &self.github_branch)
            .field("github_repo", &self.github_repo)
            .field("github_api_base", &self.github_api_base)
            .field("identity_pointers", &self.identity_pointers)
            .field("max_line_bytes", &self.max_line_bytes)
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// The default identity-pointer projection (`FKST_RAISED_IDENTITY_POINTERS`).
pub fn default_identity_pointers() -> Vec<String> {
    ["/department", "/source", "/name", "/corr"]
        .iter()
        .map(|p| p.to_string())
        .collect()
}

// `JournalConfig::from_config(&crate::config::Config)` lived here before the
// #151 extraction. fkst-journal can no longer see the control-plane's `Config`,
// so the same mapping now lives in the control-plane as the free fn
// `journal_config_from_app` (the only unavoidable decoupling of this otherwise
// pure move).

/// Identity of the session this journaler writes for.
#[derive(Debug, Clone)]
pub struct SessionCtx {
    /// `sessions._id` in uuid string form.
    pub session_id: String,
    pub package_name: String,
    /// Content fingerprint of the package (see [`package_fingerprint`]).
    pub package_fingerprint: String,
    /// Writer pod (lease `holder_pod`); `None` for local v1 runs.
    pub pod_id: Option<String>,
    /// Writer's lease fencing token; 0 when no lease exists (v1).
    pub fencing_token: i64,
}

/// Outcome of one [`Journaler::flush`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushOutcome {
    /// Completions committed to GitHub by this flush (0 when deferred,
    /// fenced, or GitHub-disabled).
    pub committed: usize,
    /// New blob sha when a GitHub write happened.
    pub commit_sha: Option<String>,
    /// True when this writer was fenced off as stale.
    pub fenced: bool,
}

impl FlushOutcome {
    pub(crate) fn skipped() -> Self {
        Self {
            committed: 0,
            commit_sha: None,
            fenced: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_config_debug_redacts_the_token() {
        let cfg = JournalConfig {
            github_token: Some(SecretString::from("ghp_leaky_value".to_string())),
            ..JournalConfig::default()
        };
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("ghp_leaky_value"), "token leaked");
        assert!(rendered.contains("<redacted>"));
    }
}
