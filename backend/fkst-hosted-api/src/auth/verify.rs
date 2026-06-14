//! JWT token verification against NyxID-issued RS256 access tokens.

use jsonwebtoken::{decode, Algorithm, Validation};
use secrecy::SecretString;
use serde::Deserialize;

use super::jwks::JwksCache;
use super::{AuthContext, AuthError, NyxIdAuthSettings};

/// Claims expected in a NyxID access token. All fields are optional except
/// `sub` (the subject identifier). Extra claims are ignored via `serde(default)`.
#[derive(Debug, Deserialize)]
struct NyxIdClaims {
    /// Subject — the user or service account identifier. Required.
    sub: String,
    /// Issuer. Validated by `jsonwebtoken::Validation`.
    #[allow(dead_code)]
    iss: Option<String>,
    /// Audience. Validated by `jsonwebtoken::Validation`.
    #[allow(dead_code)]
    aud: Option<serde_json::Value>,
    /// Expiration. Validated by `jsonwebtoken::Validation`.
    #[allow(dead_code)]
    exp: Option<u64>,
    /// Issued-at.
    #[allow(dead_code)]
    iat: Option<u64>,
    /// Token type: must be `"access"`. Refresh and ID tokens are rejected.
    token_type: Option<String>,
    /// OAuth2 scope string, space-separated.
    scope: Option<String>,
    /// Role assignments.
    #[serde(default)]
    roles: Vec<String>,
    /// Group memberships.
    #[serde(default)]
    groups: Vec<String>,
    /// Fine-grained permissions.
    #[serde(default)]
    permissions: Vec<String>,
    /// Session ID.
    sid: Option<String>,
    /// Service account flag: `true` when the token represents a service account.
    sa: Option<bool>,
    /// Delegation / token-exchange `act` claim. Opaque; forwarded as-is.
    act: Option<serde_json::Value>,
    /// Key ID in the JWKS. Not read directly (kid is extracted from the header
    /// first), but kept for completeness.
    #[allow(dead_code)]
    kid: Option<String>,
}

/// Verifier holds the JWKS cache and NyxID settings and provides the
/// `verify` method that validates a bearer token.
#[derive(Debug)]
pub struct Verifier {
    jwks: JwksCache,
    issuer: String,
    audience: String,
}

impl Verifier {
    /// Build a verifier from the NyxID auth settings. The JWKS cache is lazy:
    /// no network call is made here.
    pub fn new(settings: &NyxIdAuthSettings) -> Self {
        Self {
            jwks: JwksCache::new(&settings.base_url, settings.jwks_cache_ttl),
            issuer: settings.issuer.clone(),
            audience: settings.audience.clone(),
        }
    }

    /// Verify a raw bearer token string. Returns the decoded `AuthContext` on
    /// success, or an `AuthError` on failure.
    ///
    /// Fixed set of `InvalidToken` reason strings (static `&str`):
    /// - `"missing subject"`
    /// - `"token is not an access token"`
    /// - `"missing key id"`
    /// - `"unknown key id"`
    pub async fn verify(&self, bearer: &str) -> Result<AuthContext, AuthError> {
        // First pass: decode the header without verification to extract `kid`.
        let header = jsonwebtoken::decode_header(bearer)
            .map_err(|_| AuthError::InvalidToken("malformed token header"))?;

        if header.alg != Algorithm::RS256 {
            return Err(AuthError::InvalidToken("unsupported algorithm"));
        }

        let kid = header
            .kid
            .ok_or(AuthError::InvalidToken("missing key id"))?;

        // Look up the key from the JWKS cache (may trigger a fetch).
        let decoding_key = self.jwks.key_for(&kid).await?;

        // Build validation parameters.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        // Default leeway is 60 seconds for exp/nbf; that is fine.

        let token_data =
            decode::<NyxIdClaims>(bearer, &decoding_key, &validation).map_err(map_jwt_error)?;

        let claims = token_data.claims;

        // Enforce token_type == "access".
        match claims.token_type.as_deref() {
            Some("access") => {}
            Some(_) => return Err(AuthError::InvalidToken("token is not an access token")),
            None => return Err(AuthError::InvalidToken("token is not an access token")),
        }

        let scopes = claims
            .scope
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();

        Ok(AuthContext {
            user_id: claims.sub,
            scopes,
            roles: claims.roles,
            groups: claims.groups,
            permissions: claims.permissions,
            session_id: claims.sid,
            is_service_account: claims.sa == Some(true),
            delegation: claims.act,
            raw_token: SecretString::new(bearer.to_string().into()),
        })
    }

    /// Return a reference to the inner JWKS cache (for testing / inspection).
    pub fn jwks_cache(&self) -> &JwksCache {
        &self.jwks
    }
}

/// Map `jsonwebtoken::errors::Error` to `AuthError::InvalidToken` with a
/// fixed set of reason strings. Never echoes token material.
fn map_jwt_error(e: jsonwebtoken::errors::Error) -> AuthError {
    let msg = e.to_string();
    // The error kind gives us a stable categorisation.
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::ExpiredSignature => AuthError::InvalidToken("token has expired"),
        ErrorKind::InvalidIssuer => AuthError::InvalidToken("invalid issuer"),
        ErrorKind::InvalidAudience => AuthError::InvalidToken("invalid audience"),
        ErrorKind::InvalidToken => AuthError::InvalidToken("invalid token signature"),
        _ => {
            tracing::debug!(error = %msg, "JWT validation failed");
            AuthError::InvalidToken("invalid token signature")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_jwt_error_categorises_known_kinds() {
        use jsonwebtoken::errors::ErrorKind;
        let cases: Vec<(ErrorKind, &'static str)> = vec![
            (ErrorKind::ExpiredSignature, "token has expired"),
            (ErrorKind::InvalidIssuer, "invalid issuer"),
            (ErrorKind::InvalidAudience, "invalid audience"),
        ];
        for (kind, expected) in cases {
            let err = jsonwebtoken::errors::Error::from(kind);
            let mapped = map_jwt_error(err);
            match mapped {
                AuthError::InvalidToken(reason) => assert_eq!(reason, expected),
                other => panic!("expected InvalidToken, got {other:?}"),
            }
        }
    }
}
