//! Bridge from the application [`Config`] to [`fkst_journal::JournalConfig`].
//!
//! `JournalConfig::from_config` lived inside the journal module before the #151
//! extraction split journaling into the `fkst-journal` crate. That crate cannot
//! see the control-plane's `Config`, so the mapping now lives here as a free
//! function — the one unavoidable decoupling of an otherwise pure crate move.
//! The field-for-field mapping is unchanged from the original `from_config`.

use crate::config::Config;
use crate::journal::github::DEFAULT_API_BASE;
use crate::journal::{default_identity_pointers, JournalConfig};

/// Build the journaling config from the loaded application [`Config`]
/// (`FKST_JOURNAL_*` / `FKST_RAISED_*` / `GITHUB_TOKEN`). The pointer list is
/// parsed from its comma-separated env form; blank entries are dropped and an
/// empty result falls back to the default projection.
pub fn journal_config_from_app(config: &Config) -> JournalConfig {
    let pointers: Vec<String> = config
        .raised_identity_pointers
        .split(',')
        .map(|pointer| pointer.trim().to_string())
        .filter(|pointer| !pointer.is_empty())
        .collect();
    JournalConfig {
        flush_interval: std::time::Duration::from_millis(config.journal_flush_interval_ms),
        flush_max_batch: config.journal_flush_max_batch,
        github_enabled: config.journal_github_enabled,
        issue_comments: config.journal_issue_comments,
        activity_comment_enabled: config.journal_activity_comment_enabled,
        cas_max_retries: config.journal_cas_max_retries,
        bootstrap_read_retries: config.journal_bootstrap_read_retries,
        github_branch: config.journal_github_branch.clone(),
        github_repo: config.journal_github_repo.clone(),
        github_api_base: DEFAULT_API_BASE.to_string(),
        identity_pointers: if pointers.is_empty() {
            default_identity_pointers()
        } else {
            pointers
        },
        max_line_bytes: config.raised_max_line_bytes,
        github_token: config.github_token.clone(),
    }
}
