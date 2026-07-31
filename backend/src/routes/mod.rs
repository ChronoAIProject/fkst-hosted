//! HTTP route handlers.

pub mod auth;
// The optional broader-visibility classic-OAuth connect flow (issue #572): a second
// credential (`repo` + `read:org`) used only to enumerate the caller's repos/orgs.
// Merged into the auth router; inert unless the broader OAuth pair is configured.
pub mod auth_broader;
// The pure signed-state / redirect-URI / token-shaping helpers both OAuth login
// flows share. Split out so `auth.rs` stays within the source line budget.
pub mod auth_oauth_state;
// The canvas dashboard's live REST surface: whole-account overview + per-repo
// session detail/create/stop, all computed from live GitHub reads (stateless).
pub mod canvas;
// The chat concierge's single public surface (`POST /api/v1/chat`): an authenticated
// SSE stream of one conversation turn. Mounted only when chat is configured.
pub mod chat;
pub mod dashboard;
pub mod environments;
pub mod github_app_webhook;
pub mod health;
// The dual-mode, identity-gated session-log download endpoint (`/api/v1/logs/*`):
// Bearer-token API mode + browser user-OAuth mode, three-tier authorization, and a
// short-lived presigned URL. Self-authorizing in-handler (no documented security).
pub mod logs;
pub mod metrics;
pub mod observe;
pub mod repos;
