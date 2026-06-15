//! Webhook signature verification (issue #108): HMAC-SHA256 over the RAW body,
//! constant-time compared against `X-Hub-Signature-256`.
//!
//! Hand-rolled with `hmac` + `sha2` (no GitHub SDK — consistent with the
//! module's hand-rolled `reqwest` + `jsonwebtoken` transport). Verification MUST
//! run on the exact bytes GitHub signed, BEFORE any JSON parse: a
//! deserialize-then-reserialize changes the bytes and breaks the MAC.

use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Header carrying GitHub's HMAC-SHA256 signature (`sha256=<hex>`).
pub(super) const SIGNATURE_HEADER: &str = "x-hub-signature-256";

type HmacSha256 = Hmac<Sha256>;

/// Verify the `X-Hub-Signature-256` HMAC over the RAW body, in constant time.
///
/// Returns `true` only when the header is present, well-formed (`sha256=<hex>`),
/// and the recomputed MAC matches. The comparison uses the `hmac` crate's
/// constant-time `verify_slice`, which avoids the early-exit timing leak a
/// byte-by-byte `==` would have. Verification runs on `raw_body` exactly as
/// received, BEFORE any JSON parse.
pub(super) fn verify_signature(secret: &[u8], headers: &HeaderMap, raw_body: &[u8]) -> bool {
    let Some(header_value) = headers.get(SIGNATURE_HEADER) else {
        return false;
    };
    let Ok(header_str) = header_value.to_str() else {
        return false;
    };
    // GitHub formats the header as `sha256=<hex>`; reject anything else.
    let Some(hex_sig) = header_str.strip_prefix("sha256=") else {
        return false;
    };
    let Some(expected) = decode_hex(hex_sig) else {
        return false;
    };

    // A fresh MAC per request; the key length is unconstrained for HMAC.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(raw_body);
    mac.verify_slice(&expected).is_ok()
}

/// Decode a lowercase/uppercase hex string into bytes. Returns `None` on an odd
/// length or a non-hex digit (a malformed signature is simply unverifiable).
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Compute the `sha256=<hex>` header value for `body` under `secret`. Lives here
/// (not in the test module) so both the verify tests and the integration tests
/// share one signing path. NOT used in production (GitHub signs the webhooks).
#[cfg(test)]
pub(super) fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("key");
    let hex: String = {
        mac.update(body);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    format!("sha256={hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(sig: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(s) = sig {
            h.insert(SIGNATURE_HEADER, s.parse().unwrap());
        }
        h
    }

    #[test]
    fn valid_signature_verifies() {
        let secret = b"whsec_test";
        let body = br#"{"action":"created"}"#;
        let sig = sign(secret, body);
        assert!(verify_signature(secret, &headers_with(Some(&sig)), body));
    }

    #[test]
    fn wrong_secret_fails_verification() {
        let body = br#"{"action":"created"}"#;
        let sig = sign(b"whsec_test", body);
        assert!(
            !verify_signature(b"whsec_WRONG", &headers_with(Some(&sig)), body),
            "a different secret must not verify"
        );
    }

    #[test]
    fn tampered_body_fails_verification() {
        let secret = b"whsec_test";
        let sig = sign(secret, br#"{"action":"created"}"#);
        // Same signature, different body (the deserialize-reserialize hazard the
        // raw-bytes-before-parse ordering exists to prevent).
        assert!(!verify_signature(
            secret,
            &headers_with(Some(&sig)),
            br#"{"action":"deleted"}"#
        ));
    }

    #[test]
    fn missing_signature_header_fails() {
        assert!(!verify_signature(b"whsec_test", &headers_with(None), b"{}"));
    }

    #[test]
    fn malformed_signature_header_fails() {
        // No `sha256=` prefix.
        assert!(!verify_signature(
            b"whsec_test",
            &headers_with(Some("deadbeef")),
            b"{}"
        ));
        // Odd-length hex.
        assert!(!verify_signature(
            b"whsec_test",
            &headers_with(Some("sha256=abc")),
            b"{}"
        ));
        // Non-hex digit.
        assert!(!verify_signature(
            b"whsec_test",
            &headers_with(Some("sha256=zz")),
            b"{}"
        ));
    }

    #[test]
    fn decode_hex_round_trips() {
        assert_eq!(decode_hex("00ff10"), Some(vec![0x00, 0xff, 0x10]));
        assert_eq!(decode_hex("ABCD"), Some(vec![0xab, 0xcd]));
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
    }
}
