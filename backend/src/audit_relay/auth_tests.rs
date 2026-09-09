//! Credential tests: role separation, malformed headers, and the constant-time
//! comparison's correctness.

use axum::http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use super::*;

fn tokens() -> RelayTokens {
    RelayTokens::new(
        SecretString::from("write-secret".to_string()),
        SecretString::from("read-secret".to_string()),
    )
}

fn headers(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        HeaderValue::from_str(value).expect("header value"),
    );
    headers
}

#[test]
fn the_write_token_admits_only_write_endpoints() {
    let tokens = tokens();
    let presented = headers("Bearer write-secret");
    assert!(tokens.authorize(&presented, TokenRole::Write).is_ok());
    assert_eq!(
        tokens.authorize(&presented, TokenRole::Read),
        Err(AuthError::Rejected)
    );
}

#[test]
fn the_read_token_admits_only_the_read_endpoint() {
    let tokens = tokens();
    let presented = headers("Bearer read-secret");
    assert!(tokens.authorize(&presented, TokenRole::Read).is_ok());
    assert_eq!(
        tokens.authorize(&presented, TokenRole::Write),
        Err(AuthError::Rejected)
    );
}

#[test]
fn a_missing_header_is_missing_not_rejected() {
    assert_eq!(
        tokens().authorize(&HeaderMap::new(), TokenRole::Write),
        Err(AuthError::Missing)
    );
}

#[test]
fn a_malformed_header_is_refused() {
    for value in ["write-secret", "Basic write-secret", "Bearer", "Bearer    "] {
        assert_eq!(
            tokens().authorize(&headers(value), TokenRole::Write),
            Err(AuthError::Malformed),
            "`{value}` must be malformed"
        );
    }
}

#[test]
fn the_bearer_scheme_is_case_insensitive() {
    assert!(tokens()
        .authorize(&headers("bearer write-secret"), TokenRole::Write)
        .is_ok());
}

#[test]
fn a_prefix_of_the_secret_is_rejected() {
    // The property the constant-time comparison protects: a prefix must be as
    // wrong as any other wrong value, and must not be accepted by a length quirk.
    for candidate in ["write-secre", "write-secrets", "", "WRITE-SECRET"] {
        assert_eq!(
            tokens().authorize(&headers(&format!("Bearer {candidate}")), TokenRole::Write),
            Err(if candidate.is_empty() {
                AuthError::Malformed
            } else {
                AuthError::Rejected
            }),
            "`{candidate}` must not authenticate"
        );
    }
}

#[test]
fn constant_time_equality_agrees_with_ordinary_equality() {
    let cases: [(&[u8], &[u8]); 5] = [
        (b"", b""),
        (b"a", b"a"),
        (b"a", b"b"),
        (b"abc", b"abcd"),
        (b"abcd", b"abc"),
    ];
    for (left, right) in cases {
        assert_eq!(
            constant_time_eq(left, right),
            left == right,
            "{left:?} vs {right:?}"
        );
    }
}
