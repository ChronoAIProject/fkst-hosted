//! Per-session NyxID token provisioning (issue #111).
//!
//! At session start the driver mints ONE per-session, non-expiring NyxID
//! agent API key on the triggering user's behalf and injects it into the
//! engine environment as `NYXID_ACCESS_TOKEN` (plus the `NYXID_URL` origin),
//! so the engine's `nyxid` CLI and its codex LLM provider both authenticate
//! to NyxID as that user — with no fkst-substrate engine change. The key is
//! revoked at session teardown to bound its blast radius.
//!
//! Secret hygiene (load-bearing):
//! - The full key (`nyxid_ag_…`) lives ONLY in the returned `SecretString` env
//!   entry; it is never logged, returned over HTTP, or persisted.
//! - Only the non-secret [`NyxidTokenHandle`] (`key_id` + a short `key_prefix`
//!   for diagnostics) is held by the driver and persisted onto `SessionDoc`.
//! - The user's `raw_token` is transient in-memory; it never reaches here from
//!   anything but a live HTTP request context.

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
/// and `allow_all_services=true`; it is non-expiring (the session revokes it
/// at teardown). On a user-token rejection (expired/revoked/delegated) this
/// returns `Unauthorized`; any other NyxID failure returns `Unavailable`. The
/// minted key is never logged here — only the non-secret id/prefix are.
pub async fn provision(
    client: &NyxIdClient,
    session_id: bson::Uuid,
    origin: &str,
    raw_token: &SecretString,
) -> Result<(NyxidTokenHandle, Vec<(String, SecretString)>), AppError> {
    let name = format!("fkst-session-{session_id}");
    let created = client
        .mint_user_api_key(raw_token, &name, SESSION_KEY_SCOPES, true)
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
        "provisioned per-session nyxid token"
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

/// Best-effort revoke of a previously provisioned session key at teardown.
/// A revoke failure is logged (with the non-secret id) and swallowed: teardown
/// must never be blocked by NyxID being briefly unreachable, and the key is
/// non-expiring so a janitor sweep is the backstop.
pub async fn revoke(client: &NyxIdClient, handle: &NyxidTokenHandle) {
    match client.revoke_api_key(&handle.key_id).await {
        Ok(()) => tracing::info!(
            key_id = %handle.key_id,
            "revoked per-session nyxid token"
        ),
        Err(error) => tracing::warn!(
            key_id = %handle.key_id,
            error = %error,
            "failed to revoke per-session nyxid token (swallowed; key is non-expiring, sweep is the backstop)"
        ),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const API_KEYS_PATH: &str = "/api/v1/api-keys";
    const TOKEN_PATH: &str = "/oauth/token";

    fn client(uri: &str) -> NyxIdClient {
        NyxIdClient::new(
            uri,
            "sa_client".to_string(),
            SecretString::from("sa_secret".to_string()),
            std::time::Duration::from_secs(30),
        )
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

    #[tokio::test]
    async fn provision_mints_and_returns_handle_plus_env_entries() {
        let server = MockServer::start().await;
        let session_id = bson::Uuid::new();
        Mock::given(method("POST"))
            .and(path(API_KEYS_PATH))
            .and(header("authorization", "Bearer user_raw_tok"))
            .and(body_string_contains(format!("fkst-session-{session_id}")))
            .and(body_string_contains("proxy"))
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
        let err = provision(&client(&server.uri()), bson::Uuid::new(), "o", &raw)
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
        let err = provision(&client(&server.uri()), bson::Uuid::new(), "o", &raw)
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
        let err = provision(&client(&server.uri()), bson::Uuid::new(), "o", &raw)
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
        let (handle, env) = provision(&client(&server.uri()), bson::Uuid::new(), "o", &raw)
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
    async fn revoke_is_best_effort_and_swallows_failures() {
        let server = MockServer::start().await;
        // Service token (revoke uses it) then a failing DELETE — revoke must
        // not panic or propagate.
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "svc", "token_type": "Bearer", "expires_in": 3600
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{API_KEYS_PATH}/k")))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let handle = NyxidTokenHandle {
            key_id: "k".to_string(),
            key_prefix: "nyxid_ag_xxx".to_string(),
        };
        // Returns unit regardless of the upstream failure.
        revoke(&client(&server.uri()), &handle).await;
    }

    #[tokio::test]
    async fn revoke_succeeds_on_204() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "svc", "token_type": "Bearer", "expires_in": 3600
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{API_KEYS_PATH}/k")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let handle = NyxidTokenHandle {
            key_id: "k".to_string(),
            key_prefix: "p".to_string(),
        };
        revoke(&client(&server.uri()), &handle).await;
    }
}
