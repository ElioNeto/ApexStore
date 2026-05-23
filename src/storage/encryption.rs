//! Transparent encryption at rest for SSTable blocks and WAL frames.
//!
//! Uses **AES-256-GCM** via the `aes-gcm` crate.  Each encrypted block
//! gets a fresh random 12-byte IV (nonce) prepended to the ciphertext.
//!
//! # Key management
//!
//! The key is a 32-byte secret (`[u8; 32]`) and is provided through an
//! [`EncryptionConfig`].  The [`Encryptor`] struct wraps the cipher and
//! exposes `encrypt_block` / `decrypt_block`.
//!
//! Encryption is **optional** and **disabled by default**.

use crate::infra::error::{LsmError, Result};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Configuration for encryption at rest.
///
/// When `enabled` is `false` (the default), all operations are
/// pass-through with zero overhead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncryptionConfig {
    /// AES-256 key (exactly 32 bytes).
    pub key: [u8; 32],
    /// Whether encryption is enabled.
    pub enabled: bool,
}

impl EncryptionConfig {
    /// Create an [`EncryptionConfig`] from an optional hex-encoded key file path.
    ///
    /// * `Some(path)` — reads the file, trims whitespace, hex-decodes the
    ///   contents to obtain the 32-byte AES-256 key, and enables encryption.
    /// * `None` — returns a default (disabled) config.
    pub fn from_key_path(path: Option<&str>) -> Result<Self> {
        match path {
            Some(p) => {
                let contents = std::fs::read_to_string(p).map_err(|e| {
                    LsmError::InvalidArgument(format!("Failed to read key file '{}': {}", p, e))
                })?;
                let key_hex = contents.trim();
                let key_bytes = hex::decode(key_hex).map_err(|e| {
                    LsmError::InvalidArgument(format!(
                        "Invalid hex key in '{}': {} (expected 64 hex chars)",
                        p, e
                    ))
                })?;
                if key_bytes.len() != 32 {
                    return Err(LsmError::InvalidArgument(format!(
                        "Key file '{}' must contain exactly 32 bytes (64 hex chars), got {} bytes",
                        p,
                        key_bytes.len()
                    )));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&key_bytes);
                Ok(Self { key, enabled: true })
            }
            None => Ok(Self::default()),
        }
    }
}

/// Wraps an AES-256-GCM cipher for transparent encryption / decryption.
///
/// When `enabled` is `false`, all methods are pass-through (zero-copy
/// semantics are approximated by returning `Vec<u8>` with the same data).
pub struct Encryptor {
    cipher: Option<Aes256Gcm>,
    enabled: bool,
}

impl Encryptor {
    /// Create a new `Encryptor` from an [`EncryptionConfig`].
    pub fn new(config: &EncryptionConfig) -> Self {
        let cipher = if config.enabled {
            let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&config.key);
            Some(Aes256Gcm::new(key))
        } else {
            None
        };
        Self {
            cipher,
            enabled: config.enabled,
        }
    }

    /// Create a disabled (pass-through) encryptor.
    pub fn disabled() -> Self {
        Self {
            cipher: None,
            enabled: false,
        }
    }

    /// Returns `true` when encryption is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Encrypt a plaintext block.
    ///
    /// When encryption is disabled, returns `plaintext` unchanged.
    ///
    /// # Format
    ///
    /// The returned vector contains:
    /// ```text
    /// [12-byte random IV (nonce)][AES-256-GCM ciphertext + tag (16 bytes)]
    /// ```
    pub fn encrypt_block(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if !self.enabled {
            return Ok(plaintext.to_vec());
        }
        let cipher = self.cipher.as_ref().ok_or_else(|| {
            LsmError::CompactionFailed("Encryptor not initialized for encryption".to_string())
        })?;

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|e| {
            LsmError::CompactionFailed(format!("AES-256-GCM encryption failed: {}", e))
        })?;

        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypt a ciphertext block previously produced by [`encrypt_block`].
    ///
    /// When encryption is disabled, returns `data` unchanged.
    ///
    /// Expects the data to be in the format produced by [`encrypt_block`]:
    /// `[12-byte IV][ciphertext + tag]`.
    pub fn decrypt_block(&self, data: &[u8]) -> Result<Vec<u8>> {
        if !self.enabled {
            return Ok(data.to_vec());
        }
        let cipher = self.cipher.as_ref().ok_or_else(|| {
            LsmError::CompactionFailed("Encryptor not initialized for decryption".to_string())
        })?;

        if data.len() < 12 {
            return Err(LsmError::CorruptedData(format!(
                "Ciphertext too short ({} bytes); need at least 12 for IV",
                data.len()
            )));
        }

        let (nonce_bytes, encrypted) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher.decrypt(nonce, encrypted).map_err(|e| {
            LsmError::CorruptedData(format!(
                "AES-256-GCM decryption failed (wrong key or corrupted data): {}",
                e
            ))
        })?;

        Ok(plaintext)
    }
}

impl std::fmt::Debug for Encryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encryptor")
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EncryptionConfig {
        EncryptionConfig {
            key: [0xABu8; 32],
            enabled: true,
        }
    }

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let encryptor = Encryptor::new(&test_config());
        let plaintext = b"Hello, ApexStore encryption!";
        let ciphertext = encryptor.encrypt_block(plaintext).unwrap();
        assert_ne!(
            ciphertext, plaintext,
            "ciphertext should differ from plaintext"
        );
        assert!(ciphertext.len() > 12, "ciphertext should contain IV");

        let decrypted = encryptor.decrypt_block(&ciphertext).unwrap();
        assert_eq!(
            decrypted, plaintext,
            "round-trip should produce original plaintext"
        );
    }

    #[test]
    fn test_encrypt_produces_different_iv_each_time() {
        let encryptor = Encryptor::new(&test_config());
        let plaintext = b"same data";
        let c1 = encryptor.encrypt_block(plaintext).unwrap();
        let c2 = encryptor.encrypt_block(plaintext).unwrap();
        // With random IVs, the two ciphertexts should differ
        assert_ne!(c1, c2, "different IVs should produce different ciphertexts");
    }

    #[test]
    fn test_decrypt_wrong_key_fails() {
        let cfg_ok = test_config();
        let mut cfg_bad = cfg_ok.clone();
        cfg_bad.key[0] ^= 0xFF; // flip a bit
        let encryptor = Encryptor::new(&cfg_ok);
        let bad_encryptor = Encryptor::new(&cfg_bad);

        let plaintext = b"secret data";
        let ciphertext = encryptor.encrypt_block(plaintext).unwrap();

        let result = bad_encryptor.decrypt_block(&ciphertext);
        assert!(result.is_err(), "decryption with wrong key should fail");
    }

    #[test]
    fn test_disabled_encryptor_passthrough() {
        let encryptor = Encryptor::disabled();
        assert!(!encryptor.is_enabled());

        let data = b"plaintext data";
        let result = encryptor.encrypt_block(data).unwrap();
        assert_eq!(result, data, "disabled encryptor should pass through");

        let decrypted = encryptor.decrypt_block(data).unwrap();
        assert_eq!(decrypted, data, "disabled decryptor should pass through");
    }

    #[test]
    fn test_decrypt_truncated_data_fails() {
        let encryptor = Encryptor::new(&test_config());
        let result = encryptor.decrypt_block(b"too_short");
        assert!(result.is_err(), "truncated ciphertext should fail");
    }

    #[test]
    fn test_encryption_config_from_key_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let key_path = dir.path().join("aes.key");
        // Write 64 hex chars representing 32 bytes
        let key_hex = "ab".repeat(32); // 64 chars
        std::fs::write(&key_path, &key_hex).unwrap();

        let config = EncryptionConfig::from_key_path(Some(key_path.to_str().unwrap())).unwrap();
        assert!(config.enabled);
        assert_eq!(config.key[0], 0xAB);
        assert_eq!(config.key[31], 0xAB);
    }

    #[test]
    fn test_encryption_config_from_none() {
        let config = EncryptionConfig::from_key_path(None).unwrap();
        assert!(!config.enabled);
    }

    #[test]
    fn test_encryption_config_invalid_hex() {
        let dir = tempfile::TempDir::new().unwrap();
        let key_path = dir.path().join("bad.key");
        std::fs::write(&key_path, "not-hex!!!").unwrap();

        let result = EncryptionConfig::from_key_path(Some(key_path.to_str().unwrap()));
        assert!(result.is_err());
    }
}
