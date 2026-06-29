//! Path-B NyxID token refresh building blocks (issue #299).
//!
//! A session outlives any single NyxID token, and the only offline path is the
//! token-exchange (binding_id -> ~5-min access token). So the control plane
//! re-exchanges the stored binding for a fresh short token and rotates it into
//! the running pod by PATCHing the per-session Secret's `nyxid-token` key
//! (mounted 0400; the kubelet propagates the change). The durable binding never
//! enters the pod.
//!
//! This module is the building blocks only: the periodic ticker, the
//! reactive-on-401 trigger, and stop-on-terminal are the job-watch driver's job.

use std::time::Duration;

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, Patch, PatchParams};
use secrecy::{ExposeSecret, SecretString};

use crate::nyxid_connect::{exchange_binding_for_token, BrokerClientConfig, ConnectError};
use crate::session_spec::creds::NYXID_TOKEN_FILE;

/// How often the driver should refresh: safely under the ~5-min token TTL.
pub const NYXID_REFRESH_INTERVAL: Duration = Duration::from_secs(4 * 60);

/// The per-session Secret name for a session id (mirrors the launcher).
fn secret_name(session_id: &str) -> String {
    format!("fkst-sess-{session_id}")
}

/// The strategic-merge patch body that rotates the `nyxid-token` key in place.
/// Pure (no API call) so the shape is unit-tested.
fn nyxid_token_patch(token: &SecretString) -> serde_json::Value {
    serde_json::json!({
        "stringData": { NYXID_TOKEN_FILE: token.expose_secret() }
    })
}

/// Errors refreshing a session token.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    /// The offline binding->token exchange failed.
    #[error("nyxid token exchange: {0}")]
    Exchange(#[from] ConnectError),
    /// Patching the per-session Secret failed.
    #[error("secret patch: {0}")]
    Patch(#[from] kube::Error),
}

/// Rotates the `nyxid-token` key of a live per-session Secret.
#[derive(Clone)]
pub struct SessionSecretWriter {
    client: kube::Client,
    namespace: String,
}

impl SessionSecretWriter {
    /// Build a writer bound to the sessions namespace.
    pub fn new(client: kube::Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    /// Patch the `nyxid-token` key of `fkst-sess-<session_id>` in place.
    pub async fn patch_nyxid_token(
        &self,
        session_id: &str,
        token: &SecretString,
    ) -> Result<(), kube::Error> {
        let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);
        secrets
            .patch(
                &secret_name(session_id),
                &PatchParams::default(),
                &Patch::Merge(nyxid_token_patch(token)),
            )
            .await?;
        Ok(())
    }
}

/// The per-session refresh handle: exchanges the stored binding for a fresh
/// short token and rotates it into the pod. The driver (job-watch) calls
/// [`Self::refresh_session_token`] on the [`NYXID_REFRESH_INTERVAL`] tick and
/// reactively on an observed 401.
pub struct NyxidRefresh {
    writer: SessionSecretWriter,
    http: reqwest::Client,
    base_url: String,
    broker: BrokerClientConfig,
    binding_id: SecretString,
    session_id: String,
}

impl NyxidRefresh {
    /// Assemble a refresh handle for one session.
    pub fn new(
        writer: SessionSecretWriter,
        http: reqwest::Client,
        base_url: impl Into<String>,
        broker: BrokerClientConfig,
        binding_id: SecretString,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            writer,
            http,
            base_url: base_url.into(),
            broker,
            binding_id,
            session_id: session_id.into(),
        }
    }

    /// Exchange the binding for a fresh ~5-min token and rotate it into the pod's
    /// Secret. Idempotent and cheap to call on a tick or reactively.
    pub async fn refresh_session_token(&self) -> Result<(), RefreshError> {
        let token =
            exchange_binding_for_token(&self.http, &self.base_url, &self.broker, &self.binding_id)
                .await?;
        self.writer
            .patch_nyxid_token(&self.session_id, &token)
            .await?;
        tracing::info!(
            session_id = %self.session_id,
            "nyxid refresh: rotated session token into the pod secret"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_body_rotates_only_the_nyxid_token_key() {
        let patch = nyxid_token_patch(&SecretString::from("fresh-token"));
        assert_eq!(patch["stringData"]["nyxid-token"], "fresh-token");
        // Only stringData is touched (no data/metadata clobber).
        assert_eq!(patch.as_object().unwrap().len(), 1);
        assert!(patch.get("stringData").is_some());
    }

    #[test]
    fn secret_name_matches_the_launcher_convention() {
        assert_eq!(secret_name("abc"), "fkst-sess-abc");
    }

    #[tokio::test]
    async fn exchange_binding_returns_the_access_token() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "access_token": "tok_5min" })),
            )
            .mount(&server)
            .await;

        let cfg = BrokerClientConfig {
            client_id: "id".to_string(),
            client_secret: SecretString::from("sec"),
            redirect_uri: "https://fkst/cb".to_string(),
        };
        let http = reqwest::Client::new();
        let token =
            exchange_binding_for_token(&http, &server.uri(), &cfg, &SecretString::from("bnd_x"))
                .await
                .expect("exchange succeeds");
        assert_eq!(token.expose_secret(), "tok_5min");
    }

    #[tokio::test]
    async fn exchange_binding_maps_a_rejection() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let cfg = BrokerClientConfig {
            client_id: "id".to_string(),
            client_secret: SecretString::from("sec"),
            redirect_uri: "https://fkst/cb".to_string(),
        };
        let http = reqwest::Client::new();
        let err =
            exchange_binding_for_token(&http, &server.uri(), &cfg, &SecretString::from("bnd_x"))
                .await
                .expect_err("a 400 is a rejection");
        assert!(matches!(err, ConnectError::Rejected(400)));
    }
}
