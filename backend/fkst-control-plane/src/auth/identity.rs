//! Proxy-trusted identity decoding.
//!
//! fkst-hosted is deployed as a NyxID *downstream* service: it has no public
//! ingress and is reachable only via the NyxID proxy (see the deployment
//! manifests — the backend `Service` is `ClusterIP` with no `Ingress`). The
//! proxy authenticates the caller and forwards the verified identity in
//! request headers.
//!
//! Therefore this module **decodes** the injected `X-NyxID-Identity-Token` JWT
//! WITHOUT verifying its signature and WITHOUT fetching JWKS. This is safe — and
//! deliberate — because the proxy has already verified the token before
//! forwarding it (mirroring Ornn's `decodeJwtPayload`, whose comment reads
//! *"Safe because NyxID proxy has already verified the token before
//! forwarding"*). A token with a bad signature but a structurally valid payload
//! is accepted on purpose: fkst-hosted does not re-verify what the proxy
//! already verified, and never makes a network call to do so.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::Deserialize;

/// Identity claims read from the proxy-injected `X-NyxID-Identity-Token`.
///
/// Only `sub` is required; the array claims default to empty when absent so a
/// minimal token (or one missing a claim) still yields a usable context. Extra
/// claims are ignored.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IdentityClaims {
    /// Subject — the authenticated user identifier.
    pub sub: String,
    /// Optional email.
    #[serde(default)]
    pub email: Option<String>,
    /// Optional display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Role assignments (NyxID-managed). Carried through for observability;
    /// authorization is driven by `permissions`, not a local role mapping.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Fine-grained `fkst:*` permissions assigned by NyxID. These — not roles —
    /// drive RBAC enforcement (see `authz::permissions`).
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Group memberships (NyxID-managed). Carried through for observability.
    #[serde(default)]
    pub groups: Vec<String>,
}

/// Decode the payload of a proxy-injected identity JWT WITHOUT verifying the
/// signature.
///
/// Splits the compact JWS on `.` (expecting `header.payload.signature`),
/// base64url-decodes part 1 (the payload), and parses it as `IdentityClaims`.
/// Returns `None` when the token is not a well-formed three-part JWT, the
/// payload is not valid base64url, or the JSON does not deserialize (e.g. a
/// missing `sub`). No signature check and no network access ever occur.
pub fn decode_identity_token(raw: &str) -> Option<IdentityClaims> {
    let mut parts = raw.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    // A compact JWS has exactly three parts; reject anything else as malformed
    // so a stray dot-free string never decodes to a claims object.
    if parts.next().is_none() || parts.next().is_some() {
        return None;
    }

    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice::<IdentityClaims>(&decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a compact JWT-shaped string from a payload JSON value and an
    /// arbitrary (unverified) signature segment. The header is a fixed RS256
    /// stub — it is never inspected.
    fn jwt_with(payload: &serde_json::Value, signature: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{header}.{body}.{signature}")
    }

    #[test]
    fn decodes_full_claims() {
        let token = jwt_with(
            &serde_json::json!({
                "sub": "user-1",
                "email": "u@example.com",
                "name": "User One",
                "roles": ["org-admin"],
                "permissions": ["fkst:goal:read", "fkst:goal:create"],
                "groups": ["g1"],
            }),
            "ignored-signature",
        );
        let claims = decode_identity_token(&token).expect("valid payload decodes");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.email.as_deref(), Some("u@example.com"));
        assert_eq!(claims.name.as_deref(), Some("User One"));
        assert_eq!(claims.roles, vec!["org-admin"]);
        assert_eq!(
            claims.permissions,
            vec!["fkst:goal:read".to_string(), "fkst:goal:create".to_string()]
        );
        assert_eq!(claims.groups, vec!["g1"]);
    }

    #[test]
    fn missing_arrays_default_to_empty() {
        let token = jwt_with(&serde_json::json!({ "sub": "user-2" }), "sig");
        let claims = decode_identity_token(&token).expect("minimal payload decodes");
        assert_eq!(claims.sub, "user-2");
        assert!(claims.roles.is_empty());
        assert!(claims.permissions.is_empty());
        assert!(claims.groups.is_empty());
        assert!(claims.email.is_none());
    }

    /// The contract: a token whose signature segment is garbage but whose
    /// payload is structurally valid is STILL accepted. The proxy already
    /// verified the token; fkst-hosted trusts it and never re-checks the
    /// signature (and crucially makes no JWKS/network call to do so — there is
    /// no async, no client, nothing to call in this pure function).
    #[test]
    fn bad_signature_but_valid_payload_is_accepted() {
        let token = jwt_with(
            &serde_json::json!({ "sub": "trusted-by-proxy", "permissions": ["fkst:admin"] }),
            "this-signature-is-not-valid-and-is-never-checked",
        );
        let claims = decode_identity_token(&token).expect("trusted payload is accepted");
        assert_eq!(claims.sub, "trusted-by-proxy");
        assert_eq!(claims.permissions, vec!["fkst:admin"]);
    }

    #[test]
    fn rejects_non_jwt_shapes() {
        // Not three parts.
        assert!(decode_identity_token("only.two").is_none());
        assert!(decode_identity_token("a.b.c.d").is_none());
        assert!(decode_identity_token("no-dots-at-all").is_none());
    }

    #[test]
    fn rejects_invalid_base64_payload() {
        // Valid three-part shape, but the payload is not base64url.
        assert!(decode_identity_token("hdr.!!!not-base64!!!.sig").is_none());
    }

    #[test]
    fn rejects_payload_missing_subject() {
        let token = jwt_with(&serde_json::json!({ "email": "x@example.com" }), "sig");
        assert!(decode_identity_token(&token).is_none());
    }
}
