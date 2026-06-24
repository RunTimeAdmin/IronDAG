//! Post-Quantum Account Types
//!
//! Supports Dilithium and SPHINCS+ signature schemes for quantum-resistant transactions

use crate::types::{Address, Hash};
use pqcrypto_mldsa::mldsa65;
use pqcrypto_traits::sign::{PublicKey, SecretKey, SignedMessage};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use tracing::warn;

// SPHINCS+ imports - uses pqcrypto-sphincsplus on all platforms
#[cfg(feature = "sphincsplus")]
use pqcrypto_sphincsplus::sphincssha2128fsimple;

/// PQ account type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PqAccountType {
    /// Dilithium3 (recommended for most use cases)
    Dilithium3,
    /// SPHINCS+-SHA256-128f-simple (smaller signatures, slower)
    SphincsPlus,
    /// Traditional Ed25519 (for backward compatibility)
    Ed25519,
}

impl PqAccountType {
    /// Get signature size in bytes
    pub fn signature_size(&self) -> usize {
        match self {
            PqAccountType::Dilithium3 => 3293, // ML-DSA-65 signature size (same as Dilithium3)
            PqAccountType::SphincsPlus => 7856, // SPHINCS+ signature size
            PqAccountType::Ed25519 => 64,
        }
    }

    /// Get public key size in bytes
    pub fn public_key_size(&self) -> usize {
        match self {
            PqAccountType::Dilithium3 => 1952, // ML-DSA-65 public key size (same as Dilithium3)
            PqAccountType::SphincsPlus => 32,  // SPHINCS+ public key size
            PqAccountType::Ed25519 => 32,
        }
    }
}

/// PQ signature wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PqSignature {
    /// Account type used for this signature
    pub account_type: PqAccountType,
    /// Signature bytes (size depends on account_type)
    pub signature: Vec<u8>,
    /// Public key bytes (size depends on account_type)
    pub public_key: Vec<u8>,
}

impl PqSignature {
    /// Create a new PQ signature
    pub fn new(account_type: PqAccountType, signature: Vec<u8>, public_key: Vec<u8>) -> Self {
        Self {
            account_type,
            signature,
            public_key,
        }
    }

    /// Verify signature size matches account type
    pub fn verify_size(&self) -> bool {
        self.signature.len() == self.account_type.signature_size()
            && self.public_key.len() == self.account_type.public_key_size()
    }
}

/// Post-Quantum Account
///
/// Manages PQ keypairs and signing operations
#[derive(Clone)]
pub struct PqAccount {
    account_type: PqAccountType,
    secret_key: Vec<u8>,
    public_key: Vec<u8>,
    address: Address,
}

impl PqAccount {
    /// Generate a new Dilithium3 account
    pub fn new_dilithium3() -> Self {
        let (pk, sk) = mldsa65::keypair();
        let public_key = pk.as_bytes().to_vec();
        let secret_key = sk.as_bytes().to_vec();
        let address = Self::derive_address(&public_key, PqAccountType::Dilithium3);

        Self {
            account_type: PqAccountType::Dilithium3,
            secret_key,
            public_key,
            address,
        }
    }

    /// Generate a new SPHINCS+ account using pqcrypto-sphincsplus (all platforms)
    #[cfg(feature = "sphincsplus")]
    pub fn new_sphincsplus() -> Result<Self, String> {
        let (pk, sk) = sphincssha2128fsimple::keypair();
        let public_key = pk.as_bytes().to_vec();
        let secret_key = sk.as_bytes().to_vec();
        let address = Self::derive_address(&public_key, PqAccountType::SphincsPlus);

        Ok(Self {
            account_type: PqAccountType::SphincsPlus,
            secret_key,
            public_key,
            address,
        })
    }

    /// Generate a new SPHINCS+ account (stub when feature disabled)
    #[cfg(not(feature = "sphincsplus"))]
    pub fn new_sphincsplus() -> Result<Self, String> {
        Err("SphincsPlus not available — enable 'sphincsplus' feature".to_string())
    }

    /// Create account from existing keypair
    pub fn from_keypair(
        account_type: PqAccountType,
        secret_key: Vec<u8>,
        public_key: Vec<u8>,
    ) -> Result<Self, String> {
        // Simple validation: just check sizes match expected
        let expected_public_size = account_type.public_key_size();

        if public_key.len() != expected_public_size {
            return Err(format!(
                "Invalid public key size for {:?}: expected {}, got {}",
                account_type,
                expected_public_size,
                public_key.len()
            ));
        }

        // For PQ keys, we just store the bytes directly since pqcrypto types
        // don't support round-trip serialization reliably

        let address = Self::derive_address(&public_key, account_type);

        Ok(Self {
            account_type,
            secret_key,
            public_key,
            address,
        })
    }

    /// Sign a message hash
    ///
    /// Returns an error for unimplemented signature schemes (Ed25519 PQ, SphincsPlus).
    /// Use Dilithium3 for post-quantum signatures.
    pub fn sign(&self, message_hash: &Hash) -> Result<PqSignature, String> {
        match self.account_type {
            PqAccountType::Dilithium3 => {
                // Reconstruct secret key from bytes
                let sk = mldsa65::SecretKey::from_bytes(&self.secret_key)
                    .map_err(|_| "Invalid or corrupted Dilithium3 secret key".to_string())?;

                // Sign the message hash
                let signed_msg = mldsa65::sign(message_hash.as_ref(), &sk);
                let signature = signed_msg.as_bytes().to_vec();

                Ok(PqSignature::new(
                    self.account_type,
                    signature,
                    self.public_key.clone(),
                ))
            }
            #[cfg(feature = "sphincsplus")]
            PqAccountType::SphincsPlus => {
                // Reconstruct secret key from bytes
                let sk = sphincssha2128fsimple::SecretKey::from_bytes(&self.secret_key)
                    .map_err(|_| "Invalid SPHINCS+ secret key".to_string())?;

                // Sign the message hash
                let signed_msg = sphincssha2128fsimple::sign(message_hash.as_ref(), &sk);
                let signature = signed_msg.as_bytes().to_vec();

                Ok(PqSignature::new(
                    self.account_type,
                    signature,
                    self.public_key.clone(),
                ))
            }
            #[cfg(not(feature = "sphincsplus"))]
            PqAccountType::SphincsPlus => Err(
                "SphincsPlus not available in this build — enable 'sphincsplus' feature"
                    .to_string(),
            ),
            PqAccountType::Ed25519 => Err(
                "Ed25519 PQ signing is not yet implemented. Use Dilithium3 accounts.".to_string(),
            ),
        }
    }

    /// Verify a signature
    pub fn verify_signature(message_hash: &Hash, signature: &PqSignature) -> bool {
        match signature.account_type {
            PqAccountType::Dilithium3 => {
                // Reconstruct public key from bytes
                let pk = match mldsa65::PublicKey::from_bytes(&signature.public_key) {
                    Ok(pk) => pk,
                    Err(_) => return false,
                };

                // Reconstruct signed message from signature bytes
                let signed_msg = match mldsa65::SignedMessage::from_bytes(&signature.signature) {
                    Ok(sm) => sm,
                    Err(_) => return false,
                };

                // Verify and extract message
                match mldsa65::open(&signed_msg, &pk) {
                    Ok(msg) => msg.as_slice() == message_hash.as_ref(),
                    Err(_) => false,
                }
            }
            #[cfg(feature = "sphincsplus")]
            PqAccountType::SphincsPlus => {
                // Reconstruct public key from bytes
                let pk = match sphincssha2128fsimple::PublicKey::from_bytes(&signature.public_key) {
                    Ok(pk) => pk,
                    Err(_) => return false,
                };

                // Reconstruct signed message from signature bytes
                let signed_msg =
                    match sphincssha2128fsimple::SignedMessage::from_bytes(&signature.signature) {
                        Ok(sm) => sm,
                        Err(_) => return false,
                    };

                // Verify and extract message
                match sphincssha2128fsimple::open(&signed_msg, &pk) {
                    Ok(msg) => msg.as_slice() == message_hash.as_ref(),
                    Err(_) => false,
                }
            }
            #[cfg(not(feature = "sphincsplus"))]
            PqAccountType::SphincsPlus => {
                // SPHINCS+ not available in this build
                warn!("SphincsPlus not available in this build — enable 'sphincsplus' feature");
                false
            }
            PqAccountType::Ed25519 => {
                // Ed25519 PQ not yet implemented - log warning and return false
                warn!("Ed25519 PQ signature verification is not yet implemented");
                false
            }
        }
    }

    /// Derive address from public key
    fn derive_address(public_key: &[u8], account_type: PqAccountType) -> Address {
        let mut hasher = Keccak256::new();
        // Include account type in hash to distinguish PQ accounts
        match account_type {
            PqAccountType::Dilithium3 => hasher.update(b"DILITHIUM3"),
            PqAccountType::SphincsPlus => hasher.update(b"SPHINCS+"),
            PqAccountType::Ed25519 => hasher.update(b"ED25519"),
        };
        hasher.update(public_key);
        let hash = hasher.finalize();

        let mut addr_bytes = [0u8; 20];
        addr_bytes.copy_from_slice(&hash[12..32]);
        Address::new(addr_bytes)
    }

    /// Get account type
    pub fn account_type(&self) -> PqAccountType {
        self.account_type
    }

    /// Get public key
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// Get address
    pub fn address(&self) -> Address {
        self.address
    }

    /// Get secret key (use with caution!)
    pub fn secret_key(&self) -> &[u8] {
        &self.secret_key
    }
}

impl PqAccountType {
    /// Get secret key size in bytes
    pub fn secret_key_size(&self) -> usize {
        match self {
            PqAccountType::Dilithium3 => 4032, // ML-DSA-65 secret key size (same as Dilithium3)
            PqAccountType::SphincsPlus => 64,  // SPHINCS+ secret key size
            PqAccountType::Ed25519 => 32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dilithium3_account() {
        let account = PqAccount::new_dilithium3();
        assert_eq!(account.account_type(), PqAccountType::Dilithium3);
        assert_eq!(
            account.public_key().len(),
            PqAccountType::Dilithium3.public_key_size()
        );

        let message_hash = Hash([1u8; 32]);
        let signature = account
            .sign(&message_hash)
            .expect("Dilithium3 signing should succeed");
        assert!(PqAccount::verify_signature(&message_hash, &signature));

        // Verify that a different message fails verification
        let wrong_message = Hash([2u8; 32]);
        assert!(!PqAccount::verify_signature(&wrong_message, &signature));
    }

    #[test]
    #[cfg(feature = "sphincsplus")]
    fn test_sphincsplus_account() {
        let account =
            PqAccount::new_sphincsplus().expect("SPHINCS+ account creation should succeed");
        assert_eq!(account.account_type(), PqAccountType::SphincsPlus);
        assert_eq!(
            account.public_key().len(),
            PqAccountType::SphincsPlus.public_key_size()
        );

        let message_hash = [1u8; 32];
        let signature = account
            .sign(&message_hash)
            .expect("SPHINCS+ signing should succeed");
        assert!(PqAccount::verify_signature(&message_hash, &signature));

        // Verify that a different message fails verification
        let wrong_message = [2u8; 32];
        assert!(!PqAccount::verify_signature(&wrong_message, &signature));
    }
}
