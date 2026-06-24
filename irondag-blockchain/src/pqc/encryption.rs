//! Post-Quantum Encryption
//!
//! Provides encrypted P2P communication using session keys derived from Kyber

use crate::pqc::kyber::SessionKey;
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::convert::TryInto;

/// Encrypted message for P2P communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedMessage {
    /// Nonce used for encryption
    pub nonce: Vec<u8>,
    /// Encrypted ciphertext
    pub ciphertext: Vec<u8>,
}

impl EncryptedMessage {
    /// Create a new encrypted message
    pub fn new(nonce: Vec<u8>, ciphertext: Vec<u8>) -> Self {
        Self { nonce, ciphertext }
    }
}

/// Domain separation label for HKDF key derivation (NIST SP 800-227 compliance)
/// Versioned to ensure protocol upgrades produce distinct key material
const HKDF_DOMAIN_LABEL: &[u8] = b"IRONDAG-KYBER-AES256GCM-v1";

/// Derive AES-256-GCM key from ML-KEM shared secret using HKDF-SHA256
///
/// Per NIST SP 800-227 draft guidance, hybrid post-quantum schemes should use
/// a KDF with domain separation even when the shared secret is already uniform.
/// This prevents cross-protocol key reuse and provides defense-in-depth.
fn derive_aes_key(session_key: &SessionKey) -> Result<[u8; 32], String> {
    let hk = Hkdf::<Sha256>::new(None, session_key.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(HKDF_DOMAIN_LABEL, &mut okm)
        .map_err(|_| "HKDF expansion failed".to_string())?;
    Ok(okm)
}

/// Post-Quantum Encryption handler
pub struct PqEncryption;

impl PqEncryption {
    /// Encrypt a message using a session key
    pub fn encrypt(message: &[u8], session_key: &SessionKey) -> Result<EncryptedMessage, String> {
        let derived_key = derive_aes_key(session_key)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|_| "Invalid derived key length".to_string())?;

        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, message)
            .map_err(|e| format!("Encryption failed: {:?}", e))?;

        Ok(EncryptedMessage::new(nonce.to_vec(), ciphertext))
    }

    /// Decrypt a message using a session key
    pub fn decrypt(
        encrypted: &EncryptedMessage,
        session_key: &SessionKey,
    ) -> Result<Vec<u8>, String> {
        let derived_key = derive_aes_key(session_key)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|_| "Invalid derived key length".to_string())?;

        let nonce_bytes: [u8; 12] = encrypted
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| "Invalid nonce length".to_string())?;
        let nonce = Nonce::from(nonce_bytes);
        let plaintext = cipher
            .decrypt(&nonce, encrypted.ciphertext.as_ref())
            .map_err(|e| format!("Decryption failed: {:?}", e))?;

        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let session_key = SessionKey::new([42u8; 32]);
        let message = b"Hello, quantum-resistant world!";

        let encrypted = PqEncryption::encrypt(message, &session_key).unwrap();
        let decrypted = PqEncryption::decrypt(&encrypted, &session_key).unwrap();

        assert_eq!(message, decrypted.as_slice());
    }

    #[test]
    fn test_hkdf_domain_separation() {
        let session_key = SessionKey::new([42u8; 32]);
        let derived = super::derive_aes_key(&session_key).unwrap();

        // Derived key must differ from raw shared secret
        assert_ne!(
            &derived,
            session_key.as_bytes(),
            "HKDF must produce a key different from the raw shared secret"
        );

        // Derivation must be deterministic
        let derived2 = super::derive_aes_key(&session_key).unwrap();
        assert_eq!(derived, derived2, "HKDF derivation must be deterministic");

        // Different inputs must produce different outputs
        let other_key = SessionKey::new([99u8; 32]);
        let other_derived = super::derive_aes_key(&other_key).unwrap();
        assert_ne!(
            derived, other_derived,
            "Different shared secrets must produce different derived keys"
        );
    }
}
