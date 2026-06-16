//! Shared-secret authentication for the internal controller<->worker routes.
//!
//! The secret is compared in CONSTANT TIME (no early return on the first
//! mismatching byte) so the comparison cannot be used as a timing oracle to
//! recover the token byte-by-byte. The token is NEVER logged.

use secrecy::{ExposeSecret, SecretString};

/// Wraps the controller's internal-auth secret for verifying inbound requests.
#[derive(Clone)]
pub struct InternalAuth {
    secret: SecretString,
}

impl InternalAuth {
    pub fn new(secret: SecretString) -> Self {
        Self { secret }
    }

    /// Constant-time check of a provided header value against the secret.
    pub fn verify(&self, provided: &str) -> bool {
        verify(provided, &self.secret)
    }
}

/// Constant-time comparison of `provided` against the `expected` secret.
pub fn verify(provided: &str, expected: &SecretString) -> bool {
    constant_time_eq(provided.as_bytes(), expected.expose_secret().as_bytes())
}

/// Length-checked constant-time byte equality. The length check leaks only the
/// secret's length (not its contents); the byte loop never short-circuits.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_secret_verifies() {
        let auth = InternalAuth::new(SecretString::from("s3cr3t-token".to_string()));
        assert!(auth.verify("s3cr3t-token"));
    }

    #[test]
    fn wrong_secret_and_length_mismatch_fail() {
        let auth = InternalAuth::new(SecretString::from("s3cr3t-token".to_string()));
        assert!(!auth.verify("s3cr3t-tokeX"));
        assert!(!auth.verify("s3cr3t"));
        assert!(!auth.verify(""));
        assert!(!auth.verify("s3cr3t-token-longer"));
    }
}
