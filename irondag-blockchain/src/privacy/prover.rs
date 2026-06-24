//! Proof Generation
//!
//! Generates zk-SNARK proofs for privacy operations.

use crate::privacy::circuit::PrivateTransferCircuit;
use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, Proof, ProvingKey};
use ark_relations::r1cs::ConstraintSynthesizer;
use ark_snark::SNARK;
use ark_std::rand::RngCore;

/// Privacy Prover
pub struct PrivacyProver {
    /// Proving key (from trusted setup)
    proving_key: ProvingKey<Bn254>,
}

impl PrivacyProver {
    /// Create new prover with proving key
    pub fn new(proving_key: ProvingKey<Bn254>) -> Self {
        Self { proving_key }
    }

    /// Generate proof for a circuit
    pub fn prove<C: ConstraintSynthesizer<Fr>>(
        &self,
        circuit: C,
        rng: &mut (impl RngCore + ark_std::rand::CryptoRng),
    ) -> Result<Proof<Bn254>, String> {
        // arkworks 0.4 API: Use Groth16::prove directly
        Groth16::<Bn254>::prove(&self.proving_key, circuit, rng)
            .map_err(|e| format!("Proof generation failed: {:?}", e))
    }

    /// Serialize proof to bytes
    pub fn serialize_proof(proof: &Proof<Bn254>) -> Result<Vec<u8>, String> {
        use ark_serialize::CanonicalSerialize;
        let mut bytes = Vec::new();
        proof
            .serialize_uncompressed(&mut bytes)
            .map_err(|e| format!("Proof serialization failed: {:?}", e))?;
        Ok(bytes)
    }

    /// Generate proof for private transfer
    pub fn prove_private_transfer<R: RngCore + ark_std::rand::CryptoRng>(
        &self,
        old_balance: u128,
        amount: u128,
        new_balance: u128,
        nullifier: Fr,
        commitment: Fr,
        rng: &mut R,
    ) -> Result<Proof<Bn254>, String> {
        let expected_new_balance = old_balance
            .checked_sub(amount)
            .ok_or_else(|| "Insufficient balance for transfer".to_string())?;
        if expected_new_balance != new_balance {
            return Err("Invalid balance transition".to_string());
        }

        // Create circuit with witness values
        let circuit = PrivateTransferCircuit {
            old_balance: Some(old_balance),
            amount: Some(amount),
            new_balance: Some(new_balance),
            nullifier,
            commitment: Some(commitment),
        };
        self.prove(circuit, rng)
    }
}
