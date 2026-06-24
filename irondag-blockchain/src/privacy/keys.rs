//! Key Generation (Trusted Setup)
//!
//! Generates proving and verifying keys for zk-SNARK circuits.
//! In production, this would use a trusted setup ceremony.

use crate::privacy::circuit::PrivateTransferCircuit;
use crate::privacy::commitment::PedersenCommitment;
use ark_bn254::{Bn254, Fr, G1Projective};
use ark_groth16::{Groth16, ProvingKey, VerifyingKey};
use ark_std::rand::RngCore;

/// Generate proving and verifying keys for private transfer circuit
pub fn generate_keys<R: RngCore + ark_std::rand::CryptoRng>(
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), String> {
    // Create a dummy circuit for key generation
    // The circuit structure must match the actual circuit
    let circuit = PrivateTransferCircuit {
        old_balance: None, // No witness values needed for key generation
        amount: None,
        new_balance: None,
        nullifier: Fr::from(0u64),
        commitment: None,
    };

    // Generate keys using Groth16
    // arkworks 0.4 API: Use Groth16::generate_random_parameters_with_reduction
    // Note: This is a simplified implementation for testing
    // In production, use a trusted setup ceremony
    let pk = Groth16::<Bn254>::generate_random_parameters_with_reduction(circuit, rng)
        .map_err(|e| format!("Key generation failed: {:?}", e))?;
    // Extract verifying key from proving key
    let vk = pk.vk.clone();
    Ok((pk, vk))
}

/// Load keys from bytes (for production use with trusted setup)
pub fn load_keys_from_bytes(
    pk_bytes: &[u8],
    vk_bytes: &[u8],
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), String> {
    use ark_serialize::CanonicalDeserialize;
    let mut cursor = std::io::Cursor::new(pk_bytes);
    let pk = ProvingKey::<Bn254>::deserialize_uncompressed(&mut cursor)
        .map_err(|e| format!("Failed to deserialize proving key: {:?}", e))?;

    let mut cursor = std::io::Cursor::new(vk_bytes);
    let vk = VerifyingKey::<Bn254>::deserialize_uncompressed(&mut cursor)
        .map_err(|e| format!("Failed to deserialize verifying key: {:?}", e))?;

    Ok((pk, vk))
}

/// Serialize keys to bytes
pub fn serialize_keys(
    pk: &ProvingKey<Bn254>,
    vk: &VerifyingKey<Bn254>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    use ark_serialize::CanonicalSerialize;
    let mut pk_bytes = Vec::new();
    pk.serialize_uncompressed(&mut pk_bytes)
        .map_err(|e| format!("Failed to serialize proving key: {:?}", e))?;

    let mut vk_bytes = Vec::new();
    vk.serialize_uncompressed(&mut vk_bytes)
        .map_err(|e| format!("Failed to serialize verifying key: {:?}", e))?;

    Ok((pk_bytes, vk_bytes))
}

/// Load proving and verifying keys from files (for production trusted setup).
/// Returns (ProvingKey, VerifyingKey) so all nodes use the same keys and proofs verify across nodes.
pub fn load_keys_from_paths(
    proving_key_path: &std::path::Path,
    verifying_key_path: &std::path::Path,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), String> {
    let pk_bytes = std::fs::read(proving_key_path).map_err(|e| {
        format!(
            "Failed to read proving key from {}: {}",
            proving_key_path.display(),
            e
        )
    })?;
    let vk_bytes = std::fs::read(verifying_key_path).map_err(|e| {
        format!(
            "Failed to read verifying key from {}: {}",
            verifying_key_path.display(),
            e
        )
    })?;
    load_keys_from_bytes(&pk_bytes, &vk_bytes)
}

/// Derive Pedersen commitment generators from the verifying key.
///
/// The verifying key's `gamma_abc_g1` field contains trusted setup parameters
/// (IC coefficients for verifying Groth16 proofs). By deriving generators from
/// these, we ensure all nodes use consistent generators without additional setup.
///
/// Returns (g, h) where:
/// - g = first IC coefficient (gamma_abc_g1[0])
/// - h = second IC coefficient (gamma_abc_g1[1]) or hash-derived if only one exists
pub fn derive_pedersen_generators(vk: &VerifyingKey<Bn254>) -> (G1Projective, G1Projective) {
    let scheme = PedersenCommitment::from_verifying_key(vk);
    scheme.generators()
}

/// Get Pedersen commitment scheme from verifying key.
///
/// Convenience function that creates a PedersenCommitment instance
/// using generators derived from the trusted setup.
pub fn get_pedersen_commitment(vk: &VerifyingKey<Bn254>) -> PedersenCommitment {
    PedersenCommitment::from_verifying_key(vk)
}
