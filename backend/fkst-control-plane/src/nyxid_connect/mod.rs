//! NyxID connect-at-install: the durable per-owner broker-binding store + the
//! OAuth consent helpers (milestone #9, issue #297).
//!
//! A GitHub webhook carries no user token, but each pod-per-session run needs a
//! NyxID token for the substrate's LLM/Ornn calls. The owner authorizes
//! fkst-hosted ONCE through a NyxID OAuth consent against a broker-capable
//! client; NyxID returns a durable, storable `binding_id` (`bnd_*`, hash-stored
//! server-side). We persist it here, keyed on the **GitHub owner login** so a
//! later webhook (which knows the repo owner) can resolve it; nyxid-refresh then
//! exchanges the binding OFFLINE for short session tokens.
//!
//! Modeling note: the binding is associated with a GitHub owner login via the
//! `owner` query param at connect time. The `binding_id` is a secret and is
//! never logged.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rand::RngCore;
use secrecy::SecretString;
use serde::Deserialize;

/// The OAuth scope requesting a broker binding from NyxID.
pub const BROKER_BINDING_SCOPE: &str = "urn:nyxid:scope:broker_binding";

/// A stored durable broker binding for one GitHub owner.
#[derive(Debug, Clone)]
pub struct BindingRecord {
    /// The durable `bnd_*` id exchanged offline for short tokens. SECRET.
    pub binding_id: SecretString,
    /// The NyxID subject that authorized the binding (for observability).
    pub nyxid_user: String,
}

/// A pending consent: the owner + authorizing user a `state` value stands for,
/// stashed at `/connect` and consumed at `/connect/callback`.
#[derive(Clone)]
struct PendingConnect {
    owner: String,
    nyxid_user: String,
}

/// In-memory per-owner broker-binding store + the short-lived OAuth `state` map.
/// Cheap to clone (an `Arc` inside); one instance is shared via `AppState`.
#[derive(Clone)]
pub struct BrokerBindingStore {
    bindings: Arc<RwLock<HashMap<String, BindingRecord>>>,
    pending: Arc<RwLock<HashMap<String, PendingConnect>>>,
}

impl BrokerBindingStore {
    /// A fresh, empty store.
    pub fn new() -> Self {
        Self {
            bindings: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Begin a consent: stash `(owner, nyxid_user)` under a fresh random `state`
    /// and return it for the authorize redirect.
    pub fn begin_connect(&self, owner: &str, nyxid_user: &str) -> String {
        let state = random_state();
        self.pending.write().expect("pending lock").insert(
            state.clone(),
            PendingConnect {
                owner: owner.to_string(),
                nyxid_user: nyxid_user.to_string(),
            },
        );
        state
    }

    /// Consume a pending consent by `state`, returning the `(owner, nyxid_user)`
    /// it stands for. `None` for an unknown/replayed state.
    fn take_pending(&self, state: &str) -> Option<PendingConnect> {
        self.pending.write().expect("pending lock").remove(state)
    }

    /// Persist a binding for a GitHub owner login (overwrites any prior one).
    pub fn store_binding(&self, owner: &str, record: BindingRecord) {
        self.bindings
            .write()
            .expect("bindings lock")
            .insert(owner.to_string(), record);
    }

    /// The stored binding for a GitHub owner login, if any.
    pub fn binding_for_owner(&self, owner: &str) -> Option<BindingRecord> {
        self.bindings
            .read()
            .expect("bindings lock")
            .get(owner)
            .cloned()
    }
}

impl Default for BrokerBindingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A cryptographically-random opaque `state` (CSRF) token, hex-encoded.
fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Errors completing the consent exchange.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// Pod dispatch / connect is not configured (no broker client).
    #[error("nyxid connect is not configured")]
    NotConfigured,
    /// The callback `state` is unknown or already used.
    #[error("unknown or expired oauth state")]
    UnknownState,
    /// The NyxID token endpoint call failed.
    #[error("token exchange transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// NyxID returned a non-success status to the token exchange.
    #[error("token exchange rejected: status {0}")]
    Rejected(u16),
    /// The token response carried no `binding_id` (the client lacks broker
    /// capability, or the contract changed).
    #[error("token response carried no binding_id")]
    NoBinding,
}

/// The broker OAuth client settings (present only when all three env vars are
/// set). `client_secret` is a secret and never logged.
#[derive(Clone)]
pub struct BrokerClientConfig {
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_uri: String,
}

/// Build the NyxID authorize URL the browser is redirected to at `/connect`.
pub fn authorize_url(base_url: &str, cfg: &BrokerClientConfig, state: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!(
        "{base}/oauth/authorize?response_type=code&client_id={client}&redirect_uri={redirect}&scope={scope}&state={state}",
        client = urlencode(&cfg.client_id),
        redirect = urlencode(&cfg.redirect_uri),
        scope = urlencode(BROKER_BINDING_SCOPE),
    )
}

/// The relevant fields of NyxID's `/oauth/token` response. Tolerant: a
/// broker-capable client gets a `binding_id`; other fields are ignored.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    binding_id: Option<String>,
}

/// Exchange an authorization `code` at `{base}/oauth/token` and consume the
/// matching pending `state`, returning the captured binding + the owner it
/// belongs to. The store records nothing here — the caller persists it.
pub async fn complete_callback(
    http: &reqwest::Client,
    base_url: &str,
    cfg: &BrokerClientConfig,
    store: &BrokerBindingStore,
    code: &str,
    state: &str,
) -> Result<(String, BindingRecord), ConnectError> {
    use secrecy::ExposeSecret;

    let pending = store
        .take_pending(state)
        .ok_or(ConnectError::UnknownState)?;
    let url = format!("{}/oauth/token", base_url.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &cfg.redirect_uri),
            ("client_id", &cfg.client_id),
            ("client_secret", cfg.client_secret.expose_secret()),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(ConnectError::Rejected(resp.status().as_u16()));
    }
    let body: TokenResponse = resp.json().await?;
    let binding_id = body.binding_id.ok_or(ConnectError::NoBinding)?;
    let record = BindingRecord {
        binding_id: SecretString::from(binding_id),
        nyxid_user: pending.nyxid_user,
    };
    Ok((pending.owner, record))
}

/// Minimal percent-encoding for the query components we build (RFC 3986
/// unreserved chars pass through; everything else is `%XX`). Avoids a new dep.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn cfg() -> BrokerClientConfig {
        BrokerClientConfig {
            client_id: "fkst-broker".to_string(),
            client_secret: SecretString::from("shh"),
            redirect_uri: "https://fkst/cb".to_string(),
        }
    }

    #[test]
    fn store_round_trips_and_overwrites_per_owner() {
        let store = BrokerBindingStore::new();
        assert!(store.binding_for_owner("acme").is_none());
        store.store_binding(
            "acme",
            BindingRecord {
                binding_id: SecretString::from("bnd_1"),
                nyxid_user: "u1".to_string(),
            },
        );
        assert_eq!(
            store
                .binding_for_owner("acme")
                .unwrap()
                .binding_id
                .expose_secret(),
            "bnd_1"
        );
        store.store_binding(
            "acme",
            BindingRecord {
                binding_id: SecretString::from("bnd_2"),
                nyxid_user: "u1".to_string(),
            },
        );
        assert_eq!(
            store
                .binding_for_owner("acme")
                .unwrap()
                .binding_id
                .expose_secret(),
            "bnd_2"
        );
    }

    #[test]
    fn pending_state_is_single_use() {
        let store = BrokerBindingStore::new();
        let state = store.begin_connect("acme", "u1");
        assert!(store.take_pending(&state).is_some());
        assert!(store.take_pending(&state).is_none(), "state is single-use");
    }

    #[test]
    fn authorize_url_carries_client_redirect_scope_state() {
        let url = authorize_url("https://nyx/", &cfg(), "st8");
        assert!(url.starts_with("https://nyx/oauth/authorize?"));
        assert!(url.contains("client_id=fkst-broker"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Ffkst%2Fcb"));
        assert!(url.contains("scope=urn%3Anyxid%3Ascope%3Abroker_binding"));
        assert!(url.contains("state=st8"));
    }

    #[tokio::test]
    async fn callback_exchanges_code_and_returns_binding_for_owner() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "at",
                "binding_id": "bnd_live"
            })))
            .mount(&server)
            .await;

        let store = BrokerBindingStore::new();
        let state = store.begin_connect("acme", "u1");
        let http = reqwest::Client::new();
        let (owner, record) =
            complete_callback(&http, &server.uri(), &cfg(), &store, "the-code", &state)
                .await
                .expect("callback completes");
        assert_eq!(owner, "acme");
        assert_eq!(record.binding_id.expose_secret(), "bnd_live");
    }

    #[tokio::test]
    async fn callback_rejects_an_unknown_state() {
        let store = BrokerBindingStore::new();
        let http = reqwest::Client::new();
        let err = complete_callback(&http, "https://nyx", &cfg(), &store, "c", "bogus")
            .await
            .expect_err("unknown state must fail");
        assert!(matches!(err, ConnectError::UnknownState));
    }
}
