//! AES-256-GCM envelope encryption for one complete environment profile.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use zeroize::Zeroize;

use super::record::{ProfileEnvelope, SCHEMA_VERSION};
use crate::error::AppError;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const AAD_DOMAIN: &str = "fkst-hosted/environment-profile";

#[derive(Debug, thiserror::Error)]
pub(super) enum CryptoError {
    #[error("environment profile encryption failed")]
    Encrypt,
    #[error("environment profile integrity verification failed")]
    Integrity,
    #[error("environment profile serialization failed")]
    Serialization,
}

pub(super) struct SealedProfile {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct ProfileCipher(Aes256Gcm);

impl std::fmt::Debug for ProfileCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProfileCipher([REDACTED])")
    }
}

#[derive(Serialize)]
struct AssociatedData<'a> {
    domain: &'static str,
    schema_version: u32,
    github_user_id: i64,
    environment_name: &'a str,
}

fn associated_data(id: i64, name: &str, schema_version: u32) -> Result<Vec<u8>, CryptoError> {
    serde_json::to_vec(&AssociatedData {
        domain: AAD_DOMAIN,
        schema_version,
        github_user_id: id,
        environment_name: name,
    })
    .map_err(|_| CryptoError::Serialization)
}

impl ProfileCipher {
    pub(super) fn from_base64(encoded: &SecretString) -> Result<Self, AppError> {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.expose_secret())
            .map_err(|_| {
                AppError::Config("FKST_ENV_STORE_ENCRYPTION_KEY must be base64-encoded".to_string())
            })?;
        let mut key: [u8; KEY_BYTES] = decoded.try_into().map_err(|_| {
            AppError::Config(
                "FKST_ENV_STORE_ENCRYPTION_KEY must decode to exactly 32 bytes".to_string(),
            )
        })?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| {
            AppError::Config("FKST_ENV_STORE_ENCRYPTION_KEY is invalid".to_string())
        })?;
        key.zeroize();
        Ok(Self(cipher))
    }

    pub(super) fn seal(&self, record: &ProfileEnvelope) -> Result<SealedProfile, CryptoError> {
        let plaintext = serde_json::to_vec(record).map_err(|_| CryptoError::Serialization)?;
        let aad = associated_data(record.github_user_id, &record.name, record.schema_version)?;
        let mut nonce = [0_u8; NONCE_BYTES];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .0
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Encrypt)?;
        Ok(SealedProfile {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    pub(super) fn open(
        &self,
        github_user_id: i64,
        name: &str,
        schema_version: u32,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<ProfileEnvelope, CryptoError> {
        if schema_version != SCHEMA_VERSION || nonce.len() != NONCE_BYTES {
            return Err(CryptoError::Integrity);
        }
        let aad = associated_data(github_user_id, name, schema_version)?;
        let plaintext = self
            .0
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Integrity)?;
        serde_json::from_slice(&plaintext).map_err(|_| CryptoError::Integrity)
    }
}
