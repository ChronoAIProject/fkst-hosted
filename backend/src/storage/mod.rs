//! chrono-storage object-store client + its NyxID service-account token provider.
//!
//! chrono-storage is a MinIO/S3 front reached over plain HTTPS **behind the NyxID
//! proxy**; every call carries a NyxID service-account OAuth2 client-credentials
//! access token as `Authorization: Bearer`. This module is the self-contained,
//! wiremock-tested machinery for that: [`config::ChronoStorageConfig`] (the
//! optional, fail-closed env config), [`nyxid_token::NyxidSaTokenProvider`] (the
//! cached/refreshed token minter), and [`chrono_storage::ChronoStorageClient`]
//! (upload / download / delete / copy / bucket-readiness).
//!
//! The feature is OPTIONAL: [`try_from_env`] returns `None` when it is not
//! configured, so a control plane with no storage config runs exactly as before.
//! There is no in-pod wiring here yet — this is the client library only.
//!
//! Secret hygiene is the hard rule of this module: the client secret and the
//! minted access token live only in [`secrecy::SecretString`]s, ride only request
//! bodies / `Authorization` headers (never a URL or a log line), and every error
//! carries at most a numeric HTTP status or a URL-free transport category — never
//! a token, a secret, or a response body.

use std::time::Duration;

pub mod chrono_storage;
pub mod config;
pub mod nyxid_token;

pub use chrono_storage::ChronoStorageClient;
pub use config::ChronoStorageConfig;
pub use nyxid_token::NyxidSaTokenProvider;

/// Per-request timeout for every chrono-storage / NyxID call (mirrors the
/// GitHub-App transport's budget).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Typed failure for the storage client + token provider.
///
/// Deliberately narrow: each variant carries at most a numeric HTTP status or a
/// URL-free transport category. It NEVER carries the bearer token, the client
/// secret, a signed URL, or a response body — so logging or rendering an error
/// can never leak a credential (asserted by the `secret*never*leak` tests).
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The NyxID token request failed at the transport layer (connect/timeout/
    /// body). The detail is a fixed category string, never the URL or body.
    #[error("nyxid token request failed: {0}")]
    TokenTransport(String),
    /// The NyxID token endpoint returned a non-success HTTP status.
    #[error("nyxid token endpoint returned status {status}")]
    TokenStatus { status: u16 },
    /// The NyxID token response body was missing `access_token` or unparseable.
    #[error("nyxid token response was malformed")]
    TokenMalformed,
    /// A chrono-storage call failed at the transport layer. The detail is a
    /// fixed category string, never the URL (which may carry a signed token) or
    /// the body.
    #[error("chrono-storage request failed: {0}")]
    Transport(String),
    /// A chrono-storage call returned a non-success HTTP status.
    #[error("chrono-storage returned status {status}")]
    Status { status: u16 },
    /// A chrono-storage success body did not match the expected shape.
    #[error("chrono-storage response was malformed")]
    Malformed,
}

/// Render a [`reqwest::Error`] as a fixed, URL-free category string.
///
/// why: a `reqwest::Error`'s own `Display` embeds the request URL, and a
/// chrono-storage download URL is a short-lived *signed* URL. Reducing the error
/// to a coarse category guarantees no URL (and therefore no signature, object
/// key, or token) can ever reach a log line or a returned error.
pub(crate) fn scrub_transport(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "request timed out".to_string()
    } else if e.is_connect() {
        "connection failed".to_string()
    } else if e.is_body() {
        "request/response body error".to_string()
    } else if e.is_decode() {
        "response decode error".to_string()
    } else if e.is_request() {
        "request error".to_string()
    } else {
        "transport error".to_string()
    }
}

/// Build a [`ChronoStorageClient`] from an already-resolved [`ChronoStorageConfig`]
/// over the shared, timeout-bounded pooled HTTP client.
///
/// Two callers reach for this once the config has been loaded + fail-closed-
/// validated at startup, so the client is built from the SAME parsed values (no
/// second env pass, unlike [`try_from_env`]):
/// - the in-pod log uploader, which resolves the WRITE-ONLY SA config from its
///   mounted creds;
/// - the control plane's log-download path, which reuses `Config::storage`.
///
/// Kept here so the single [`build_http_client`] recipe (timeout + User-Agent) is
/// shared by every entry point.
pub fn client_from_config(config: ChronoStorageConfig) -> ChronoStorageClient {
    ChronoStorageClient::new(build_http_client(), config)
}

/// A pooled HTTP client for chrono-storage + NyxID, built with a bounded timeout
/// and a User-Agent (some proxies reject a UA-less request). `reqwest::Client`
/// holds a connection pool and is cheap to clone, so the same client is shared by
/// the token provider and the storage client.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("fkst-hosted")
        .build()
        .expect("build chrono-storage http client")
}

/// Build a [`ChronoStorageClient`] from the process environment, or `None` when
/// the feature is not configured.
///
/// `None` covers both "entirely unset" (the common case — log streaming stays
/// disabled) and, defensively, an invalid/partial config: the standalone
/// constructor fails OPEN (a warning, feature disabled) rather than panicking,
/// because chrono-storage is an optional add-on and must never take the control
/// plane down. (The fail-CLOSED, process-refusing check on a partial config lives
/// in [`crate::config::Config::from_vars`], which surfaces the misconfiguration
/// at startup.)
pub fn try_from_env() -> Option<ChronoStorageClient> {
    let vars: Vec<(String, String)> = std::env::vars().collect();
    match ChronoStorageConfig::from_vars(&vars) {
        Ok(Some(config)) => {
            tracing::info!(bucket = %config.bucket, "chrono-storage log streaming enabled");
            Some(ChronoStorageClient::new(build_http_client(), config))
        }
        Ok(None) => {
            tracing::debug!("chrono-storage not configured; log streaming disabled");
            None
        }
        Err(e) => {
            // Never logs a secret: `AppError::Config` carries only the missing
            // variable names.
            tracing::warn!(error = %e, "chrono-storage config invalid; log streaming disabled");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The complete, fixed vocabulary `scrub_transport` may emit — none of which
    /// contains a URL, a signature, or a token.
    const SCRUB_CATEGORIES: &[&str] = &[
        "request timed out",
        "connection failed",
        "request/response body error",
        "response decode error",
        "request error",
        "transport error",
    ];

    #[test]
    fn scrub_transport_reduces_to_a_url_free_category() {
        // A malformed URL that embeds a fake signature yields a builder error
        // whose own `Display` would carry the URL; the scrub helper must reduce
        // it to a fixed category with no trace of the URL.
        let err = reqwest::Client::new()
            .get("http://[bad-signature-SECRET")
            .build()
            .expect_err("malformed URL must fail to build");
        let scrubbed = scrub_transport(&err);
        assert!(!scrubbed.contains("SECRET"), "{scrubbed}");
        assert!(!scrubbed.contains("bad-signature"), "{scrubbed}");
        assert!(
            SCRUB_CATEGORIES.contains(&scrubbed.as_str()),
            "unexpected category: {scrubbed}"
        );
    }

    #[test]
    fn storage_error_display_carries_only_status_or_category() {
        // The status variants render just the number; the malformed variants
        // carry no detail at all. None can embed a token or body.
        assert_eq!(
            StorageError::TokenStatus { status: 503 }.to_string(),
            "nyxid token endpoint returned status 503"
        );
        assert_eq!(
            StorageError::Status { status: 404 }.to_string(),
            "chrono-storage returned status 404"
        );
        // The malformed variants carry no detail, so a body/secret cannot ride
        // in one.
        assert_eq!(
            StorageError::TokenMalformed.to_string(),
            "nyxid token response was malformed"
        );
        assert_eq!(
            StorageError::Malformed.to_string(),
            "chrono-storage response was malformed"
        );
    }
}
