//! App JWT minting (RS256, short-lived).
//!
//! GitHub requires the app JWT to carry `iss = app_id`, be RS256-signed,
//! and have a lifetime <= 10 minutes. We use 9 minutes with a 60-second
//! `iat` backdate for clock-skew absorption.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// JWT lifetime: 9 minutes (540 seconds), safely under GitHub's 10-minute cap.
pub const APP_JWT_LIFETIME_SECS: u64 = 540;

/// `iat` backdate: 60 seconds for clock-skew absorption.
pub const IAT_BACKDATE_SECS: u64 = 60;

/// JWT claims for the GitHub App token.
#[derive(Debug, Serialize, Deserialize)]
struct AppClaims {
    /// Application ID (the `iss` field).
    iss: String,
    /// Issued-at (backdated by [`IAT_BACKDATE_SECS`]).
    iat: u64,
    /// Expiration ([`APP_JWT_LIFETIME_SECS`] after the backdated `iat`).
    exp: u64,
}

/// Build an [`EncodingKey`] from a PEM-encoded RSA private key.
pub fn build_encoding_key(pem: &SecretString) -> Result<EncodingKey, jsonwebtoken::errors::Error> {
    EncodingKey::from_rsa_pem(pem.expose_secret().as_bytes())
}

/// Mint a short-lived RS256 app JWT carrying the given `app_id`.
///
/// Claims shape: `{ iss: app_id, iat: now - 60, exp: now + 540 }`.
/// The returned token is wrapped in a [`SecretString`] to prevent accidental
/// logging or debugging exposure.
pub fn mint_app_jwt(
    app_id: u64,
    encoding_key: &EncodingKey,
) -> Result<SecretString, jsonwebtoken::errors::Error> {
    let now = epoch_secs();
    let claims = AppClaims {
        iss: app_id.to_string(),
        iat: now.saturating_sub(IAT_BACKDATE_SECS),
        exp: now + APP_JWT_LIFETIME_SECS,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)?;
    Ok(SecretString::from(token))
}

/// Current time as whole seconds since epoch (UTC).
fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::RsaPrivateKey;

    fn generate_keypair() -> (RsaPrivateKey, rsa::RsaPublicKey) {
        use rand::rngs::OsRng;
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
        let public = rsa::RsaPublicKey::from(&private);
        (private, public)
    }

    fn encoding_key_from_private(key: &RsaPrivateKey) -> EncodingKey {
        let pem = key.to_pkcs8_pem(LineEnding::LF).expect("pkcs8 pem");
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key")
    }

    fn public_key_pem(key: &rsa::RsaPublicKey) -> String {
        key.to_public_key_pem(LineEnding::LF)
            .expect("public key pem")
    }

    #[test]
    fn claims_shape_matches_github_requirements() {
        let (private, public) = generate_keypair();
        let enc = encoding_key_from_private(&private);
        let token = mint_app_jwt(42, &enc).expect("mint");
        let token_str = token.expose_secret();

        let pub_pem = public_key_pem(&public);
        let dec_key = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec key");
        let mut validation = Validation::new(Algorithm::RS256);
        // Do not validate audience/expired for this structural test.
        validation.validate_exp = false;
        validation.insecure_disable_signature_validation();

        let data = decode::<AppClaims>(token_str, &dec_key, &validation).expect("decode");
        let claims = data.claims;
        assert_eq!(claims.iss, "42");
        assert!(claims.exp > claims.iat);
        assert!(
            claims.exp - claims.iat <= APP_JWT_LIFETIME_SECS + IAT_BACKDATE_SECS,
            "lifetime must be <= {APP_JWT_LIFETIME_SECS} + {IAT_BACKDATE_SECS}"
        );
        assert!(
            claims.exp - claims.iat >= APP_JWT_LIFETIME_SECS - IAT_BACKDATE_SECS,
            "lifetime should be at least {APP_JWT_LIFETIME_SECS} - {IAT_BACKDATE_SECS}"
        );
    }

    #[test]
    fn rs256_signature_verifies_against_test_public_key() {
        let (private, public) = generate_keypair();
        let enc = encoding_key_from_private(&private);
        let token = mint_app_jwt(99999, &enc).expect("mint");
        let token_str = token.expose_secret();

        let pub_pem = public_key_pem(&public);
        let dec_key = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).expect("dec key");
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = false;

        let data = decode::<AppClaims>(token_str, &dec_key, &validation).expect("verify");
        assert_eq!(data.claims.iss, "99999");
        assert_eq!(data.header.alg, Algorithm::RS256);
    }

    #[test]
    fn minted_token_is_brief_and_within_10_minute_cap() {
        let (private, _) = generate_keypair();
        let enc = encoding_key_from_private(&private);
        let now = epoch_secs();
        let token = mint_app_jwt(1, &enc).expect("mint");
        let token_str = token.expose_secret();

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = false;
        validation.insecure_disable_signature_validation();
        let data = decode::<AppClaims>(token_str, &DecodingKey::from_secret(b"-"), &validation)
            .expect("decode");

        // exp must be < now + 600 (10 min GitHub cap)
        assert!(
            data.claims.exp < now + 600,
            "exp ({}) must be < now+600 ({})",
            data.claims.exp,
            now + 600
        );
        // iat must be backdated
        assert!(
            data.claims.iat <= now,
            "iat ({}) must be <= now ({})",
            data.claims.iat,
            now
        );
    }
}
