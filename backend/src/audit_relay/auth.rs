//! Bearer authentication for the internal relay API.
//!
//! Three properties, each of which is a security requirement rather than a
//! stylistic preference:
//!
//! 1. **Two credentials, checked separately.** Write endpoints accept only the
//!    write token; the scoped read accepts only the read token. A write token
//!    presented to the read endpoint is `401`, and vice versa — that separation
//!    is the whole reason the relay has two secrets (see [`super::config`]).
//! 2. **Constant-time comparison.** A byte-by-byte `==` on a secret leaks its
//!    prefix through timing. [`subtle::ConstantTimeEq`] is used rather than a
//!    hand-rolled loop, which an optimizer is free to short-circuit.
//! 3. **Nothing is ever logged.** No token, no prefix, no length, no hash. A
//!    rejection logs the ROLE that was expected and nothing else; the failure
//!    reason is a closed enum that is structurally incapable of carrying the
//!    presented value.

use axum::http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use subtle::ConstantTimeEq;

/// Which credential an endpoint requires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenRole {
    /// Admits records (the three write endpoints).
    Write,
    /// Reads the recent audit trail (the scoped read endpoint).
    Read,
}

impl TokenRole {
    /// The bounded label for logs and metrics.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenRole::Write => "write",
            TokenRole::Read => "read",
        }
    }
}

/// Why a request was refused. Deliberately valueless.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer credentials")]
    Missing,
    #[error("malformed authorization header")]
    Malformed,
    #[error("bearer credentials rejected")]
    Rejected,
}

impl AuthError {
    /// The bounded reason label.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthError::Missing => "missing",
            AuthError::Malformed => "malformed",
            AuthError::Rejected => "rejected",
        }
    }
}

/// The relay's two credentials.
#[derive(Clone)]
pub struct RelayTokens {
    write: SecretString,
    read: SecretString,
}

// Hand-written: neither secret may reach a log through a `{:?}` on the tokens,
// on the HTTP state, or on anything embedding them.
impl std::fmt::Debug for RelayTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayTokens")
            .field("write", &"<redacted>")
            .field("read", &"<redacted>")
            .finish()
    }
}

impl RelayTokens {
    pub fn new(write: SecretString, read: SecretString) -> Self {
        Self { write, read }
    }

    /// Check the request's `Authorization` header against the role's credential.
    pub fn authorize(&self, headers: &HeaderMap, role: TokenRole) -> Result<(), AuthError> {
        let presented = bearer(headers)?;
        let expected = match role {
            TokenRole::Write => &self.write,
            TokenRole::Read => &self.read,
        };
        if constant_time_eq(presented.as_bytes(), expected.expose_secret().as_bytes()) {
            return Ok(());
        }
        // The role is a compile-time constant; nothing about the presented value
        // — not its length, not a prefix, not a digest — is recorded.
        tracing::warn!(
            role = role.as_str(),
            "audit relay: rejected bearer credentials"
        );
        Err(AuthError::Rejected)
    }
}

/// Extract the `Bearer <token>` value.
fn bearer(headers: &HeaderMap) -> Result<&str, AuthError> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(AuthError::Missing)?;
    let value = value.to_str().map_err(|_| AuthError::Malformed)?;
    let (scheme, token) = value.split_once(' ').ok_or(AuthError::Malformed)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(AuthError::Malformed);
    }
    let token = token.trim();
    if token.is_empty() {
        return Err(AuthError::Malformed);
    }
    Ok(token)
}

/// Constant-time byte equality.
///
/// The length check up front is deliberate and safe: a credential's LENGTH is
/// not the secret, and `ct_eq` requires equal-length slices. Everything past it
/// runs in time independent of how many bytes matched.
fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    if presented.len() != expected.len() {
        return false;
    }
    presented.ct_eq(expected).into()
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
