//! Envelope encryption at rest for vault secrets.
//!
//! Scheme (`AES-256-GCM/envelope-v1`): each secret value is sealed under a
//! freshly random 256-bit **DEK** with a freshly random 96-bit nonce; the DEK
//! is then wrapped (encrypted) by a **KEK** held by a [`KeyProvider`]. Only the
//! ciphertext + nonce + wrapped DEK + key id are persisted — never the DEK in
//! the clear and never the plaintext.
//!
//! The `KeyProvider` is the swap seam: `LocalKeyProvider` derives the KEK from
//! an operator-supplied master key, and `KmsKeyProvider` is a documented stub
//! for a future external KMS. Callers depend only on the trait, so the wrapping
//! backend can change without touching the encrypt/decrypt path.
//!
//! Zeroization: the DEK and the plaintext buffer are wiped after use (the DEK
//! via the `Zeroizing`/`SecretBox` wrappers, the plaintext explicitly), so they
//! do not linger in freed memory. No key material or value is ever logged.

use std::fmt;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::Zeroizing;

use super::model::{EncryptedBlob, ENVELOPE_ALG};
use crate::error::AppError;

/// AES-256 key/DEK length in bytes.
const KEY_LEN: usize = 32;
/// AES-GCM nonce length in bytes (96-bit, the GCM standard).
const NONCE_LEN: usize = 12;
/// Associated data binding ciphertext to this envelope scheme, so a blob from a
/// different scheme/version cannot be decrypted under v1 even with the right key.
const ENVELOPE_AAD: &[u8] = ENVELOPE_ALG.as_bytes();

/// Source of the key-encryption key (KEK). Object-safe so it can be stored as
/// `Arc<dyn KeyProvider + Send + Sync>` and swapped (local master key today, an
/// external KMS later) without changing the encrypt/decrypt callers.
///
/// `wrap_dek`/`unwrap_dek` map their own failures to the shared [`AppError`]:
/// an AEAD/KEK mismatch on unwrap is a fail-closed [`AppError::Unprocessable`]
/// (the stored blob cannot be decrypted under the configured key), never a
/// silent success.
pub trait KeyProvider: Send + Sync {
    /// Stable identifier of the active KEK (stamped into each blob for rotation).
    fn key_id(&self) -> &str;
    /// Wrap (encrypt) a plaintext DEK with the KEK.
    fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>, AppError>;
    /// Unwrap (decrypt) a wrapped DEK with the KEK. A KEK mismatch or tamper is
    /// [`AppError::Unprocessable`] (fail closed).
    fn unwrap_dek(&self, wrapped: &[u8]) -> Result<SecretBox<[u8]>, AppError>;
}

/// Local KEK provider: the KEK is the operator-supplied 32-byte master key
/// itself. DEKs are wrapped with AES-256-GCM under that key (a nonce is
/// prepended to each wrapped DEK). Suitable for single-operator/self-hosted
/// deployments; an external KMS is the production hardening path (see
/// [`KmsKeyProvider`]).
pub struct LocalKeyProvider {
    /// The KEK. Held in a `SecretBox` so it redacts in `Debug` and zeroizes on
    /// drop; never logged or exposed.
    kek: SecretBox<[u8]>,
    key_id: String,
}

impl fmt::Debug for LocalKeyProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKeyProvider")
            .field("kek", &"<redacted>")
            .field("key_id", &self.key_id)
            .finish()
    }
}

impl LocalKeyProvider {
    /// Stable key id for the local provider's single KEK.
    pub const KEY_ID: &'static str = "local-v1";

    /// Build from raw 32 master-key bytes. Rejects any other length so a
    /// truncated/oversized key is a loud failure, not a silent weak key.
    pub fn from_key_bytes(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.len() != KEY_LEN {
            return Err(AppError::Config(format!(
                "vault master key must be exactly {KEY_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        Ok(Self {
            kek: SecretBox::new(bytes.to_vec().into_boxed_slice()),
            key_id: Self::KEY_ID.to_string(),
        })
    }

    /// Build from a base64-encoded 32-byte master key (the
    /// `FKST_HOSTED_VAULT_MASTER_KEY` form). Decode + length are validated so a
    /// malformed key fails closed at boot.
    pub fn from_base64(b64: &str) -> Result<Self, AppError> {
        let bytes = Zeroizing::new(BASE64.decode(b64.trim().as_bytes()).map_err(|e| {
            // Never echo the (secret) key material into the error — only the
            // decode-failure category.
            AppError::Config(format!("vault master key is not valid base64: {e}"))
        })?);
        Self::from_key_bytes(&bytes)
    }

    /// The AES-256-GCM cipher keyed by the KEK.
    fn cipher(&self) -> Aes256Gcm {
        let key = Key::<Aes256Gcm>::from_slice(self.kek.expose_secret());
        Aes256Gcm::new(key)
    }
}

impl KeyProvider for LocalKeyProvider {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn wrap_dek(&self, dek: &[u8]) -> Result<Vec<u8>, AppError> {
        let cipher = self.cipher();
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let mut wrapped = cipher.encrypt(nonce, dek).map_err(|_| {
            // An encrypt failure here is an internal/crypto-library fault, not a
            // client error.
            AppError::Internal(anyhow::anyhow!("failed to wrap vault DEK"))
        })?;
        // Prepend the nonce so unwrap can recover it; the wrapped blob is
        // [nonce || ciphertext+tag].
        let mut out = Vec::with_capacity(NONCE_LEN + wrapped.len());
        out.extend_from_slice(&nonce_bytes);
        out.append(&mut wrapped);
        Ok(out)
    }

    fn unwrap_dek(&self, wrapped: &[u8]) -> Result<SecretBox<[u8]>, AppError> {
        if wrapped.len() <= NONCE_LEN {
            return Err(AppError::Unprocessable(
                "wrapped vault DEK is malformed".to_string(),
            ));
        }
        let (nonce_bytes, ciphertext) = wrapped.split_at(NONCE_LEN);
        let cipher = self.cipher();
        let nonce = Nonce::from_slice(nonce_bytes);
        let dek = cipher.decrypt(nonce, ciphertext).map_err(|_| {
            // Fail closed: a wrong KEK or a tampered wrapped DEK cannot be
            // distinguished and must never silently succeed.
            AppError::Unprocessable("vault DEK could not be unwrapped".to_string())
        })?;
        Ok(SecretBox::new(dek.into_boxed_slice()))
    }
}

/// Documented seam for a future external-KMS KEK provider (AWS KMS, GCP KMS,
/// Vault Transit, …). Intentionally NOT implemented in #100: the methods fail
/// closed so a half-wired KMS can never silently store unencrypted secrets, and
/// the type is never constructed at boot. The trait above is the contract a
/// real implementation plugs into.
#[allow(dead_code)]
pub struct KmsKeyProvider {
    key_id: String,
}

impl KeyProvider for KmsKeyProvider {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn wrap_dek(&self, _dek: &[u8]) -> Result<Vec<u8>, AppError> {
        // TODO(#100 follow-up): call the external KMS Encrypt API.
        Err(AppError::Internal(anyhow::anyhow!(
            "KmsKeyProvider is not implemented"
        )))
    }

    fn unwrap_dek(&self, _wrapped: &[u8]) -> Result<SecretBox<[u8]>, AppError> {
        // TODO(#100 follow-up): call the external KMS Decrypt API.
        Err(AppError::Internal(anyhow::anyhow!(
            "KmsKeyProvider is not implemented"
        )))
    }
}

/// Encrypt `plaintext` under a fresh per-secret DEK wrapped by `provider`'s KEK.
///
/// The DEK and the per-encrypt nonce are both cryptographically random, so two
/// encrypts of the same plaintext yield different ciphertext + nonce. The DEK
/// is wiped (`Zeroizing`) once wrapped + used.
pub fn encrypt(provider: &dyn KeyProvider, plaintext: &[u8]) -> Result<EncryptedBlob, AppError> {
    // Fresh 256-bit DEK; wiped on scope exit.
    let mut dek = Zeroizing::new([0u8; KEY_LEN]);
    rand::thread_rng().fill_bytes(dek.as_mut_slice());

    // Fresh 96-bit nonce for the value encryption.
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = Key::<Aes256Gcm>::from_slice(dek.as_slice());
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: ENVELOPE_AAD,
            },
        )
        .map_err(|_| AppError::Internal(anyhow::anyhow!("failed to encrypt vault value")))?;

    let wrapped_dek = provider.wrap_dek(dek.as_slice())?;

    Ok(EncryptedBlob {
        ciphertext,
        nonce: nonce_bytes,
        wrapped_dek,
        key_id: provider.key_id().to_string(),
        alg: ENVELOPE_ALG.to_string(),
    })
}

/// Decrypt an [`EncryptedBlob`] back to a [`SecretString`].
///
/// Fail-closed: a blob with an unexpected `alg`, a wrong KEK, or any AEAD
/// integrity failure yields [`AppError::Unprocessable`] — never a partial or
/// silent result. The recovered DEK is wiped after the value is decrypted.
pub fn decrypt(provider: &dyn KeyProvider, blob: &EncryptedBlob) -> Result<SecretString, AppError> {
    if blob.alg != ENVELOPE_ALG {
        return Err(AppError::Unprocessable(format!(
            "unsupported vault envelope algorithm: {}",
            blob.alg
        )));
    }
    let dek = provider.unwrap_dek(&blob.wrapped_dek)?;
    let key = Key::<Aes256Gcm>::from_slice(dek.expose_secret());
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&blob.nonce);
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &blob.ciphertext,
                    aad: ENVELOPE_AAD,
                },
            )
            .map_err(|_| {
                AppError::Unprocessable("vault value could not be decrypted".to_string())
            })?,
    );
    // The value must be valid UTF-8 (it was a `String` on the way in); a
    // non-UTF-8 blob is corruption and fails closed.
    let value = std::str::from_utf8(&plaintext).map_err(|_| {
        AppError::Unprocessable("decrypted vault value is not valid UTF-8".to_string())
    })?;
    Ok(SecretString::from(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic 32-byte test key (base64). NOT a real secret.
    fn test_key_b64() -> String {
        BASE64.encode([7u8; KEY_LEN])
    }

    fn provider() -> LocalKeyProvider {
        LocalKeyProvider::from_base64(&test_key_b64()).expect("provider")
    }

    #[test]
    fn round_trip_recovers_plaintext() {
        let p = provider();
        let blob = encrypt(&p, b"sk-secret-value").expect("encrypt");
        let recovered = decrypt(&p, &blob).expect("decrypt");
        assert_eq!(recovered.expose_secret(), "sk-secret-value");
        assert_eq!(blob.alg, ENVELOPE_ALG);
        assert_eq!(blob.key_id, LocalKeyProvider::KEY_ID);
    }

    #[test]
    fn nonce_is_unique_across_two_encrypts_of_same_plaintext() {
        let p = provider();
        let a = encrypt(&p, b"same").expect("a");
        let b = encrypt(&p, b"same").expect("b");
        // Both the nonce and the resulting ciphertext must differ — proving the
        // per-encrypt randomness (no nonce reuse, the GCM cardinal sin).
        assert_ne!(a.nonce, b.nonce, "nonce reused across encrypts");
        assert_ne!(a.ciphertext, b.ciphertext, "ciphertext identical");
        // Both still decrypt to the same plaintext.
        assert_eq!(decrypt(&p, &a).unwrap().expose_secret(), "same");
        assert_eq!(decrypt(&p, &b).unwrap().expose_secret(), "same");
    }

    #[test]
    fn wrong_kek_fails_closed_unprocessable() {
        let writer = provider();
        let blob = encrypt(&writer, b"value").expect("encrypt");
        // A different KEK must not decrypt the wrapped DEK.
        let other = LocalKeyProvider::from_base64(&BASE64.encode([9u8; KEY_LEN])).expect("other");
        let err = decrypt(&other, &blob).expect_err("wrong KEK must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[test]
    fn tampered_ciphertext_fails_closed() {
        let p = provider();
        let mut blob = encrypt(&p, b"value").expect("encrypt");
        // Flip a byte of the ciphertext: the GCM tag check must reject it.
        blob.ciphertext[0] ^= 0xff;
        let err = decrypt(&p, &blob).expect_err("tamper must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[test]
    fn unknown_alg_fails_closed() {
        let p = provider();
        let mut blob = encrypt(&p, b"value").expect("encrypt");
        blob.alg = "ROT13/v0".to_string();
        let err = decrypt(&p, &blob).expect_err("unknown alg must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[test]
    fn from_base64_rejects_wrong_length() {
        // 16 bytes (AES-128) is too short for a 256-bit KEK.
        let err = LocalKeyProvider::from_base64(&BASE64.encode([1u8; 16]))
            .expect_err("short key must fail");
        assert!(matches!(err, AppError::Config(_)), "got {err:?}");
    }

    #[test]
    fn from_base64_rejects_non_base64() {
        let err = LocalKeyProvider::from_base64("not base64 @@@").expect_err("must fail");
        assert!(matches!(err, AppError::Config(_)), "got {err:?}");
    }

    #[test]
    fn local_provider_debug_redacts_kek() {
        let rendered = format!("{:?}", provider());
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("local-v1"));
    }

    #[test]
    fn kms_provider_is_an_unimplemented_seam() {
        let kms = KmsKeyProvider {
            key_id: "kms-stub".to_string(),
        };
        assert_eq!(kms.key_id(), "kms-stub");
        assert!(matches!(kms.wrap_dek(b"x"), Err(AppError::Internal(_))));
        assert!(matches!(kms.unwrap_dek(b"x"), Err(AppError::Internal(_))));
    }

    #[test]
    fn malformed_wrapped_dek_fails_closed() {
        let p = provider();
        // Too short to even carry a nonce.
        let err = p.unwrap_dek(&[0u8; 4]).expect_err("must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }
}
