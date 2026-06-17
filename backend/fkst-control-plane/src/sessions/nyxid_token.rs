//! Per-session NyxID token provisioning (issue #111; TTL cleanup #216).
//!
//! At session start the driver mints ONE per-session, SELF-EXPIRING NyxID
//! agent API key on the triggering user's behalf and injects it into the
//! engine environment as `NYXID_ACCESS_TOKEN` (plus the `NYXID_URL` origin),
//! so the engine's `nyxid` CLI and its codex LLM provider both authenticate
//! to NyxID as that user — with no fkst-substrate engine change.
//!
//! Key cleanup is a self-expiring TTL (#216): the key is minted with
//! `expires_at = now + FKST_SESSION_KEY_TTL_SECS`, so NyxID disables it on its
//! own once the run is over. This replaces the former service-account
//! revoke-at-teardown, which NyxID rejects on a human-minted key (the SA token
//! is refused by the human-only key store). A background janitor sweep that
//! deletes already-expired keys is a follow-up backstop (out of scope here);
//! the TTL is the primary and sufficient fix.
//!
//! Secret hygiene (load-bearing):
//! - The full key (`nyxid_ag_…`) lives ONLY in the returned `SecretString` env
//!   entry; it is never logged, returned over HTTP, or persisted.
//! - Only the non-secret [`NyxidTokenHandle`] (`key_id` + a short `key_prefix`
//!   for diagnostics) is held by the driver and persisted onto `SessionDoc`.
//! - The user's `raw_token` is transient in-memory; it never reaches here from
//!   anything but a live HTTP request context.

use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret, SecretString};

use crate::error::AppError;
use crate::nyxid::{NyxIdClient, NyxIdError};

/// Engine env var carrying the per-session NyxID agent key. A SECRET — rides
/// the `env_profile` (never the host allow-list).
pub const NYXID_ACCESS_TOKEN_KEY: &str = "NYXID_ACCESS_TOKEN";

/// Engine env var carrying the NyxID origin the engine talks to. Non-secret,
/// but it is carried per-session in the `env_profile` (NOT the host
/// allow-list): a key on `ENGINE_ENV_ALLOWLIST` is treated as reserved and
/// dropped from the `env_profile` by the engine env filter, and the allow-list
/// only copies a var from the parent pod env (where this per-session origin
/// does not exist). Keeping it non-reserved is what lets the per-session value
/// reach the engine. (See the #111 changeset for the full rationale.)
pub const NYXID_URL_KEY: &str = "NYXID_URL";

/// Scope requested for the session key. With `allow_all_services=true` this
/// grants proxy access across every service the user has connected — the one
/// key serves both the `nyxid` CLI and codex's chrono-llm provider.
const SESSION_KEY_SCOPES: &str = "proxy";

/// Number of leading chars of the minted key kept as a NON-secret diagnostic
/// prefix (e.g. `nyxid_ag_abc`). Short enough that the prefix alone cannot be
/// used to authenticate, long enough to disambiguate keys in a log line.
const KEY_PREFIX_LEN: usize = 12;

/// Non-secret reference to a provisioned session key. Held by the driver for
/// teardown revoke and persisted onto `SessionDoc`. Carries NO secret material.
#[derive(Debug, Clone)]
pub struct NyxidTokenHandle {
    /// Stable key id used to revoke the key at teardown.
    pub key_id: String,
    /// Short, non-secret prefix of the full key for diagnostics only.
    pub key_prefix: String,
}

/// Mint a per-session NyxID agent key for `session_id` on the user's behalf
/// and return its non-secret handle plus the engine env entries to inject.
///
/// The key is named `fkst-session-<id>` (so an operator can correlate a key
/// to its session and an orphan-sweep can match by prefix), scoped `proxy`,
/// and `allow_all_services=true`. It SELF-EXPIRES after `ttl`: the key is
/// minted with `expires_at = now + ttl` (#216), so NyxID disables it on its
/// own — there is no service-account revoke (NyxID rejects it on a human-minted
/// key). On a user-token rejection (expired/revoked/delegated) this returns
/// `Unauthorized`; any other NyxID failure returns `Unavailable`. The minted
/// key is never logged here — only the non-secret id/prefix are.
pub async fn provision(
    client: &NyxIdClient,
    session_id: bson::Uuid,
    origin: &str,
    raw_token: &SecretString,
    ttl: Duration,
) -> Result<(NyxidTokenHandle, Vec<(String, SecretString)>), AppError> {
    let name = format!("fkst-session-{session_id}");
    let expires_at = expires_at_rfc3339(session_id, ttl)?;
    let created = client
        .mint_user_api_key(
            raw_token,
            &name,
            SESSION_KEY_SCOPES,
            true,
            Some(&expires_at),
        )
        .await
        .map_err(|err| map_mint_error(session_id, err))?;

    let key_prefix = derive_prefix(created.full_key.expose_secret());
    let handle = NyxidTokenHandle {
        key_id: created.id,
        key_prefix,
    };
    tracing::info!(
        session_id = %session_id,
        key_id = %handle.key_id,
        key_prefix = %handle.key_prefix,
        expires_at = %expires_at,
        "provisioned per-session nyxid token (self-expiring)"
    );

    let env = vec![
        (NYXID_ACCESS_TOKEN_KEY.to_string(), created.full_key),
        (
            NYXID_URL_KEY.to_string(),
            SecretString::from(origin.to_string()),
        ),
    ];
    Ok((handle, env))
}

/// Teardown hook for a previously provisioned session key.
///
/// NO LONGER calls NyxID (#216): the key is minted self-expiring, and the only
/// revoke route is the service-account `DELETE` NyxID rejects on a human-minted
/// key. Cleanup is therefore the TTL — this hook just records that teardown ran
/// and that the key will lapse on its own. Kept as a function (rather than
/// deleted at the call sites) so a future janitor backstop has one place to
/// land, and so the teardown path stays symmetric with `provision`. The
/// `client` is unused now but retained in the signature for that backstop.
pub async fn revoke(_client: &NyxIdClient, handle: &NyxidTokenHandle) {
    tracing::info!(
        key_id = %handle.key_id,
        "per-session nyxid token left to self-expire (no service-account revoke; TTL is the cleanup)"
    );
}

/// Map a mint error onto an `AppError`. The DISTINCT user-token rejection is a
/// 401 (the caller presented an invalid/delegated token); everything else is
/// the credential proxy being unavailable. The token/key are never in the error.
fn map_mint_error(session_id: bson::Uuid, err: NyxIdError) -> AppError {
    match err {
        NyxIdError::UserTokenRejected => {
            tracing::error!(
                session_id = %session_id,
                "nyxid rejected the user token while minting the session key"
            );
            AppError::Unauthorized(
                "nyxid rejected the user token for the session; cannot mint session credential"
                    .to_string(),
            )
        }
        other => {
            tracing::error!(
                session_id = %session_id,
                error = %other,
                "nyxid api-key mint failed"
            );
            AppError::Unavailable(
                "credential proxy unavailable; cannot mint session credential".to_string(),
            )
        }
    }
}

/// Take the leading [`KEY_PREFIX_LEN`] chars of a key as a non-secret
/// diagnostic prefix, respecting char boundaries (the key is ASCII in
/// practice, but be defensive).
fn derive_prefix(full_key: &str) -> String {
    full_key.chars().take(KEY_PREFIX_LEN).collect()
}

/// Render the absolute key expiry (`now + ttl`) as an RFC 3339 timestamp string
/// — the exact shape NyxID's `CreateApiKeyRequest.expires_at` accepts (verified
/// against NyxID `main`: it parses RFC 3339 or `YYYY-MM-DD`). Uses
/// `bson::DateTime` (already a dependency) so no new crate is pulled in. A TTL
/// so large it overflows `SystemTime` is mapped to `Unavailable` rather than
/// panicking — a misconfiguration, not a user error.
fn expires_at_rfc3339(session_id: bson::Uuid, ttl: Duration) -> Result<String, AppError> {
    let when = SystemTime::now().checked_add(ttl).ok_or_else(|| {
        tracing::error!(
            session_id = %session_id,
            "session key TTL overflowed the system clock; refusing to mint"
        );
        AppError::Unavailable("session key TTL is out of range".to_string())
    })?;
    bson::DateTime::from_system_time(when)
        .try_to_rfc3339_string()
        .map_err(|error| {
            tracing::error!(
                session_id = %session_id,
                error = %error,
                "failed to render the session key expiry timestamp"
            );
            AppError::Unavailable("could not compute session key expiry".to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const API_KEYS_PATH: &str = "/api/v1/api-keys";

    fn client(uri: &str) -> NyxIdClient {
        NyxIdClient::new(uri, "api-github", std::time::Duration::from_secs(30))
            .expect("client build")
    }

    #[test]
    fn derive_prefix_keeps_only_a_short_non_secret_head() {
        let prefix = derive_prefix("nyxid_ag_0123456789abcdef");
        assert_eq!(prefix, "nyxid_ag_012");
        assert_eq!(prefix.len(), KEY_PREFIX_LEN);
    }

    #[test]
    fn derive_prefix_handles_a_short_key_without_panicking() {
        assert_eq!(derive_prefix("short"), "short");
    }

    /// A representative session-key TTL for the provision tests (#216).
    const TEST_TTL: Duration = Duration::from_secs(3600);

    #[tokio::test]
    async fn provision_mints_and_returns_handle_plus_env_entries() {
        let server = MockServer::start().await;
        let session_id = bson::Uuid::new();
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .and(header("authorization", "Bearer user_raw_tok"))
            .and(body_string_contains(format!("fkst-session-{session_id}")))
            .and(body_string_contains("proxy"))
            // The TTL must reach NyxID as a self-expiring `expires_at` (#216).
            .and(body_string_contains("expires_at"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "key-1",
                "full_key": "nyxid_ag_supersecretkeyvalue000"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let raw = SecretString::from("user_raw_tok".to_string());
        let (handle, env) = provision(
            &client(&server.uri()),
            session_id,
            "https://nyxid.test",
            &raw,
            TEST_TTL,
        )
        .await
        .expect("provision");

        assert_eq!(handle.key_id, "key-1");
        assert_eq!(handle.key_prefix, "nyxid_ag_sup");
        // The handle is non-secret: it must never carry the full key.
        assert!(!format!("{handle:?}").contains("supersecretkeyvalue"));

        // Two env entries: the secret token and the non-secret origin.
        assert_eq!(env.len(), 2);
        let token = env
            .iter()
            .find(|(k, _)| k == NYXID_ACCESS_TOKEN_KEY)
            .expect("token entry");
        assert_eq!(token.1.expose_secret(), "nyxid_ag_supersecretkeyvalue000");
        let url = env
            .iter()
            .find(|(k, _)| k == NYXID_URL_KEY)
            .expect("url entry");
        assert_eq!(url.1.expose_secret(), "https://nyxid.test");
    }

    #[tokio::test]
    async fn provision_maps_a_401_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let raw = SecretString::from("bad".to_string());
        let err = provision(
            &client(&server.uri()),
            bson::Uuid::new(),
            "o",
            &raw,
            TEST_TTL,
        )
        .await
        .expect_err("must fail");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn provision_maps_a_403_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let raw = SecretString::from("delegated".to_string());
        let err = provision(
            &client(&server.uri()),
            bson::Uuid::new(),
            "o",
            &raw,
            TEST_TTL,
        )
        .await
        .expect_err("must fail");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn provision_maps_a_500_to_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let raw = SecretString::from("tok".to_string());
        let err = provision(
            &client(&server.uri()),
            bson::Uuid::new(),
            "o",
            &raw,
            TEST_TTL,
        )
        .await
        .expect_err("must fail");
        assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn provision_never_logs_or_returns_the_secret_in_the_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "k", "full_key": "nyxid_ag_NEVER_LOG_THIS_VALUE"
            })))
            .mount(&server)
            .await;
        let raw = SecretString::from("tok".to_string());
        let (handle, env) = provision(
            &client(&server.uri()),
            bson::Uuid::new(),
            "o",
            &raw,
            TEST_TTL,
        )
        .await
        .expect("provision");
        // The handle's Debug must not contain the secret tail.
        assert!(!format!("{handle:?}").contains("NEVER_LOG_THIS_VALUE"));
        // The secret lives only inside a SecretString, whose Debug is redacted.
        let token = env
            .iter()
            .find(|(k, _)| k == NYXID_ACCESS_TOKEN_KEY)
            .expect("token");
        assert!(!format!("{:?}", token.1).contains("NEVER_LOG_THIS_VALUE"));
    }

    #[tokio::test]
    async fn revoke_makes_no_nyxid_call_and_returns_unit() {
        // #216: the key self-expires via its TTL, so teardown must NOT hit
        // NyxID at all. Mount a server that rejects EVERY request: if revoke
        // tried to call the (rejected) service-account DELETE, the 418 would
        // make `revoke_api_key` error — but revoke no longer calls it, so the
        // hook completes cleanly without touching the server.
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(418))
            .expect(0)
            .mount(&server)
            .await;
        let handle = NyxidTokenHandle {
            key_id: "k".to_string(),
            key_prefix: "nyxid_ag_xxx".to_string(),
        };
        // Returns unit and issues no HTTP request (the `expect(0)` above is
        // verified on server drop).
        revoke(&client(&server.uri()), &handle).await;
    }

    #[test]
    fn expires_at_rfc3339_renders_a_parseable_rfc3339_timestamp() {
        // The expiry must be a well-formed RFC 3339 string NyxID can parse, and
        // it must lie in the future for a positive TTL.
        let before = bson::DateTime::now();
        let rendered =
            expires_at_rfc3339(bson::Uuid::new(), Duration::from_secs(3600)).expect("render");
        let parsed = bson::DateTime::parse_rfc3339_str(&rendered).expect("valid rfc3339");
        assert!(
            parsed.timestamp_millis() > before.timestamp_millis(),
            "expiry must be in the future, got {rendered}"
        );
    }
}
