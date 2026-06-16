//! NyxID-backed LLM gateway: the concrete `LlmGateway` implementation.
//!
//! The gateway mints a service-account bearer (scope `llm:proxy`) via the
//! [`crate::nyxid::NyxIdClient`] and POSTs an OpenAI-compatible
//! chat-completions request to the operator-configured NyxID LLM gateway.
//!
//! Secret hygiene (mirroring `nyxid/mod.rs`): the bearer is exposed only at
//! request-build time and never captured into any error variant, `Debug`, or
//! log line. Model output and prompts are never logged here.

use std::fmt;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde::Deserialize;

use crate::llm::{LlmConfig, LlmError, LlmGateway};
use crate::nyxid::{NyxIdClient, NyxIdError};

/// OpenAI-compatible chat-completions path appended to the configured NyxID LLM
/// gateway base URL (operator sets FKST_HOSTED_LLM_GATEWAY_URL to NyxID's
/// `{base}/api/v1/llm/gateway/v1`). VERIFIED against NyxID `main`:
/// backend/src/handlers/llm_gateway.rs is a passthrough proxy of
/// `POST .../v1/chat/completions`; backend/src/services/llm_gateway_service.rs
/// builds `gateway_url = {base}/api/v1/llm/gateway/v1`; routing is by model name.
/// Confined here so the exact route stays swappable without touching the
/// generate path.
const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

/// NyxID-backed LLM gateway. Cheaply cloneable (the `NyxIdClient` is `Arc`-backed
/// and `reqwest::Client` shares its pool).
#[derive(Clone)]
pub struct NyxLlmGateway {
    http: reqwest::Client,
    nyxid: NyxIdClient,
    config: LlmConfig,
}

impl fmt::Debug for NyxLlmGateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NyxLlmGateway")
            .field("gateway_url", &self.config.gateway_url)
            .field("model", &self.config.model)
            .field("timeout", &self.config.timeout)
            .field("max_output_bytes", &self.config.max_output_bytes)
            .finish()
    }
}

impl NyxLlmGateway {
    /// Build a new gateway over the NyxID service client and wiring config.
    pub fn new(nyxid: NyxIdClient, config: LlmConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| LlmError::Http(format!("client build: {e}")))?;
        Ok(Self {
            http,
            nyxid,
            config,
        })
    }
}

/// OpenAI-compatible chat-completions response: `{choices:[{message:{content}}]}`.
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[async_trait]
impl LlmGateway for NyxLlmGateway {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        // Mint the service-account bearer (scope llm:proxy). A rejected
        // service credential is an auth failure; any other NyxID error is a
        // transport-level gateway failure. Neither carries the secret.
        let token = self.nyxid.service_token().await.map_err(|e| match e {
            NyxIdError::ServiceAuth => LlmError::Auth,
            other => LlmError::Http(format!("service token: {other}")),
        })?;

        let url = format!(
            "{}{}",
            self.config.gateway_url.trim_end_matches('/'),
            CHAT_COMPLETIONS_PATH
        );
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });

        let response = self
            .http
            .post(&url)
            .bearer_auth(token.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LlmError::Timeout
                } else {
                    // The reqwest Display never includes the bearer; the URL
                    // and model are non-secret routing config.
                    LlmError::Http(format!("send: {e}"))
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::error!(status = %status, "llm gateway rejected the service bearer");
            return Err(LlmError::Auth);
        }
        if !status.is_success() {
            tracing::error!(status = %status, "llm gateway returned a non-2xx status");
            return Err(LlmError::Http(format!("gateway status {status}")));
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::Malformed(format!("response body: {e}")))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| LlmError::Malformed("no choices in response".to_string()))?;
        tracing::debug!(
            output_bytes = content.len(),
            "llm gateway completion received"
        );
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::nyxid::TOKEN_PATH;

    /// Build a `NyxIdClient` pointed at `token_server` (answers the OAuth
    /// token endpoint) and a gateway pointed at `gateway_url`.
    fn gateway(token_base: &str, gateway_url: &str) -> NyxLlmGateway {
        let nyxid = NyxIdClient::new(
            token_base,
            "api-github",
            "sa_test".to_string(),
            SecretString::from("sas_secret".to_string()),
            Duration::from_secs(30),
        )
        .expect("nyxid client");
        NyxLlmGateway::new(
            nyxid,
            LlmConfig {
                gateway_url: gateway_url.to_string(),
                model: "test-model".to_string(),
                timeout: Duration::from_secs(10),
                max_output_bytes: 1_048_576,
            },
        )
        .expect("gateway")
    }

    /// Mount a service-token endpoint on `server`.
    async fn mount_token(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "svc_tok", "token_type": "Bearer", "expires_in": 3600
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn complete_returns_choice_content() {
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let gw_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(CHAT_COMPLETIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [ { "message": { "role": "assistant", "content": "hello draft" } } ]
            })))
            .mount(&gw_server)
            .await;

        let gw = gateway(&token_server.uri(), &gw_server.uri());
        let out = gw.complete("sys", "usr").await.expect("completion");
        assert_eq!(out, "hello draft");
    }

    #[tokio::test]
    async fn complete_maps_401_to_auth() {
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let gw_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(CHAT_COMPLETIONS_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&gw_server)
            .await;

        let gw = gateway(&token_server.uri(), &gw_server.uri());
        let err = gw.complete("sys", "usr").await.expect_err("must fail");
        assert!(matches!(err, LlmError::Auth), "got {err:?}");
    }

    #[tokio::test]
    async fn complete_maps_non_2xx_to_http() {
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let gw_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(CHAT_COMPLETIONS_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&gw_server)
            .await;

        let gw = gateway(&token_server.uri(), &gw_server.uri());
        let err = gw.complete("sys", "usr").await.expect_err("must fail");
        assert!(matches!(err, LlmError::Http(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn complete_maps_malformed_body_to_malformed() {
        let token_server = MockServer::start().await;
        mount_token(&token_server).await;
        let gw_server = MockServer::start().await;
        // 200 but the shape is wrong (no choices array).
        Mock::given(method("POST"))
            .and(path(CHAT_COMPLETIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "unexpected": "shape"
            })))
            .mount(&gw_server)
            .await;

        let gw = gateway(&token_server.uri(), &gw_server.uri());
        let err = gw.complete("sys", "usr").await.expect_err("must fail");
        assert!(matches!(err, LlmError::Malformed(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejected_service_credentials_map_to_auth() {
        // The token endpoint rejects the service credential => LlmError::Auth.
        let token_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(TOKEN_PATH))
            .respond_with(ResponseTemplate::new(401))
            .mount(&token_server)
            .await;
        let gw_server = MockServer::start().await;

        let gw = gateway(&token_server.uri(), &gw_server.uri());
        let err = gw.complete("sys", "usr").await.expect_err("must fail");
        assert!(matches!(err, LlmError::Auth), "got {err:?}");
    }

    #[tokio::test]
    async fn no_error_or_debug_ever_contains_the_service_secret() {
        const SECRET: &str = "sas_should_never_leak_12345";
        let nyxid = NyxIdClient::new(
            "http://127.0.0.1:1",
            "api-github",
            "sa_test".to_string(),
            SecretString::from(SECRET.to_string()),
            Duration::from_secs(30),
        )
        .expect("client");
        let gw = NyxLlmGateway::new(
            nyxid,
            LlmConfig {
                gateway_url: "http://127.0.0.1:1".to_string(),
                model: "m".to_string(),
                timeout: Duration::from_secs(1),
                max_output_bytes: 1024,
            },
        )
        .expect("gateway");
        // Unreachable token endpoint surfaces a live transport error.
        let err = gw.complete("s", "u").await.expect_err("unreachable");
        assert!(!format!("{err}").contains(SECRET), "Display leaked");
        assert!(!format!("{err:?}").contains(SECRET), "Debug leaked");
        assert!(!format!("{gw:?}").contains(SECRET), "gateway Debug leaked");
    }
}
