//! HTTP route handlers.

pub mod auth;
pub mod environments;
pub mod github_app_webhook;
pub mod health;
// The dual-mode, identity-gated session-log download endpoint (`/api/v1/logs/*`):
// Bearer-token API mode + browser user-OAuth mode, three-tier authorization, and a
// short-lived presigned URL. Self-authorizing in-handler (no documented security).
pub mod logs;
pub mod metrics;
