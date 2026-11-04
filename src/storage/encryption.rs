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
//! Encryption is disabled by default and requires an explicit nonzero key.

use crate::infra::error::{LsmError, Result};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::{rngs::SysRng, TryCryptoRng};
use zeroize::{Zeroize, Zeroizing};

/// Runtime encryption configuration. Keys are not serializable and Debug is redacted.
#[derive(Clone, Default)]
pub struct EncryptionConfig {
    /// AES-256 key (exactly 32 bytes).
    pub key: [u8; 32],
    /// Whether encryption is enabled. Disabled by default.
    pub enabled: bool,
}

impl std::fmt::Debug for EncryptionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionConfig")
            .field("enabled", &self.enabled)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for EncryptionConfig {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}
impl zeroize::ZeroizeOnDrop for EncryptionConfig {}

impl EncryptionConfig {
    /// Validate application storage settings before touching an SSTable or database.
    pub fn from_storage_config(config: &crate::infra::config::StorageConfig) -> Result<Self> {
        if !config.encryption_enabled {
            return Ok(Self::default());
        }
        let path = config
            .encryption_key_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .ok_or(LsmError::EncryptionKeyRequired)?;
        Self::from_key_path(Some(path))
    }

    /// Create an [`EncryptionConfig`] from an optional hex-encoded key file path.
    ///
    /// * `Some(path)` — reads the file, trims whitespace, hex-decodes the
    ///   contents to obtain the 32-byte AES-256 key, and enables encryption.
    /// * `None` — returns the disabled default configuration.
    pub fn from_key_path(path: Option<&str>) -> Result<Self> {
        match path {
            Some(p) => {
                let contents = Zeroizing::new(std::fs::read_to_string(p).map_err(|e| {
                    LsmError::InvalidArgument(format!("Failed to read key file '{}': {}", p, e))
                })?);
                let key_hex = contents.trim();
                let key_bytes = Zeroizing::new(hex::decode(key_hex).map_err(|e| {
                    LsmError::InvalidArgument(format!(
                        "Invalid hex key in '{}': {} (expected 64 hex chars)",
                        p, e
                    ))
                })?);
                if key_bytes.len() != 32 {
                    return Err(LsmError::InvalidArgument(format!(
                        "Key file '{}' must contain exactly 32 bytes (64 hex chars), got {} bytes",
                        p,
                        key_bytes.len()
                    )));
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&key_bytes);
                if key == [0; 32] {
                    return Err(LsmError::LegacyEncryptionMigrationRequired);
                }
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
    /// Decode an encrypted persisted payload without silently accepting a
    /// legacy zero key. Probing that key only diagnoses migration requirements.
    pub(crate) fn decrypt_for_recovery(&self, data: &[u8]) -> Result<Vec<u8>> {
        let result = if self.is_enabled() {
            self.decrypt_block(data)
        } else {
            Err(LsmError::EncryptionKeyRequired)
        };
        match result {
            Ok(plaintext) => Ok(plaintext),
            Err(error) => {
                let legacy = Self::new(&EncryptionConfig {
                    enabled: true,
                    key: [0; 32],
                });
                if let Ok(mut plaintext) = legacy.decrypt_block(data) {
                    plaintext.zeroize();
                    return Err(LsmError::LegacyEncryptionMigrationRequired);
                }
                Err(error)
            }
        }
    }

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
        self.encrypt_block_with_rng(plaintext, &mut SysRng)
    }

    fn encrypt_block_with_rng(
        &self,
        plaintext: &[u8],
        rng: &mut impl TryCryptoRng,
    ) -> Result<Vec<u8>> {
        if !self.enabled {
            return Ok(plaintext.to_vec());
        }
        let cipher = self.cipher.as_ref().ok_or_else(|| {
            LsmError::CompactionFailed("Encryptor not initialized for encryption".to_string())
        })?;

        let mut nonce_bytes = [0u8; 12];
        rng.try_fill_bytes(&mut nonce_bytes).map_err(|e| {
            LsmError::CompactionFailed(format!("Secure nonce randomness unavailable: {e}"))
        })?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher.encrypt(nonce, plaintext).map_err(|e| {
            LsmError::CompactionFailed(format!("AES-256-GCM encryption failed: {}", e))
        })?;

        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypt a ciphertext block previously produced by `encrypt_block`.
    ///
    /// When encryption is disabled, returns `data` unchanged.
    ///
    /// Expects the data to be in the format produced by `encrypt_block`:
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

        let plaintext = cipher
            .decrypt(nonce, encrypted)
            .map_err(|_| LsmError::EncryptionAuthenticationFailed)?;

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

    struct FailingRng;

    impl rand::TryRng for FailingRng {
        type Error = std::io::Error;

        fn try_next_u32(&mut self) -> std::io::Result<u32> {
            Err(std::io::Error::other("entropy unavailable"))
        }

        fn try_next_u64(&mut self) -> std::io::Result<u64> {
            Err(std::io::Error::other("entropy unavailable"))
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> std::io::Result<()> {
            // A failed source may have partially written the destination.
            dest[0] = 0xAB;
            Err(std::io::Error::other("entropy unavailable"))
        }
    }

    impl rand::TryCryptoRng for FailingRng {}

    #[test]
    fn test_encryption_rejects_entropy_failure() {
        let encryptor = Encryptor::new(&test_config());
        let error = encryptor
            .encrypt_block_with_rng(b"secret", &mut FailingRng)
            .unwrap_err();
        assert!(
            matches!(error, LsmError::CompactionFailed(message) if message.contains("entropy unavailable"))
        );
    }

    #[test]
    fn test_disabled_encryption_does_not_require_entropy() {
        let result = Encryptor::disabled()
            .encrypt_block_with_rng(b"plaintext", &mut FailingRng)
            .unwrap();
        assert_eq!(result, b"plaintext");
    }

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
        assert_eq!(ciphertext.len(), 12 + plaintext.len() + 16);

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
        assert_ne!(&c1[..12], &c2[..12], "each encryption needs a fresh nonce");
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
        assert!(!config.enabled, "no key must leave encryption disabled");
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
