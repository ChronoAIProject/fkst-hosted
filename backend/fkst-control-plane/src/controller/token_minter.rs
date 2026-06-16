//! Session-scoped token minting seam for the controller's mid-run credential
//! refresh channel (#151).
//!
//! The worker no longer mints GitHub-App installation tokens itself: when an
//! engine needs a fresh token mid-run it asks the controller over the
//! internal-auth channel and the controller mints. This module is the injectable
//! abstraction that does the minting, so the credential-refresh handler depends
//! only on the [`SessionTokenMinter`] trait — the concrete [`GithubAppMinter`]
//! plugs in at construction and a [`RecordingMinter`] (in tests) plugs in for
//! unit tests, with no change to the handler.
//!
//! ## Consecutive-failure escalation lives here now
//! When the engine ran on the worker, the worker tracked consecutive mint
//! failures and decided when a session was unrecoverable. With minting moved
//! controller-side (#151), that escalation state moves here too: the minter
//! keeps a per-session consecutive-failure counter, resets it on success, and
//! logs loudly once it crosses [`MAX_CONSECUTIVE_MINT_FAILURES`]. It does NOT
//! push a `StopSession` itself — wiring the controller→worker stop channel is a
//! later increment; see the TODO at the escalation point.
//!
//! A token value NEVER appears in any log, `Debug`, or error: [`MintResult::Token`]
//! carries a [`SecretString`], and nothing here renders it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::github_app::{GithubAppError, GithubAppTokens};

/// Consecutive mint failures for one session at/after which the minter logs a
/// loud escalation warning. Mirrors the worker-side threshold that used to gate
/// the engine's just-in-time credential helper before minting moved
/// controller-side (#151).
pub const MAX_CONSECUTIVE_MINT_FAILURES: u32 = 5;

/// Outcome of a session token mint. `Token` carries a [`SecretString`] so the
/// value is never rendered by `Debug`/logs; `Gone` is the App-uninstalled
/// signal (the worker should stop); `Failed` is a transient error (the worker
/// keeps its current token and retries later).
pub enum MintResult {
    /// A fresh installation token plus its absolute expiry.
    Token {
        token: SecretString,
        expires_at: SystemTime,
    },
    /// The App installation is gone for this repo (uninstalled). Terminal for
    /// the session: the worker should stop, never retry.
    Gone,
    /// A transient mint failure (auth blip, rate limit, transport). The worker
    /// keeps its current token and retries on its next refresh trigger.
    Failed,
}

impl std::fmt::Debug for MintResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Never render the token value, even in Debug.
            Self::Token { expires_at, .. } => f
                .debug_struct("Token")
                .field("token", &"<redacted>")
                .field("expires_at", expires_at)
                .finish(),
            Self::Gone => write!(f, "Gone"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

/// Mints a fresh installation token for a running session's repo. The seam the
/// credential-refresh handler depends on; the concrete impl ([`GithubAppMinter`])
/// plugs in at construction and a test fake plugs in for unit tests.
#[async_trait]
pub trait SessionTokenMinter: Send + Sync {
    /// Mint a fresh token for `session_id`'s `repo_ref` (`owner/repo`). The fence
    /// guard runs in the handler BEFORE this is ever called, so a stale worker
    /// never reaches here.
    async fn mint(&self, session_id: bson::Uuid, repo_ref: &str) -> MintResult;
}

/// The production minter: delegates to [`GithubAppTokens`] and owns the
/// per-session consecutive-failure escalation state (#151).
pub struct GithubAppMinter {
    tokens: GithubAppTokens,
    /// Per-session consecutive mint-failure counter. Reset on success;
    /// incremented on a transient `Failed`; consulted for the loud escalation
    /// log once it crosses [`MAX_CONSECUTIVE_MINT_FAILURES`].
    failures: Mutex<HashMap<bson::Uuid, u32>>,
}

impl GithubAppMinter {
    pub fn new(tokens: GithubAppTokens) -> Self {
        Self {
            tokens,
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// Reset a session's consecutive-failure counter after a successful mint.
    fn reset_failures(&self, session_id: bson::Uuid) {
        self.failures
            .lock()
            .expect("mint failures map poisoned")
            .remove(&session_id);
    }

    /// Record a transient mint failure and return the new consecutive count.
    fn record_failure(&self, session_id: bson::Uuid) -> u32 {
        let mut map = self.failures.lock().expect("mint failures map poisoned");
        let count = map.entry(session_id).or_insert(0);
        *count += 1;
        *count
    }
}

#[async_trait]
impl SessionTokenMinter for GithubAppMinter {
    async fn mint(&self, session_id: bson::Uuid, repo_ref: &str) -> MintResult {
        match self.tokens.token_with_expiry_for_repo(repo_ref, None).await {
            Ok((token, expires_at)) => {
                self.reset_failures(session_id);
                tracing::debug!(
                    session_id = %session_id,
                    "session token minted for credential refresh"
                );
                MintResult::Token { token, expires_at }
            }
            Err(GithubAppError::InstallationGone { .. }) => {
                // App uninstalled: terminal for the session. Not counted as a
                // transient failure (it will never recover by retrying).
                tracing::warn!(
                    session_id = %session_id,
                    "session token mint: installation gone (App uninstalled)"
                );
                // TODO(#151 activation): push StopSession on FatalExpired/Gone.
                MintResult::Gone
            }
            Err(error) => {
                let consecutive = self.record_failure(session_id);
                if consecutive >= MAX_CONSECUTIVE_MINT_FAILURES {
                    // Loud escalation: this session has failed to mint a token
                    // for too many consecutive attempts and is likely stuck.
                    // The error type is redacted of token-like detail by its own
                    // `Debug`; we log the type, never a token.
                    tracing::error!(
                        session_id = %session_id,
                        consecutive,
                        error = ?error,
                        "session token mint failed {consecutive} times consecutively; \
                         session is likely unrecoverable"
                    );
                    // TODO(#151 activation): push StopSession on FatalExpired/Gone.
                } else {
                    tracing::warn!(
                        session_id = %session_id,
                        consecutive,
                        error = ?error,
                        "session token mint failed (transient); worker keeps current token"
                    );
                }
                MintResult::Failed
            }
        }
    }
}
