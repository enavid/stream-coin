use aes_gcm::aead::{Aead, KeyInit, OsRng as AeadOsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// An AES-256-GCM ciphertext + nonce, both base64-encoded — the shape stored in
/// `user_exchange_credentials.credentials_enc` (JSONB).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptedEnvelope {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("invalid envelope encoding: {0}")]
    InvalidEncoding(String),
}

/// Decodes a base64 string into exactly 32 key bytes, or `None` if malformed/wrong length.
fn parse_key_b64(raw: &str) -> Option<[u8; 32]> {
    let bytes = BASE64.decode(raw.trim()).ok()?;
    bytes.try_into().ok()
}

pub struct CredentialCipher {
    key: [u8; 32],
}

impl CredentialCipher {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Builds the cipher from `CREDENTIALS_ENCRYPTION_KEY` (base64-encoded 32 bytes).
    /// Returns `None` if the env var is unset or does not decode to exactly 32 bytes.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("CREDENTIALS_ENCRYPTION_KEY").ok()?;
        parse_key_b64(&raw).map(Self::new)
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> EncryptedEnvelope {
        let cipher = Aes256Gcm::new_from_slice(&self.key).expect("key must be 32 bytes");
        let mut nonce_bytes = [0u8; 12];
        AeadOsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .expect("encryption must not fail");
        EncryptedEnvelope {
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(ciphertext),
        }
    }

    pub fn decrypt(&self, envelope: &EncryptedEnvelope) -> Result<Vec<u8>, CryptoError> {
        let nonce_bytes = BASE64
            .decode(&envelope.nonce)
            .map_err(|e| CryptoError::InvalidEncoding(e.to_string()))?;
        let ciphertext = BASE64
            .decode(&envelope.ciphertext)
            .map_err(|e| CryptoError::InvalidEncoding(e.to_string()))?;
        let cipher = Aes256Gcm::new_from_slice(&self.key).expect("key must be 32 bytes");
        let nonce = Nonce::from_slice(&nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext.as_slice())
            .map_err(|_| CryptoError::DecryptionFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> CredentialCipher {
        CredentialCipher::new([7u8; 32])
    }

    #[test]
    fn decrypt_recovers_original_plaintext() {
        let c = cipher();
        let envelope = c.encrypt(b"super-secret-api-key");
        let recovered = c.decrypt(&envelope).unwrap();
        assert_eq!(recovered, b"super-secret-api-key");
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_call() {
        let c = cipher();
        let e1 = c.encrypt(b"same-plaintext");
        let e2 = c.encrypt(b"same-plaintext");
        assert_ne!(
            e1.ciphertext, e2.ciphertext,
            "nonce must randomize ciphertext"
        );
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let envelope = CredentialCipher::new([1u8; 32]).encrypt(b"secret");
        let result = CredentialCipher::new([2u8; 32]).decrypt(&envelope);
        assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
    }

    // `from_env` itself is a thin env-var wrapper around `parse_key_b64`; tested here via
    // the pure function directly to avoid races from mutating process-global env vars
    // across parallel test threads.

    #[test]
    fn parse_key_b64_returns_none_for_wrong_length_key() {
        assert!(parse_key_b64(&BASE64.encode(b"too-short")).is_none());
    }

    #[test]
    fn parse_key_b64_returns_none_for_invalid_base64() {
        assert!(parse_key_b64("not valid base64!!!").is_none());
    }

    #[test]
    fn parse_key_b64_decodes_valid_32_byte_key() {
        let key_b64 = BASE64.encode([9u8; 32]);
        let key = parse_key_b64(&key_b64).expect("valid key must decode");
        let c = CredentialCipher::new(key);
        let envelope = c.encrypt(b"hello");
        assert_eq!(c.decrypt(&envelope).unwrap(), b"hello");
    }
}
