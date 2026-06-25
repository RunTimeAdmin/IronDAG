//! ML-KEM Key Exchange (FIPS 203)
//!
//! Implements ML-KEM-768 for P2P handshake and session key derivation.
//! Pure Rust via the `ml-kem` crate — works on all platforms (Windows, Linux, macOS).

use serde::{Deserialize, Serialize};

// ML-KEM imports (FIPS 203, pure Rust, all platforms)
#[cfg(feature = "kyber")]
use ml_kem::kem::{Ciphertext, Decapsulate, DecapsulationKey, Encapsulate, EncapsulationKey};
#[cfg(feature = "kyber")]
use ml_kem::{kem::Kem, KeyExport, MlKem768, TryKeyInit};

/// Bridge rand 0.8 OsRng to rand_core 0.10 CryptoRng for ml-kem compatibility
#[cfg(feature = "kyber")]
struct OsRngCompat;

#[cfg(feature = "kyber")]
impl rand_core_09::TryRng for OsRngCompat {
    type Error = rand_core_09::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        use rand::RngCore;
        Ok(rand::rngs::OsRng.next_u32())
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        use rand::RngCore;
        Ok(rand::rngs::OsRng.next_u64())
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(dst);
        Ok(())
    }
}

#[cfg(feature = "kyber")]
impl rand_core_09::TryCryptoRng for OsRngCompat {}

/// Session key derived from Kyber key exchange (32 bytes)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionKey(pub [u8; 32]);

impl SessionKey {
    /// Create a new session key from bytes
    pub fn new(key: [u8; 32]) -> Self {
        Self(key)
    }

    /// Get key bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Kyber key exchange for P2P handshake
#[derive(Clone)]
pub struct KyberKeyExchange {
    /// Kyber public key (encapsulation key)
    public_key: Vec<u8>,
    /// Kyber secret key (decapsulation key)
    #[allow(dead_code)]
    secret_key: Vec<u8>,
}

#[cfg(feature = "kyber")]
impl KyberKeyExchange {
    /// Generate a new Kyber keypair using ML-KEM-768 (FIPS 203)
    ///
    /// Note: This is a blocking operation that should be called via spawn_blocking in async contexts
    pub fn generate() -> Self {
        // Use rand_core 0.9 OsRng via unwrap_err() to satisfy kem 0.3's CryptoRng bound
        let mut rng = rand_core_09::UnwrapErr(OsRngCompat);
        let (dk, ek) = MlKem768::generate_keypair_from_rng(&mut rng);
        let public_key = ek.to_bytes().to_vec();
        let secret_key = dk.to_bytes().to_vec();
        Self {
            public_key,
            secret_key,
        }
    }

    /// Generate a new Kyber keypair using ML-KEM-768 (async version)
    ///
    /// This wraps the blocking key generation in spawn_blocking for safe use in async contexts
    pub async fn generate_async() -> Self {
        tokio::task::spawn_blocking(|| Self::generate())
            .await
            .expect("Kyber key generation panicked")
    }

    /// Create from existing keypair
    pub fn from_keypair(public_key: Vec<u8>, secret_key: Vec<u8>) -> Result<Self, String> {
        // ML-KEM-768 key sizes
        const EK_SIZE: usize = 1184; // Encapsulation key size
        const SEED_SIZE: usize = 64; // Decapsulation key seed size (ml-kem 0.3.0 uses seed-based keys)

        // Verify key sizes
        if public_key.len() != EK_SIZE {
            return Err(format!(
                "Invalid public key size: expected {}, got {}",
                EK_SIZE,
                public_key.len()
            ));
        }
        if secret_key.len() != SEED_SIZE {
            return Err(format!(
                "Invalid secret key size: expected {}, got {}",
                SEED_SIZE,
                secret_key.len()
            ));
        }

        // Verify encapsulation key can be deserialized via TryKeyInit
        let _ek: EncapsulationKey<MlKem768> = TryKeyInit::new_from_slice(&public_key)
            .map_err(|_| "Invalid public key".to_string())?;

        // Verify decapsulation key bytes are valid (seed-based reconstruction)
        let mut seed_arr = [0u8; 64];
        seed_arr.copy_from_slice(&secret_key);
        let _dk = DecapsulationKey::<MlKem768>::from(ml_kem::Seed::from(seed_arr));

        Ok(Self {
            public_key,
            secret_key,
        })
    }

    /// Encapsulate a shared secret (client side)
    /// Returns (ciphertext, shared_secret)
    ///
    /// Note: This is a blocking operation that should be called via spawn_blocking in async contexts
    pub fn encapsulate(&self, peer_public_key: &[u8]) -> Result<(Vec<u8>, SessionKey), String> {
        const EK_SIZE: usize = 1184;

        // Verify size
        if peer_public_key.len() != EK_SIZE {
            return Err(format!(
                "Invalid public key size: expected {}, got {}",
                EK_SIZE,
                peer_public_key.len()
            ));
        }

        // Deserialize the peer's encapsulation key via TryKeyInit
        let ek: EncapsulationKey<MlKem768> = TryKeyInit::new_from_slice(peer_public_key)
            .map_err(|_| "Invalid public key".to_string())?;

        // Encapsulate using rand_core 0.9 OsRng (unwraperr satisfies CryptoRng)
        let mut rng = rand_core_09::UnwrapErr(OsRngCompat);
        let (ciphertext, shared_key) = ek.encapsulate_with_rng(&mut rng);

        // Convert shared key to SessionKey (32 bytes)
        let sk_bytes: &[u8] = &*shared_key;
        let shared_bytes: [u8; 32] = sk_bytes
            .try_into()
            .map_err(|_| "Shared secret conversion failed".to_string())?;

        // Convert ciphertext to Vec<u8>
        let ct_bytes: Vec<u8> = (*ciphertext).to_vec();

        Ok((ct_bytes, SessionKey(shared_bytes)))
    }

    /// Encapsulate a shared secret (client side) - async version
    ///
    /// This wraps the blocking encapsulation in spawn_blocking for safe use in async contexts
    pub async fn encapsulate_async(
        &self,
        peer_public_key: Vec<u8>,
    ) -> Result<(Vec<u8>, SessionKey), String> {
        let public_key_clone = self.public_key.clone();
        let secret_key_clone = self.secret_key.clone();

        tokio::task::spawn_blocking(move || {
            let kyber = Self {
                public_key: public_key_clone,
                secret_key: secret_key_clone,
            };
            kyber.encapsulate(&peer_public_key)
        })
        .await
        .map_err(|e| format!("Encapsulation task panicked: {}", e))?
    }

    /// Decapsulate a shared secret (server side)
    /// Returns the shared secret
    ///
    /// Note: This is a blocking operation that should be called via spawn_blocking in async contexts
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<SessionKey, String> {
        const CT_SIZE: usize = 1088; // Ciphertext size for ML-KEM-768
        const SEED_SIZE: usize = 64; // Decapsulation key seed size

        // Reconstruct decapsulation key from 64-byte seed
        if self.secret_key.len() != SEED_SIZE {
            return Err(format!(
                "Invalid secret key size: expected {}, got {}",
                SEED_SIZE,
                self.secret_key.len()
            ));
        }
        let mut seed_arr = [0u8; 64];
        seed_arr.copy_from_slice(&self.secret_key);
        let dk = DecapsulationKey::<MlKem768>::from(ml_kem::Seed::from(seed_arr));

        // Verify and deserialize ciphertext
        if ciphertext.len() != CT_SIZE {
            return Err(format!(
                "Invalid ciphertext size: expected {}, got {}",
                CT_SIZE,
                ciphertext.len()
            ));
        }

        // Convert ciphertext bytes to Ciphertext type
        let mut ct: Ciphertext<MlKem768> = Default::default();
        ct.copy_from_slice(ciphertext);

        // Decapsulate to get shared key
        let shared_key = dk.decapsulate(&ct);

        // Convert shared key to SessionKey (32 bytes)
        let sk_bytes: &[u8] = &*shared_key;
        let shared_bytes: [u8; 32] = sk_bytes
            .try_into()
            .map_err(|_| "Shared secret conversion failed".to_string())?;

        Ok(SessionKey(shared_bytes))
    }

    /// Decapsulate a shared secret (server side) - async version
    ///
    /// This wraps the blocking decapsulation in spawn_blocking for safe use in async contexts
    pub async fn decapsulate_async(&self, ciphertext: Vec<u8>) -> Result<SessionKey, String> {
        let public_key_clone = self.public_key.clone();
        let secret_key_clone = self.secret_key.clone();

        tokio::task::spawn_blocking(move || {
            let kyber = Self {
                public_key: public_key_clone,
                secret_key: secret_key_clone,
            };
            kyber.decapsulate(&ciphertext)
        })
        .await
        .map_err(|e| format!("Decapsulation task panicked: {}", e))?
    }

    /// Get public key
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// Get public key as bytes (for serialization)
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.public_key.clone()
    }
}

#[cfg(not(feature = "kyber"))]
impl KyberKeyExchange {
    /// Generate a new Kyber keypair (stub when feature disabled)
    pub fn generate() -> Self {
        // Stub: returns zero keys. Actual crypto operations (encapsulate/decapsulate)
        // will return Err when kyber feature is disabled.
        Self {
            public_key: vec![0u8; 1184], // ML-KEM-768 encapsulation key size
            secret_key: vec![0u8; 2400], // ML-KEM-768 decapsulation key size
        }
    }

    /// Create from existing keypair
    pub fn from_keypair(public_key: Vec<u8>, secret_key: Vec<u8>) -> Result<Self, String> {
        // Verify key sizes (ML-KEM-768)
        if public_key.len() != 1184 {
            return Err(format!(
                "Invalid public key size: expected 1184, got {}",
                public_key.len()
            ));
        }
        if secret_key.len() != 2400 {
            return Err(format!(
                "Invalid secret key size: expected 2400, got {}",
                secret_key.len()
            ));
        }

        Ok(Self {
            public_key,
            secret_key,
        })
    }

    /// Encapsulate a shared secret (client side)
    /// Returns (ciphertext, shared_secret)
    pub fn encapsulate(&self, _peer_public_key: &[u8]) -> Result<(Vec<u8>, SessionKey), String> {
        Err("Kyber key exchange is not available — enable 'kyber' feature".to_string())
    }

    /// Decapsulate a shared secret (server side)
    /// Returns the shared secret
    pub fn decapsulate(&self, _ciphertext: &[u8]) -> Result<SessionKey, String> {
        Err("Kyber key exchange is not available — enable 'kyber' feature".to_string())
    }

    /// Get public key
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// Get public key as bytes (for serialization)
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.public_key.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "kyber")]
    fn test_kyber_key_exchange() {
        // Generate two keypairs
        let alice = KyberKeyExchange::generate();
        let bob = KyberKeyExchange::generate();

        // Alice encapsulates to Bob
        let (ciphertext, alice_session) = alice.encapsulate(bob.public_key()).unwrap();

        // Bob decapsulates
        let bob_session = bob.decapsulate(&ciphertext).unwrap();

        // Both should have the same shared secret
        assert_eq!(alice_session, bob_session);
        assert_eq!(alice_session.as_bytes().len(), 32);
    }

    #[test]
    #[cfg(not(feature = "kyber"))]
    fn test_kyber_key_exchange_stub() {
        // When kyber feature is disabled, encapsulate/decapsulate should return errors
        let alice = KyberKeyExchange::generate();
        let bob = KyberKeyExchange::generate();

        assert!(alice.encapsulate(bob.public_key()).is_err());
        assert!(bob.decapsulate(&[0u8; 1088]).is_err());
    }
}
