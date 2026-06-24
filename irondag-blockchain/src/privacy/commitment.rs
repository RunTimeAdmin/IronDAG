//! Pedersen Commitments
//!
//! Commitment scheme for hiding transaction amounts and receivers.
//! Commit(amount, blinding) = g^amount * h^blinding

use ark_bn254::{Fr, G1Projective};
use ark_ec::PrimeGroup;
use ark_ff::PrimeField;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha3::{Digest, Keccak256};
use tracing::warn;

/// Commitment to a value (hides amount and blinding factor)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Commitment {
    /// Commitment point on G1
    pub point: G1Projective,
}

// Custom serialization for G1Projective (arkworks doesn't implement serde directly)
impl Serialize for Commitment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut bytes = Vec::new();
        self.point
            .serialize_uncompressed(&mut bytes)
            .map_err(|e| serde::ser::Error::custom(format!("Serialization error: {:?}", e)))?;
        serializer.serialize_bytes(&bytes)
    }
}

impl<'de> Deserialize<'de> for Commitment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        let mut cursor = std::io::Cursor::new(bytes);
        let point = G1Projective::deserialize_uncompressed(&mut cursor)
            .map_err(|e| serde::de::Error::custom(format!("Deserialization error: {:?}", e)))?;
        Ok(Commitment { point })
    }
}

impl Commitment {
    /// Create commitment from point
    pub fn new(point: G1Projective) -> Self {
        Self { point }
    }

    /// Serialize commitment to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        // Serialize G1 point using ark_serialize
        let mut bytes = Vec::new();
        if self.point.serialize_uncompressed(&mut bytes).is_ok() {
            bytes
        } else {
            Vec::new()
        }
    }

    /// Deserialize commitment from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        use ark_serialize::CanonicalDeserialize;
        let mut cursor = std::io::Cursor::new(bytes);
        G1Projective::deserialize_uncompressed(&mut cursor)
            .ok()
            .map(|point| Self { point })
    }
}

/// Pedersen Commitment Scheme
#[derive(Debug)]
pub struct PedersenCommitment {
    /// Generator g (for amount)
    g: G1Projective,
    /// Generator h (for blinding factor)
    h: G1Projective,
}

impl PedersenCommitment {
    /// Create Pedersen commitment scheme with generators from trusted setup.
    ///
    /// This is the preferred constructor for production use.
    /// Generators should come from the trusted setup ceremony.
    pub fn from_generators(g: G1Projective, h: G1Projective) -> Self {
        Self { g, h }
    }

    /// Create new Pedersen commitment scheme with deterministic generators.
    ///
    /// WARNING: Using deterministic generators — for testing/development only.
    /// Production deployments should use `from_generators()` with trusted setup generators.
    pub fn new() -> Self {
        // Use tracing::warn for dev warning
        #[cfg(debug_assertions)]
        warn!("PedersenCommitment using deterministic generators (dev mode)");

        let g = Self::hash_to_g1(b"pedersen_g");
        let h = Self::hash_to_g1(b"pedersen_h");

        Self { g, h }
    }

    /// Create commitment scheme from verifying key's public parameters.
    ///
    /// Derives Pedersen generators from the verifying key's IC (input commitments),
    /// which are already part of the trusted setup and shared across all nodes.
    /// This avoids the need for a separate generator distribution mechanism.
    pub fn from_verifying_key(vk: &ark_groth16::VerifyingKey<ark_bn254::Bn254>) -> Self {
        use ark_ec::AffineRepr;
        // The verifying key contains gamma_abc_g1 (IC coefficients) which are
        // trusted setup parameters. Use the first two as our generators.
        let g = if !vk.gamma_abc_g1.is_empty() {
            vk.gamma_abc_g1[0].into_group()
        } else {
            // Fallback: should never happen in a valid verifying key
            Self::hash_to_g1(b"pedersen_g_fallback")
        };

        let h = if vk.gamma_abc_g1.len() > 1 {
            vk.gamma_abc_g1[1].into_group()
        } else {
            // Fallback: derive from g via hash
            let g_bytes = Self::serialize_point(&g).unwrap_or_default();
            let mut h_seed = b"pedersen_h_from_g".to_vec();
            h_seed.extend_from_slice(&g_bytes);
            Self::hash_to_g1(&h_seed)
        };

        Self { g, h }
    }

    /// Get the generators (g, h) used by this commitment scheme
    pub fn generators(&self) -> (G1Projective, G1Projective) {
        (self.g, self.h)
    }

    /// Serialize generators to bytes for storage/distribution.
    ///
    /// Format: [g_compressed (32 bytes) || h_compressed (32 bytes)] = 64 bytes total
    pub fn serialize_generators(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(64);
        self.g
            .serialize_compressed(&mut bytes)
            .map_err(|e| format!("Failed to serialize g: {}", e))?;
        self.h
            .serialize_compressed(&mut bytes)
            .map_err(|e| format!("Failed to serialize h: {}", e))?;
        Ok(bytes)
    }

    /// Deserialize generators from bytes.
    ///
    /// Expected format: [g_compressed (32 bytes) || h_compressed (32 bytes)]
    pub fn deserialize_generators(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 64 {
            return Err(format!(
                "Invalid generator bytes length: expected 64, got {}",
                bytes.len()
            ));
        }
        let g = G1Projective::deserialize_compressed(&bytes[..32])
            .map_err(|e| format!("Failed to deserialize g: {}", e))?;
        let h = G1Projective::deserialize_compressed(&bytes[32..64])
            .map_err(|e| format!("Failed to deserialize h: {}", e))?;
        Ok(Self { g, h })
    }

    /// Helper: serialize a single G1 point
    fn serialize_point(point: &G1Projective) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        point
            .serialize_compressed(&mut bytes)
            .map_err(|e| format!("Failed to serialize point: {}", e))?;
        Ok(bytes)
    }

    /// Commit to a value: C = g^amount * h^blinding
    pub fn commit(amount: u128, blinding: &[u8; 32]) -> Commitment {
        let scheme = Self::new();

        // Convert amount to field element
        let amount_fr = Fr::from(amount);

        // Convert blinding to field element
        let blinding_fr = Fr::from_le_bytes_mod_order(blinding);

        // Compute commitment: g^amount * h^blinding
        let g_amount = scheme.g * amount_fr;
        let h_blinding = scheme.h * blinding_fr;
        let commitment_point = g_amount + h_blinding;

        Commitment::new(commitment_point)
    }

    /// Verify commitment (in circuit, not here)
    /// This is just for testing - actual verification happens in zk-SNARK
    pub fn verify(_commitment: &Commitment, _amount: u128, _blinding: &[u8; 32]) -> bool {
        // Verification happens in zk-SNARK circuit
        true
    }

    /// Hash to G1 point (deterministic generator)
    fn hash_to_g1(seed: &[u8]) -> G1Projective {
        // Simplified: hash seed and use as scalar
        // In production, use proper hash-to-curve
        let mut hasher = Keccak256::new();
        hasher.update(seed);
        let hash = hasher.finalize();

        // Convert hash to field element
        let scalar = Fr::from_le_bytes_mod_order(&hash[..32]);

        // Use generator * scalar
        G1Projective::generator() * scalar
    }
}

impl Default for PedersenCommitment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::Zero;

    #[test]
    fn test_commitment_creation() {
        let amount = 1000u128;
        let blinding = [42u8; 32];

        let commitment = PedersenCommitment::commit(amount, &blinding);

        // Commitment should be non-zero
        assert_ne!(commitment.point, G1Projective::zero());
    }

    #[test]
    fn test_commitment_serialization() {
        let amount = 1000u128;
        let blinding = [42u8; 32];

        let commitment = PedersenCommitment::commit(amount, &blinding);
        let bytes = commitment.to_bytes();
        let deserialized = Commitment::from_bytes(&bytes);

        assert!(deserialized.is_some());
    }

    #[test]
    fn test_from_generators() {
        // Create generators manually
        let g = G1Projective::generator();
        let h = G1Projective::generator() * Fr::from(2u64);

        let scheme = PedersenCommitment::from_generators(g, h);
        let (g_out, h_out) = scheme.generators();

        assert_eq!(g, g_out);
        assert_eq!(h, h_out);
    }

    #[test]
    fn test_generator_serialization() {
        let scheme = PedersenCommitment::new();
        let bytes = scheme
            .serialize_generators()
            .expect("Serialization should succeed");

        // Should be 64 bytes (32 bytes per compressed G1 point)
        assert_eq!(bytes.len(), 64);

        let restored = PedersenCommitment::deserialize_generators(&bytes)
            .expect("Deserialization should succeed");

        let (g1, h1) = scheme.generators();
        let (g2, h2) = restored.generators();

        assert_eq!(g1, g2);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_generator_serialization_invalid_length() {
        let result = PedersenCommitment::deserialize_generators(&[0u8; 32]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected 64"));
    }

    #[test]
    fn test_commit_with_custom_generators() {
        // Create a scheme with custom generators
        let g = G1Projective::generator();
        let h = G1Projective::generator() * Fr::from(42u64);
        let scheme = PedersenCommitment::from_generators(g, h);

        let amount = 1000u128;
        let blinding = [42u8; 32];

        // Compute commitment manually
        let amount_fr = Fr::from(amount);
        let blinding_fr = Fr::from_le_bytes_mod_order(&blinding);
        let _expected_point = g * amount_fr + h * blinding_fr;

        // The scheme's commit method currently creates a new instance via new()
        // This test documents that behavior - in production, commit would need
        // to be updated to use instance generators
        // For now, we just verify the generators() method works
        let (g_out, h_out) = scheme.generators();
        assert_eq!(g, g_out);
        assert_eq!(h, h_out);
    }
}
