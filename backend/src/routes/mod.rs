//! HTTP route handlers.

pub mod environments;
pub mod github_app_webhook;
pub mod health;
pub mod metrics;
// `session_ops` + `github_app_webhook::comment_control` (the `/stop` + `/status`
// issue-comment control path) are unreachable after the PR6 Model B flip; the
// files are left in place and deleted in PR7. Not declared here so the now-unused
// `pub(crate)` helpers do not trip the dead-code lint under `-D warnings`.
