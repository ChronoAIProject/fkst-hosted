//! Safe arguments for the browser authentication and OAuth surfaces.
//!
//! These are the routes whose REQUEST is almost entirely secret: an
//! authorization code, a CSRF state, a refresh token, GitHub's own error slug.
//! None of it is a valid audit property, and none of it appears below.
//!
//! What IS recorded is the pair that makes an incident readable: WHICH flow ran
//! (four closed constants) and HOW it ended (four closed outcomes). Combined
//! with the verified actor — which the OAuth callbacks publish separately, only
//! after `GET /user` names the token's owner — that is enough to answer "did
//! this person sign in, and did it work" without the analytics store ever
//! holding a credential.
//!
//! The session-logs callback is the one flow carrying a correlation handle. Its
//! `session_id` comes from the SIGNED state, and only after the HMAC verified:
//! an unverified state is attacker-chosen text, so a failed verification records
//! the flow and the outcome and nothing else.

use serde::Serialize;

use super::bounds::safe_session_id;
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};

/// The frontend sign-in flow.
pub const FLOW_LOGIN: &str = "login";
/// The frontend token-refresh flow.
pub const FLOW_REFRESH: &str = "refresh";
/// The optional classic-OAuth broader-visibility connect flow.
pub const FLOW_BROADER_VISIBILITY: &str = "broader_visibility";
/// The browser log-download authorization flow.
pub const FLOW_SESSION_LOGS: &str = "session_logs";

/// How an OAuth round-trip ended. A closed enum, so the property is bounded by
/// construction and safe as a dashboard facet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OauthResult {
    /// The exchange completed and the identity behind the token resolved.
    Success,
    /// The user declined consent on GitHub's screen.
    Denied,
    /// Missing, tampered, expired, or replayed request material.
    Invalid,
    /// GitHub rejected the exchange, or the deployment could not reach it.
    UpstreamError,
}

/// `github_login` — the authorize redirect. No result: the response IS the
/// redirect, and its target carries the client id and state.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeGithubLogin {
    flow: &'static str,
}

impl SafeGithubLogin {
    pub fn new() -> Self {
        Self { flow: FLOW_LOGIN }
    }
}

impl Default for SafeGithubLogin {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundedAuditArguments for SafeGithubLogin {
    const OPERATION_ID: &'static str = "github_login";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_LOGIN_FIELDS;
}

/// `github_login_callback` — GitHub's return leg.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeGithubLoginCallback {
    flow: &'static str,
    result: OauthResult,
}

impl SafeGithubLoginCallback {
    pub fn new(result: OauthResult) -> Self {
        Self {
            flow: FLOW_LOGIN,
            result,
        }
    }
}

impl BoundedAuditArguments for SafeGithubLoginCallback {
    const OPERATION_ID: &'static str = "github_login_callback";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_LOGIN_CALLBACK_FIELDS;
}

/// `github_refresh_token` — redeeming a refresh token. The body is the
/// credential, so nothing about it is recorded beyond the outcome.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeGithubRefreshToken {
    flow: &'static str,
    result: OauthResult,
}

impl SafeGithubRefreshToken {
    pub fn new(result: OauthResult) -> Self {
        Self {
            flow: FLOW_REFRESH,
            result,
        }
    }
}

impl BoundedAuditArguments for SafeGithubRefreshToken {
    const OPERATION_ID: &'static str = "github_refresh_token";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_REFRESH_TOKEN_FIELDS;
}

/// `github_broader_connect` — the classic-OAuth authorize redirect.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeGithubBroaderConnect {
    flow: &'static str,
}

impl SafeGithubBroaderConnect {
    pub fn new() -> Self {
        Self {
            flow: FLOW_BROADER_VISIBILITY,
        }
    }
}

impl Default for SafeGithubBroaderConnect {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundedAuditArguments for SafeGithubBroaderConnect {
    const OPERATION_ID: &'static str = "github_broader_connect";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_BROADER_CONNECT_FIELDS;
}

/// `github_broader_callback` — the classic-OAuth return leg.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeGithubBroaderCallback {
    flow: &'static str,
    result: OauthResult,
}

impl SafeGithubBroaderCallback {
    pub fn new(result: OauthResult) -> Self {
        Self {
            flow: FLOW_BROADER_VISIBILITY,
            result,
        }
    }
}

impl BoundedAuditArguments for SafeGithubBroaderCallback {
    const OPERATION_ID: &'static str = "github_broader_callback";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_BROADER_CALLBACK_FIELDS;
}

/// `session_logs_oauth_callback` — the browser log-download return leg.
#[derive(Clone, Debug, Serialize)]
pub struct SafeSessionLogsOauthCallback {
    flow: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    result: OauthResult,
}

impl BoundedAuditArguments for SafeSessionLogsOauthCallback {
    const OPERATION_ID: &'static str = "session_logs_oauth_callback";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::SESSION_LOGS_OAUTH_CALLBACK_FIELDS;
}

/// The input view for the session-logs callback.
///
/// `verified_session_id` is `Some` ONLY once the state's HMAC verified. The type
/// makes that ordering visible at the call site: there is no way to pass an
/// unverified state through it.
pub struct SessionLogsCallbackInput<'a> {
    /// The session id recovered from the SIGNED state, after verification.
    pub verified_session_id: Option<&'a str>,
    pub result: OauthResult,
}

impl Sealed for SessionLogsCallbackInput<'_> {}

impl ToSafeAuditArguments for SessionLogsCallbackInput<'_> {
    type Safe = SafeSessionLogsOauthCallback;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeSessionLogsOauthCallback {
            flow: FLOW_SESSION_LOGS,
            session_id: self.verified_session_id.and_then(safe_session_id),
            result: self.result,
        }
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
