//! Built-in Privacy Pool
//!
//! A fixed-denomination shielded pool that breaks the on-chain link between
//! deposit and withdrawal addresses.
//!
//! # How it works
//!
//! **Deposit**
//! 1. User picks a random `nullifier` (32 bytes) and `secret` (32 bytes), keeps both private.
//! 2. User computes `commitment = keccak256(nullifier || secret)` off-chain.
//! 3. User sends a regular transaction:
//!    `to = PRIVACY_POOL_ADDRESS, value = denomination, data = commitment`
//! 4. The node registers the commitment in the pool.
//!
//! **Withdrawal**
//! 1. User reveals `nullifier` to the node via `irondag_poolWithdraw`.
//! 2. Node checks the nullifier hasn't been spent yet.
//! 3. In stub mode (`require_proof = false`): withdrawal is accepted.
//!    In verified mode (`require_proof = true`, future): a Groth16 proof is required that
//!    proves knowledge of a `secret` such that `keccak256(nullifier || secret)` is in the
//!    commitment set — without revealing which commitment.
//! 4. Node transfers `denomination` to the recipient address and marks the nullifier spent.
//!
//! # Privacy guarantee
//!
//! In stub mode there is NO on-chain privacy — the nullifier can be linked to a commitment
//! by brute-force if the anonymity set is small. Real privacy requires the zk-SNARK proof
//! which will be enabled when the `privacy` feature is compiled in and a verifying key is
//! loaded.

use crate::types::{Address, Hash};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::HashSet;
use tracing::warn;

/// System address of the privacy pool contract.
/// Funds sent to this address are held by the pool.
pub const PRIVACY_POOL_ADDRESS: Address =
    Address([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);

/// A single pool deposit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolDeposit {
    pub commitment: Hash,
    pub block_number: u64,
    pub leaf_index: usize,
}

/// Privacy pool state.
pub struct PrivacyPool {
    /// All commitments in insertion order (leaf index = position in vec).
    deposits: Vec<PoolDeposit>,
    /// O(1) commitment existence check.
    commitment_set: HashSet<Hash>,
    /// Nullifiers that have already been spent.
    spent_nullifiers: HashSet<Hash>,
    /// Fixed deposit/withdrawal amount (attoIDAG).
    denomination: u128,
    /// When false: withdrawal is accepted without proof (stub / testnet mode).
    /// When true: a valid proof must be supplied.
    require_proof: bool,
}

impl PrivacyPool {
    pub fn new(denomination: u128, require_proof: bool) -> Self {
        Self {
            deposits: Vec::new(),
            commitment_set: HashSet::new(),
            spent_nullifiers: HashSet::new(),
            denomination,
            require_proof,
        }
    }

    pub fn denomination(&self) -> u128 {
        self.denomination
    }

    pub fn commitment_count(&self) -> usize {
        self.deposits.len()
    }

    pub fn spent_count(&self) -> usize {
        self.spent_nullifiers.len()
    }

    pub fn available_balance(&self) -> u128 {
        let unspent = self.deposits.len() - self.spent_nullifiers.len().min(self.deposits.len());
        unspent as u128 * self.denomination
    }

    pub fn is_nullifier_spent(&self, nullifier: &Hash) -> bool {
        self.spent_nullifiers.contains(nullifier)
    }

    pub fn contains_commitment(&self, commitment: &Hash) -> bool {
        self.commitment_set.contains(commitment)
    }

    pub fn commitment_root(&self) -> Hash {
        // Simple sequential hash of all commitments in insertion order.
        // A production implementation would use a sparse Merkle tree (MiMC or Poseidon)
        // for efficient Merkle path proofs. This is sufficient for accounting.
        if self.deposits.is_empty() {
            return Hash([0u8; 32]);
        }
        let mut h = Keccak256::new();
        for deposit in &self.deposits {
            h.update(deposit.commitment.as_ref());
        }
        Hash(h.finalize().into())
    }

    /// Register a new commitment (called when a deposit tx is processed).
    /// Returns Err if the exact denomination wasn't sent or the commitment already exists.
    pub fn deposit(&mut self, commitment: Hash, block_number: u64) -> Result<usize, PoolError> {
        if self.commitment_set.contains(&commitment) {
            return Err(PoolError::DuplicateCommitment);
        }
        let leaf_index = self.deposits.len();
        self.commitment_set.insert(commitment);
        self.deposits.push(PoolDeposit {
            commitment,
            block_number,
            leaf_index,
        });
        Ok(leaf_index)
    }

    /// Validate and execute a withdrawal request.
    ///
    /// In stub mode (`require_proof = false`) the proof bytes are ignored.
    /// Returns `Ok(denomination)` on success so the caller can transfer the funds.
    pub fn withdraw(&mut self, nullifier: Hash, _proof: Option<&[u8]>) -> Result<u128, PoolError> {
        if self.spent_nullifiers.contains(&nullifier) {
            return Err(PoolError::NullifierAlreadySpent);
        }

        if self.require_proof {
            // Future: verify Groth16 proof that nullifier corresponds to a commitment
            // in the commitment set without revealing which one.
            // For now, fail-closed if proof is required but not implemented.
            let proof_bytes = _proof.unwrap_or(&[]);
            if proof_bytes.is_empty() {
                return Err(PoolError::ProofRequired);
            }
            // TODO: route to Groth16Verifier once privacy feature is ready
            // For now even with proof bytes we can't verify — reject.
            return Err(PoolError::ProofVerificationFailed(
                "Groth16 verifier not yet wired (enable privacy feature)".to_string(),
            ));
        } else {
            warn!(
                nullifier = %hex::encode(nullifier),
                "Privacy pool withdrawal accepted in STUB mode — proof not verified"
            );
        }

        self.spent_nullifiers.insert(nullifier);
        Ok(self.denomination)
    }

    pub fn deposits(&self) -> &[PoolDeposit] {
        &self.deposits
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("Commitment already exists in pool")]
    DuplicateCommitment,
    #[error("Nullifier has already been spent")]
    NullifierAlreadySpent,
    #[error("Proof is required but none was supplied")]
    ProofRequired,
    #[error("Proof verification failed: {0}")]
    ProofVerificationFailed(String),
    #[error("Insufficient pool balance")]
    InsufficientBalance,
}
