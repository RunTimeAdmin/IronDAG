//! Privacy Manager
//!
//! Manages privacy operations, nullifiers, and commitments.

use crate::privacy::{Commitment, Nullifier, NullifierSet, PrivacyTransaction, PrivacyVerifier};
use crate::types::Address;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Privacy Manager
pub struct PrivacyManager {
    /// Nullifier set (tracks spent notes)
    nullifier_set: Arc<RwLock<NullifierSet>>,
    /// Commitment to note mapping (for tracking)
    #[allow(dead_code)]
    commitments: Arc<RwLock<HashMap<Commitment, Address>>>,
    /// Privacy enabled flag
    enabled: bool,
    /// Privacy verifier (for proof verification)
    verifier: Option<Arc<PrivacyVerifier>>,
}

impl PrivacyManager {
    /// Create new privacy manager
    pub fn new(enabled: bool) -> Self {
        Self {
            nullifier_set: Arc::new(RwLock::new(NullifierSet::default())),
            commitments: Arc::new(RwLock::new(HashMap::new())),
            enabled,
            verifier: None,
        }
    }

    /// Create privacy manager with verifier
    pub fn with_verifier(enabled: bool, verifier: PrivacyVerifier) -> Self {
        Self {
            nullifier_set: Arc::new(RwLock::new(NullifierSet::default())),
            commitments: Arc::new(RwLock::new(HashMap::new())),
            enabled,
            verifier: Some(Arc::new(verifier)),
        }
    }

    /// Set verifier
    pub fn set_verifier(&mut self, verifier: PrivacyVerifier) {
        self.verifier = Some(Arc::new(verifier));
    }

    /// Check if privacy is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if nullifier exists (already spent)
    pub async fn is_nullifier_spent(&self, nullifier: &Nullifier) -> bool {
        let set = self.nullifier_set.read().await;
        set.contains(nullifier)
    }

    /// Add nullifier (mark as spent)
    pub async fn add_nullifier(&self, nullifier: Nullifier) -> Result<(), String> {
        let mut set = self.nullifier_set.write().await;
        set.add(nullifier)
    }

    /// Verify proof with public inputs
    pub async fn verify_proof(
        &self,
        proof: &ark_groth16::Proof<ark_bn254::Bn254>,
        public_inputs: &[ark_bn254::Fr],
    ) -> bool {
        if let Some(ref verifier) = self.verifier {
            verifier.verify_with_inputs(proof, public_inputs)
        } else {
            false
        }
    }

    /// Process privacy transaction
    pub async fn process_transaction(&self, tx: &PrivacyTransaction) -> Result<(), String> {
        if !self.enabled {
            return Err("Privacy layer is disabled".to_string());
        }

        // Verify proof if verifier is available
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            "Privacy layer enabled but verifier not loaded — cannot process transaction".to_string()
        })?;

        // Deserialize proof
        let proof = PrivacyVerifier::deserialize_proof(&tx.proof)
            .map_err(|e| format!("Failed to deserialize proof: {}", e))?;

        // Parse public inputs (nullifier, commitment)
        use ark_bn254::Fr;
        use ark_ff::PrimeField;
        let mut public_inputs = Vec::new();
        for input_bytes in &tx.public_inputs {
            // H8: Enforce exactly 32 bytes — reject truncated or padded inputs
            if input_bytes.len() != 32 {
                return Err(format!(
                    "Privacy input must be exactly 32 bytes, got {}",
                    input_bytes.len()
                ));
            }
            let fr = Fr::from_le_bytes_mod_order(input_bytes.as_slice().try_into().unwrap());
            public_inputs.push(fr);
        }

        // Verify proof
        if !verifier.verify_with_inputs(&proof, &public_inputs) {
            return Err("Privacy proof verification failed".to_string());
        }

        if let Some(nullifier) = extract_nullifier_from_inputs(&tx.public_inputs) {
            // Check if already spent
            if self.is_nullifier_spent(&nullifier).await {
                return Err("Nullifier already spent (double-spend attempt)".to_string());
            }

            // Add nullifier to set (mark as spent)
            self.add_nullifier(nullifier).await?;
        }

        Ok(())
    }

    /// Extract nullifier from privacy transaction
    pub fn extract_nullifier(tx: &PrivacyTransaction) -> Option<Nullifier> {
        extract_nullifier_from_inputs(&tx.public_inputs)
    }

    /// Get nullifier set size
    pub async fn nullifier_count(&self) -> usize {
        let set = self.nullifier_set.read().await;
        set.len()
    }
}

impl Default for PrivacyManager {
    fn default() -> Self {
        Self::new(true)
    }
}

fn extract_nullifier_from_inputs(public_inputs: &[Vec<u8>]) -> Option<Nullifier> {
    let nullifier_index = if public_inputs.len() >= 2 { 1 } else { 0 };
    if let Some(nullifier_bytes) = public_inputs.get(nullifier_index) {
        let mut nullifier_hash = [0u8; 32];
        if nullifier_bytes.len() >= 32 {
            nullifier_hash.copy_from_slice(&nullifier_bytes[..32]);
            return Some(Nullifier {
                hash: nullifier_hash,
            });
        }
        if !nullifier_bytes.is_empty() {
            nullifier_hash[..nullifier_bytes.len()].copy_from_slice(nullifier_bytes);
            return Some(Nullifier {
                hash: nullifier_hash,
            });
        }
    }
    None
}
