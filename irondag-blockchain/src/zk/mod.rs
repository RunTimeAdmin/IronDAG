//! ZK (Zero-Knowledge) Module
//!
//! This module provides consensus-level ZK circuits for proving state transitions.
//! Unlike the `privacy` module which focuses on transaction privacy, this module
//! focuses on verifiable state transitions that can be validated by the consensus layer.
//!
//! ## Features
//!
//! - `StateTransitionCircuit`: Proves that a batch of transactions preserves total balance
//! - Groth16 proving and verification wrappers for BN254 curve
//!
//! ## Usage
//!
//! All ZK functionality is gated behind the `privacy` feature flag since it depends
//! on the optional ark-* crates.

pub mod state_transition;

#[cfg(feature = "privacy")]
pub use state_transition::{StateTransitionCircuit, VerklePathWitness};

#[cfg(feature = "privacy")]
use ark_bn254::Bn254;
#[cfg(feature = "privacy")]
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
#[cfg(feature = "privacy")]
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
#[cfg(feature = "privacy")]
use ark_snark::SNARK;
#[cfg(feature = "privacy")]
use std::io::Cursor;

/// Generate a Groth16 proof for a state transition batch
///
/// # Arguments
/// * `pk` - The proving key from trusted setup
/// * `circuit` - The state transition circuit with witness values
///
/// # Returns
/// * `Ok(Vec<u8>)` - Serialized proof bytes on success
/// * `Err(String)` - Error message on failure
///
/// # Example
/// ```ignore
/// use irondag::zk::{StateTransitionCircuit, prove_state_transition};
/// use ark_bn254::Fr;
/// use ark_groth16::ProvingKey;
///
/// let mut circuit = StateTransitionCircuit::new_batch(2);
/// circuit.set_transaction(0, Fr::from(100), Fr::from(1000), Fr::from(500));
/// circuit.set_public_inputs(Fr::from(1), Fr::from(2), Fr::from(3));
///
/// // Assuming pk is obtained from setup
/// let proof = prove_state_transition(&pk, circuit).unwrap();
/// ```
#[cfg(feature = "privacy")]
pub fn prove_state_transition(
    pk: &ProvingKey<Bn254>,
    circuit: StateTransitionCircuit<ark_bn254::Fr>,
) -> Result<Vec<u8>, String> {
    let start = std::time::Instant::now();

    // Create a random number generator
    let mut rng = ark_std::rand::thread_rng();

    // Generate the proof
    let proof = Groth16::<Bn254>::prove(pk, circuit, &mut rng)
        .map_err(|e| format!("Proof generation failed: {:?}", e))?;

    // Serialize the proof to bytes
    let mut bytes = Vec::new();
    proof
        .serialize_uncompressed(&mut bytes)
        .map_err(|e| format!("Proof serialization failed: {:?}", e))?;

    let elapsed = start.elapsed();
    tracing::info!(
        "Stream C ZK proof generated in {:.1}ms",
        elapsed.as_secs_f64() * 1000.0
    );

    if elapsed.as_millis() > 100 {
        tracing::warn!(
            "ZK proof exceeded 100ms target: {:.1}ms",
            elapsed.as_secs_f64() * 1000.0
        );
    }

    Ok(bytes)
}

/// Verify a Groth16 state transition proof
///
/// # Arguments
/// * `vk` - The verifying key (public)
/// * `proof_bytes` - Serialized proof bytes
/// * `public_inputs` - Array of public inputs (pre_state_hash, post_state_hash, transactions_root)
///
/// # Returns
/// * `Ok(true)` - Proof is valid
/// * `Ok(false)` - Proof is invalid
/// * `Err(String)` - Error during verification
///
/// # Example
/// ```ignore
/// use irondag::zk::verify_state_transition;
/// use ark_bn254::Fr;
/// use ark_groth16::VerifyingKey;
///
/// // Assuming vk, proof_bytes, and public_inputs are available
/// let is_valid = verify_state_transition(&vk, &proof_bytes, &public_inputs).unwrap();
/// assert!(is_valid);
/// ```
#[cfg(feature = "privacy")]
pub fn verify_state_transition(
    vk: &VerifyingKey<Bn254>,
    proof_bytes: &[u8],
    public_inputs: &[ark_bn254::Fr],
) -> Result<bool, String> {
    // Deserialize the proof
    let mut cursor = Cursor::new(proof_bytes);
    let proof = Proof::<Bn254>::deserialize_uncompressed(&mut cursor)
        .map_err(|e| format!("Proof deserialization failed: {:?}", e))?;

    // Verify the proof
    let result = Groth16::<Bn254>::verify(vk, public_inputs, &proof)
        .map_err(|e| format!("Verification error: {:?}", e))?;

    Ok(result)
}

/// Serialize a proving key to bytes
///
/// # Arguments
/// * `pk` - The proving key
///
/// # Returns
/// * `Ok(Vec<u8>)` - Serialized proving key
/// * `Err(String)` - Error message on failure
#[cfg(feature = "privacy")]
pub fn serialize_proving_key(pk: &ProvingKey<Bn254>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    pk.serialize_uncompressed(&mut bytes)
        .map_err(|e| format!("Proving key serialization failed: {:?}", e))?;
    Ok(bytes)
}

/// Deserialize a proving key from bytes
///
/// # Arguments
/// * `bytes` - Serialized proving key bytes
///
/// # Returns
/// * `Ok(ProvingKey)` - Deserialized proving key
/// * `Err(String)` - Error message on failure
#[cfg(feature = "privacy")]
pub fn deserialize_proving_key(bytes: &[u8]) -> Result<ProvingKey<Bn254>, String> {
    let mut cursor = Cursor::new(bytes);
    ProvingKey::<Bn254>::deserialize_uncompressed(&mut cursor)
        .map_err(|e| format!("Proving key deserialization failed: {:?}", e))
}

/// Serialize a verifying key to bytes
///
/// # Arguments
/// * `vk` - The verifying key
///
/// # Returns
/// * `Ok(Vec<u8>)` - Serialized verifying key
/// * `Err(String)` - Error message on failure
#[cfg(feature = "privacy")]
pub fn serialize_verifying_key(vk: &VerifyingKey<Bn254>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    vk.serialize_uncompressed(&mut bytes)
        .map_err(|e| format!("Verifying key serialization failed: {:?}", e))?;
    Ok(bytes)
}

/// Deserialize a verifying key from bytes
///
/// # Arguments
/// * `bytes` - Serialized verifying key bytes
///
/// # Returns
/// * `Ok(VerifyingKey)` - Deserialized verifying key
/// * `Err(String)` - Error message on failure
#[cfg(feature = "privacy")]
pub fn deserialize_verifying_key(bytes: &[u8]) -> Result<VerifyingKey<Bn254>, String> {
    let mut cursor = Cursor::new(bytes);
    VerifyingKey::<Bn254>::deserialize_uncompressed(&mut cursor)
        .map_err(|e| format!("Verifying key deserialization failed: {:?}", e))
}

/// Generate proving and verifying keys for a state transition circuit
///
/// This performs circuit-specific setup (trusted setup alternative for testing).
/// In production, a universal trusted setup should be used.
///
/// # Arguments
/// * `circuit` - The state transition circuit (used to determine circuit structure)
///
/// # Returns
/// * `Ok((ProvingKey, VerifyingKey))` - The generated keys
/// * `Err(String)` - Error message on failure
#[cfg(feature = "privacy")]
pub fn setup_state_transition_circuit(
    circuit: StateTransitionCircuit<ark_bn254::Fr>,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), String> {
    let mut rng = ark_std::rand::thread_rng();

    Groth16::<Bn254>::circuit_specific_setup(circuit, &mut rng)
        .map_err(|e| format!("Circuit setup failed: {:?}", e))
}

/// Load proving key from file
///
/// # Arguments
/// * `path` - Path to the proving key file
///
/// # Returns
/// * `Ok(ProvingKey)` - The loaded proving key
/// * `Err(String)` - Error message on failure
///
/// # Example
/// ```ignore
/// use irondag::zk::load_proving_key;
///
/// let pk = load_proving_key("data/zk/proving_key.bin").unwrap();
/// ```
#[cfg(feature = "privacy")]
pub fn load_proving_key(path: &str) -> Result<ProvingKey<Bn254>, String> {
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path)
        .map_err(|e| format!("Failed to open proving key file '{}': {:?}", path, e))?;
    let reader = BufReader::new(file);

    ProvingKey::<Bn254>::deserialize_uncompressed(reader)
        .map_err(|e| format!("Failed to deserialize proving key from '{}': {:?}", path, e))
}

/// Load verifying key from file
///
/// # Arguments
/// * `path` - Path to the verifying key file
///
/// # Returns
/// * `Ok(VerifyingKey)` - The loaded verifying key
/// * `Err(String)` - Error message on failure
///
/// # Example
/// ```ignore
/// use irondag::zk::load_verifying_key;
///
/// let vk = load_verifying_key("data/zk/verifying_key.bin").unwrap();
/// ```
#[cfg(feature = "privacy")]
pub fn load_verifying_key(path: &str) -> Result<VerifyingKey<Bn254>, String> {
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path)
        .map_err(|e| format!("Failed to open verifying key file '{}': {:?}", path, e))?;
    let reader = BufReader::new(file);

    VerifyingKey::<Bn254>::deserialize_uncompressed(reader).map_err(|e| {
        format!(
            "Failed to deserialize verifying key from '{}': {:?}",
            path, e
        )
    })
}

#[cfg(test)]
#[cfg(feature = "privacy")]
mod tests {
    use super::*;
    use ark_bn254::Fr;
    use ark_std::rand::SeedableRng;

    #[test]
    fn test_prove_and_verify_state_transition() {
        let rng = ark_std::rand::rngs::StdRng::seed_from_u64(42u64);

        // Create and configure circuit
        let mut circuit = StateTransitionCircuit::new_batch(2);
        circuit.set_transaction(0, Fr::from(100u64), Fr::from(1000u64), Fr::from(500u64));
        circuit.set_transaction(1, Fr::from(50u64), Fr::from(200u64), Fr::from(300u64));
        circuit.set_public_inputs(Fr::from(12345u64), Fr::from(67890u64), Fr::from(11111u64));

        // Setup
        let (pk, vk) =
            setup_state_transition_circuit(circuit.clone()).expect("Setup should succeed");

        // Generate proof
        let proof_bytes =
            prove_state_transition(&pk, circuit.clone()).expect("Proof generation should succeed");

        // Verify proof
        let public_inputs = circuit.public_inputs();
        let is_valid = verify_state_transition(&vk, &proof_bytes, &public_inputs)
            .expect("Verification should not error");

        assert!(is_valid, "Valid proof should verify successfully");
    }

    #[test]
    fn test_key_serialization() {
        let mut circuit = StateTransitionCircuit::new_batch(1);
        circuit.set_transaction(0, Fr::from(100u64), Fr::from(1000u64), Fr::from(500u64));
        circuit.set_public_inputs(Fr::from(1u64), Fr::from(2u64), Fr::from(3u64));

        let (pk, vk) = setup_state_transition_circuit(circuit).expect("Setup should succeed");

        // Serialize and deserialize proving key
        let pk_bytes = serialize_proving_key(&pk).expect("PK serialization should succeed");
        let pk_deserialized =
            deserialize_proving_key(&pk_bytes).expect("PK deserialization should succeed");

        // Serialize and deserialize verifying key
        let vk_bytes = serialize_verifying_key(&vk).expect("VK serialization should succeed");
        let vk_deserialized =
            deserialize_verifying_key(&vk_bytes).expect("VK deserialization should succeed");

        // Keys should be identical after roundtrip
        let pk_bytes2 =
            serialize_proving_key(&pk_deserialized).expect("PK serialization should succeed");
        let vk_bytes2 =
            serialize_verifying_key(&vk_deserialized).expect("VK serialization should succeed");

        assert_eq!(
            pk_bytes, pk_bytes2,
            "Proving key should be identical after roundtrip"
        );
        assert_eq!(
            vk_bytes, vk_bytes2,
            "Verifying key should be identical after roundtrip"
        );
    }

    #[test]
    fn test_verify_with_wrong_public_inputs() {
        let mut circuit = StateTransitionCircuit::new_batch(1);
        circuit.set_transaction(0, Fr::from(100u64), Fr::from(1000u64), Fr::from(500u64));
        circuit.set_public_inputs(Fr::from(1u64), Fr::from(2u64), Fr::from(3u64));

        let (pk, vk) =
            setup_state_transition_circuit(circuit.clone()).expect("Setup should succeed");

        let proof_bytes =
            prove_state_transition(&pk, circuit).expect("Proof generation should succeed");

        // Try to verify with wrong public inputs
        let wrong_inputs = vec![Fr::from(999u64), Fr::from(2u64), Fr::from(3u64)];
        let is_valid = verify_state_transition(&vk, &proof_bytes, &wrong_inputs)
            .expect("Verification should not error");

        assert!(
            !is_valid,
            "Proof with wrong public inputs should fail verification"
        );
    }
}
