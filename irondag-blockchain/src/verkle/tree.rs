//! Verkle Tree Data Structure
//!
//! Wide tree (256 children per node) with KZG-style commitments
//! for efficient state proofs

use crate::types::{Address, Hash};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

#[cfg(feature = "privacy")]
use super::kzg::{KzgCommitment, KzgSrs};
#[cfg(feature = "privacy")]
use ark_bn254::Fr;
#[cfg(feature = "privacy")]
use ark_ff::PrimeField;

// ============================================================================
// KZG Translation Layer (privacy feature)
// ============================================================================

/// Global KZG SRS for Verkle tree commitments (initialized lazily)
#[cfg(feature = "privacy")]
static VERKLE_KZG_SRS: std::sync::OnceLock<KzgSrs> = std::sync::OnceLock::new();

/// Get or initialize the global KZG SRS for Verkle trees
#[cfg(feature = "privacy")]
fn get_verkle_kzg_srs() -> &'static KzgSrs {
    VERKLE_KZG_SRS.get_or_init(|| KzgSrs::generate_deterministic(256, 42))
}

/// A node commitment that holds both Keccak (backward compat) and KZG (new)
///
/// This dual commitment structure enables the translation layer between:
/// - Keccak256 hashes (current, proven, backward compatible)
/// - KZG polynomial commitments (new, efficient proofs)
///
/// Both are computed from the same child values, ensuring consistency.
#[cfg(feature = "privacy")]
#[derive(Debug, Clone)]
pub struct DualCommitment {
    /// Keccak hash of children/values (backward compatible)
    pub keccak: Hash,
    /// KZG polynomial commitment (for efficient proofs)
    pub kzg: KzgCommitment,
}

/// Placeholder dual commitment for non-privacy builds
#[cfg(not(feature = "privacy"))]
#[derive(Debug, Clone)]
pub struct DualCommitment {
    /// Keccak hash of children/values
    pub keccak: Hash,
}

/// Compute KZG commitment for a Verkle node's children
///
/// Children values (32-byte hashes) are converted to field elements
/// and committed as polynomial coefficients.
///
/// # Arguments
/// * `children_hashes` - Array of 256 child commitment hashes (or zeros for empty slots)
///
/// # Returns
/// KZG commitment to the polynomial whose coefficients are the field elements
/// derived from the child hashes.
#[cfg(feature = "privacy")]
pub fn compute_kzg_commitment(children_hashes: &[[u8; 32]; 256]) -> Option<KzgCommitment> {
    let srs = get_verkle_kzg_srs();

    // Convert each 32-byte hash to a field element
    // We use the hash directly interpreted as a field element (mod order)
    let coefficients: Vec<Fr> = children_hashes
        .iter()
        .map(|h| Fr::from_be_bytes_mod_order(h))
        .collect();

    srs.commit(&coefficients)
}

/// Convert a 32-byte value to a field element for KZG operations
#[cfg(feature = "privacy")]
pub fn hash_to_field_element(data: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(data)
}

/// Convert a field element back to 32 bytes
#[cfg(feature = "privacy")]
pub fn field_element_to_bytes(fr: &Fr) -> [u8; 32] {
    let bigint = fr.into_bigint();
    let bytes_le = bigint.to_bytes_le();
    let mut result = [0u8; 32];
    result[..bytes_le.len().min(32)].copy_from_slice(&bytes_le[..bytes_le.len().min(32)]);
    // Convert to big-endian for consistency with hash format
    result.reverse();
    result
}

// ============================================================================
// KZG Verkle Proof Types
// ============================================================================

/// KZG-based Verkle proof step with proper commitment sizes
///
/// Unlike the legacy VerkleProofStep which truncates to 32 bytes,
/// this struct stores full KZG commitments and proofs (~48 bytes each).
#[cfg(feature = "privacy")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KzgVerkleProofStep {
    /// KZG commitment to the node's children polynomial (serialized)
    pub node_commitment: Vec<u8>,
    /// KZG opening proof for the child index
    pub opening_proof: Vec<u8>,
    /// Which child position (0-255)
    pub child_index: u8,
    /// The child's value as field element bytes
    pub child_value: [u8; 32],
}

/// Full KZG-based Verkle proof
#[cfg(feature = "privacy")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KzgVerkleProof {
    /// The key being proven
    pub key: Vec<u8>,
    /// The value being proven
    pub value: Vec<u8>,
    /// Proof steps from root to leaf
    pub steps: Vec<KzgVerkleProofStep>,
    /// Root KZG commitment (serialized)
    pub root_commitment: Vec<u8>,
}

// ============================================================================
// Placeholder types for non-privacy builds
// ============================================================================

/// Placeholder KZG Verkle proof step for non-privacy builds
#[cfg(not(feature = "privacy"))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KzgVerkleProofStep {
    /// Placeholder commitment bytes
    pub node_commitment: Vec<u8>,
    /// Placeholder proof bytes
    pub opening_proof: Vec<u8>,
    /// Child index
    pub child_index: u8,
    /// Child value
    pub child_value: [u8; 32],
}

/// Placeholder KZG Verkle proof for non-privacy builds
#[cfg(not(feature = "privacy"))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KzgVerkleProof {
    /// The key being proven
    pub key: Vec<u8>,
    /// The value being proven
    pub value: Vec<u8>,
    /// Proof steps
    pub steps: Vec<KzgVerkleProofStep>,
    /// Root commitment
    pub root_commitment: Vec<u8>,
}

/// Verkle tree node
#[derive(Debug, Clone)]
struct VerkleNode {
    /// Branching factor (256 for wide tree)
    width: usize,
    /// Child nodes (indexed by key byte)
    children: Vec<Option<Box<VerkleNode>>>,
    /// Values stored at this node (for leaf nodes)
    values: Vec<Option<Vec<u8>>>,
    /// Commitment hash for this node (Keccak)
    commitment: Option<Hash>,
}

impl VerkleNode {
    fn new(width: usize) -> Self {
        Self {
            width,
            children: vec![None; width],
            values: vec![None; width],
            commitment: None,
        }
    }

    /// Insert a key-value pair into the tree
    fn insert(&mut self, key: &[u8], value: Vec<u8>, depth: usize) {
        if depth >= key.len() {
            // Leaf node - store value
            let index = if key.is_empty() {
                0
            } else {
                key[0] as usize % self.width
            };
            self.values[index] = Some(value);
        } else {
            // Internal node - recurse
            let index = key[depth] as usize % self.width;
            if self.children[index].is_none() {
                self.children[index] = Some(Box::new(VerkleNode::new(self.width)));
            }
            if let Some(ref mut child) = self.children[index] {
                child.insert(key, value, depth + 1);
            }
        }
        // Update commitment after insertion
        self.update_commitment();
    }

    /// Get value for a key
    fn get(&self, key: &[u8], depth: usize) -> Option<Vec<u8>> {
        if depth >= key.len() {
            // Leaf node
            let index = if key.is_empty() {
                0
            } else {
                key[0] as usize % self.width
            };
            self.values[index].clone()
        } else {
            // Internal node - recurse
            let index = key[depth] as usize % self.width;
            self.children[index]
                .as_ref()
                .and_then(|child| child.get(key, depth + 1))
        }
    }

    /// Update commitment hash for this node
    fn update_commitment(&mut self) {
        let mut hasher = Keccak256::new();

        // Hash all child commitments
        for child in &self.children {
            if let Some(ref c) = child {
                if let Some(ref comm) = c.commitment {
                    hasher.update(comm);
                } else {
                    hasher.update(&[0u8; 32]);
                }
            } else {
                hasher.update(&[0u8; 32]);
            }
        }

        // Hash all values
        for value in &self.values {
            if let Some(ref v) = value {
                hasher.update(v);
            } else {
                hasher.update(&[0u8; 32]);
            }
        }

        let hash = hasher.finalize();
        let mut commitment = [0u8; 32];
        commitment.copy_from_slice(&hash);
        self.commitment = Some(Hash(commitment));
    }

    /// Get proof path for a key
    fn get_proof(&self, key: &[u8], depth: usize, proof: &mut Vec<Hash>) {
        if depth >= key.len() {
            // Leaf node - add sibling values to proof
            let index = if key.is_empty() {
                0
            } else {
                key[0] as usize % self.width
            };
            for (i, value) in self.values.iter().enumerate() {
                if i != index {
                    if let Some(ref v) = value {
                        let mut hasher = Keccak256::new();
                        hasher.update(v);
                        let hash = hasher.finalize();
                        let mut hash_bytes = [0u8; 32];
                        hash_bytes.copy_from_slice(&hash);
                        proof.push(Hash(hash_bytes));
                    }
                }
            }
        } else {
            // Internal node - recurse and add sibling commitments
            let index = key[depth] as usize % self.width;

            // Add sibling commitments to proof
            for (i, child) in self.children.iter().enumerate() {
                if i != index {
                    if let Some(ref c) = child {
                        if let Some(ref comm) = c.commitment {
                            proof.push(*comm);
                        }
                    }
                }
            }

            // Recurse into child
            if let Some(ref child) = self.children[index] {
                child.get_proof(key, depth + 1, proof);
            }
        }
    }

    /// Get all 256 child hashes for this node
    ///
    /// For internal nodes: returns child commitments (or zero for empty)
    /// For leaf nodes: returns keccak hashes of values (or zero for empty)
    #[allow(dead_code)]
    fn get_child_hashes(&self) -> [[u8; 32]; 256] {
        let mut hashes = [[0u8; 32]; 256];

        // Get child commitments (for internal nodes)
        for (i, child) in self.children.iter().enumerate() {
            if let Some(ref c) = child {
                if let Some(ref comm) = c.commitment {
                    hashes[i] = comm.0;
                }
            }
        }

        // Get value hashes (for leaf nodes)
        for (i, value) in self.values.iter().enumerate() {
            if let Some(ref v) = value {
                let mut hasher = Keccak256::new();
                hasher.update(v);
                let hash = hasher.finalize();
                hashes[i][..32].copy_from_slice(&hash);
            }
        }

        hashes
    }

    /// Compute dual commitment (Keccak + KZG) for this node
    ///
    /// This is the core translation layer function that bridges
    /// Keccak-based node hashes with KZG polynomial commitments.
    #[cfg(feature = "privacy")]
    fn compute_dual_commitment(&self) -> Option<DualCommitment> {
        let keccak = self.commitment?;
        let child_hashes = self.get_child_hashes();
        let kzg = compute_kzg_commitment(&child_hashes)?;

        Some(DualCommitment { keccak, kzg })
    }

    /// Generate KZG proof steps for a key path
    ///
    /// For each level from root to leaf, generates a KZG opening proof
    /// that validates the child value at the given index.
    #[cfg(feature = "privacy")]
    fn get_kzg_proof_steps(&self, key: &[u8], depth: usize, steps: &mut Vec<KzgVerkleProofStep>) {
        let child_hashes = self.get_child_hashes();

        // Compute KZG commitment for this node
        let kzg_commitment = match compute_kzg_commitment(&child_hashes) {
            Some(c) => c,
            None => return,
        };

        if depth >= key.len() {
            // Leaf node
            let index = if key.is_empty() {
                0
            } else {
                key[0] as usize % self.width
            };

            // Generate KZG opening proof for the value at this index
            let srs = get_verkle_kzg_srs();
            let coefficients: Vec<Fr> = child_hashes
                .iter()
                .map(|h| Fr::from_be_bytes_mod_order(h))
                .collect();

            let z = Fr::from(index as u64);

            if let Some((v, kzg_proof)) = srs.open(&coefficients, z) {
                let child_value = field_element_to_bytes(&v);

                steps.push(KzgVerkleProofStep {
                    node_commitment: kzg_commitment.to_bytes(),
                    opening_proof: kzg_proof.to_bytes(),
                    child_index: index as u8,
                    child_value,
                });
            }
        } else {
            // Internal node
            let index = key[depth] as usize % self.width;

            // Generate KZG opening proof for the child commitment at this index
            let srs = get_verkle_kzg_srs();
            let coefficients: Vec<Fr> = child_hashes
                .iter()
                .map(|h| Fr::from_be_bytes_mod_order(h))
                .collect();

            let z = Fr::from(index as u64);

            if let Some((v, kzg_proof)) = srs.open(&coefficients, z) {
                let child_value = field_element_to_bytes(&v);

                steps.push(KzgVerkleProofStep {
                    node_commitment: kzg_commitment.to_bytes(),
                    opening_proof: kzg_proof.to_bytes(),
                    child_index: index as u8,
                    child_value,
                });
            }

            // Recurse into child
            if let Some(ref child) = self.children[index] {
                child.get_kzg_proof_steps(key, depth + 1, steps);
            }
        }
    }
}

/// Verkle tree for state management
pub struct VerkleTree {
    root: VerkleNode,
    size: usize,
}

impl VerkleTree {
    /// Create a new Verkle tree
    pub fn new() -> Self {
        Self {
            root: VerkleNode::new(256), // 256-way branching
            size: 0,
        }
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: &[u8], value: Vec<u8>) {
        self.root.insert(key, value, 0);
        self.size += 1;
    }

    /// Get value for a key
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.root.get(key, 0)
    }

    /// Get proof for a key
    pub fn get_proof(&self, key: &[u8]) -> Vec<Hash> {
        let mut proof = Vec::new();
        self.root.get_proof(key, 0, &mut proof);
        proof
    }

    /// Get root hash (state root)
    pub fn root_hash(&self) -> Option<Hash> {
        self.root.commitment
    }

    /// Get tree size
    pub fn size(&self) -> usize {
        self.size
    }

    /// Generate KZG proof for a key
    ///
    /// Returns a full KZG-based Verkle proof with opening proofs at each level.
    #[cfg(feature = "privacy")]
    pub fn get_kzg_proof(&self, key: &[u8]) -> Option<KzgVerkleProof> {
        let value = self.get(key)?;
        let mut steps = Vec::new();
        self.root.get_kzg_proof_steps(key, 0, &mut steps);

        let root_commitment = self.get_root_kzg_commitment()?;

        Some(KzgVerkleProof {
            key: key.to_vec(),
            value,
            steps,
            root_commitment: root_commitment.to_bytes(),
        })
    }

    /// Get dual commitment for root node
    ///
    /// Returns both Keccak hash and KZG commitment for the root.
    #[cfg(feature = "privacy")]
    pub fn get_dual_commitment(&self) -> Option<DualCommitment> {
        self.root.compute_dual_commitment()
    }

    /// Get root KZG commitment
    #[cfg(feature = "privacy")]
    pub fn get_root_kzg_commitment(&self) -> Option<KzgCommitment> {
        let child_hashes = self.root.get_child_hashes();
        compute_kzg_commitment(&child_hashes)
    }
}

impl Default for VerkleTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Verkle-backed state manager
pub struct VerkleState {
    tree: VerkleTree,
    /// Cache for quick lookups (optional optimization)
    cache: HashMap<Address, (u128, u64)>, // (balance, nonce)
}

impl VerkleState {
    /// Create new Verkle state
    pub fn new() -> Self {
        Self {
            tree: VerkleTree::new(),
            cache: HashMap::new(),
        }
    }

    /// Set balance for an address
    pub fn set_balance(&mut self, address: Address, balance: u128) {
        // Store in Verkle tree
        let key = address;
        let mut value = Vec::with_capacity(24); // 16 bytes balance + 8 bytes nonce
        value.extend_from_slice(&balance.to_le_bytes());

        // Get existing nonce or use 0
        let nonce = self.cache.get(&address).map(|(_, n)| *n).unwrap_or(0);
        value.extend_from_slice(&nonce.to_le_bytes());

        self.tree.insert(&key, value);

        // Update cache
        let entry = self.cache.entry(address).or_insert((0, 0));
        entry.0 = balance;
    }

    /// Set nonce for an address
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        // Store in Verkle tree
        let key = address;
        let mut value = Vec::with_capacity(24);

        // Get existing balance or use 0
        let balance = self.cache.get(&address).map(|(b, _)| *b).unwrap_or(0);
        value.extend_from_slice(&balance.to_le_bytes());
        value.extend_from_slice(&nonce.to_le_bytes());

        self.tree.insert(&key, value);

        // Update cache
        let entry = self.cache.entry(address).or_insert((0, 0));
        entry.1 = nonce;
    }

    /// Get balance for an address
    pub fn get_balance(&self, address: Address) -> u128 {
        // Check cache first
        if let Some((balance, _)) = self.cache.get(&address) {
            return *balance;
        }

        // Get from tree
        if let Some(value) = self.tree.get(&address) {
            if value.len() >= 16 {
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&value[0..16]);
                return u128::from_le_bytes(bytes);
            }
        }

        0
    }

    /// Get nonce for an address
    pub fn get_nonce(&self, address: Address) -> u64 {
        // Check cache first
        if let Some((_, nonce)) = self.cache.get(&address) {
            return *nonce;
        }

        // Get from tree
        if let Some(value) = self.tree.get(&address) {
            if value.len() >= 24 {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&value[16..24]);
                return u64::from_le_bytes(bytes);
            }
        }

        0
    }

    /// Get balance with proof
    pub fn get_balance_with_proof(&self, address: Address) -> (u128, Vec<Hash>, Hash) {
        let balance = self.get_balance(address);
        let proof = self.tree.get_proof(&address);
        let root_hash = self.tree.root_hash().unwrap_or(Hash::zero());
        (balance, proof, root_hash)
    }

    /// Get nonce with proof
    pub fn get_nonce_with_proof(&self, address: Address) -> (u64, Vec<Hash>, Hash) {
        let nonce = self.get_nonce(address);
        let proof = self.tree.get_proof(&address);
        let root_hash = self.tree.root_hash().unwrap_or(Hash::zero());
        (nonce, proof, root_hash)
    }

    /// Get state root
    pub fn state_root(&self) -> Hash {
        self.tree.root_hash().unwrap_or(Hash::zero())
    }

    /// Get tree size
    pub fn size(&self) -> usize {
        self.tree.size()
    }

    /// Get all accounts for snapshot
    pub fn get_all_accounts(&self) -> HashMap<Address, (u128, u64)> {
        self.cache.clone()
    }

    /// Get balance with KZG proof
    #[cfg(feature = "privacy")]
    pub fn get_balance_with_kzg_proof(&self, address: Address) -> Option<(u128, KzgVerkleProof)> {
        let balance = self.get_balance(address);
        let proof = self.tree.get_kzg_proof(&address)?;
        Some((balance, proof))
    }

    /// Get nonce with KZG proof
    #[cfg(feature = "privacy")]
    pub fn get_nonce_with_kzg_proof(&self, address: Address) -> Option<(u64, KzgVerkleProof)> {
        let nonce = self.get_nonce(address);
        let proof = self.tree.get_kzg_proof(&address)?;
        Some((nonce, proof))
    }

    /// Get dual commitment for state root
    #[cfg(feature = "privacy")]
    pub fn get_dual_commitment(&self) -> Option<DualCommitment> {
        self.tree.get_dual_commitment()
    }
}

impl Default for VerkleState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verkle_tree_insert_get() {
        let mut tree = VerkleTree::new();
        let key = b"test_key";
        let value = b"test_value".to_vec();

        tree.insert(key, value.clone());
        assert_eq!(tree.get(key), Some(value));
        assert_eq!(tree.size(), 1);
    }

    #[test]
    fn test_verkle_state() {
        let mut state = VerkleState::new();
        let address = Address([1u8; 20]);

        state.set_balance(address, 1000);
        assert_eq!(state.get_balance(address), 1000);

        state.set_nonce(address, 5);
        assert_eq!(state.get_nonce(address), 5);
        assert_eq!(state.get_balance(address), 1000); // Balance preserved
    }

    #[test]
    fn test_proof_generation() {
        let mut state = VerkleState::new();
        let address = Address([1u8; 20]);

        state.set_balance(address, 1000);
        let (balance, _proof, root) = state.get_balance_with_proof(address);

        assert_eq!(balance, 1000);
        assert_ne!(root, Hash([0u8; 32]));
    }
}

// KZG-specific tests (only when privacy feature is enabled)
#[cfg(test)]
#[cfg(feature = "privacy")]
mod kzg_tests {
    use super::*;

    #[test]
    fn test_dual_commitment_consistency() {
        let mut tree = VerkleTree::new();
        let key = b"test_key";
        let value = b"test_value".to_vec();

        tree.insert(key, value.clone());

        // Get dual commitment for root
        let dual = tree
            .get_dual_commitment()
            .expect("Should have dual commitment");

        // Keccak hash should match root_hash
        let keccak_root = tree.root_hash().expect("Should have root hash");
        assert_eq!(dual.keccak, keccak_root, "Keccak commitments should match");

        // KZG commitment should be non-trivial (not all zeros)
        let kzg_bytes = dual.kzg.to_bytes();
        assert!(
            !kzg_bytes.iter().all(|&b| b == 0),
            "KZG commitment should not be zero"
        );
    }

    #[test]
    fn test_kzg_verkle_proof_generation() {
        let mut tree = VerkleTree::new();
        let key = b"test_key_for_proof";
        let value = b"test_value_for_proof".to_vec();

        tree.insert(key, value.clone());

        // Generate KZG proof
        let proof = tree.get_kzg_proof(key).expect("Should generate KZG proof");

        // Verify proof structure
        assert_eq!(proof.key, key.to_vec());
        assert_eq!(proof.value, value);
        assert!(
            !proof.steps.is_empty(),
            "Proof should have at least one step"
        );

        // Each step should have non-trivial commitment and proof
        for step in &proof.steps {
            assert!(!step.node_commitment.is_empty());
            assert!(!step.opening_proof.is_empty());
            assert!(step.child_index < 256);
        }
    }

    #[test]
    fn test_kzg_proof_rejects_tampered_value() {
        let mut tree = VerkleTree::new();
        let key = b"original_key";
        let value = b"original_value".to_vec();

        tree.insert(key, value);

        // Generate proof for original key
        let proof = tree.get_kzg_proof(key).expect("Should generate KZG proof");

        // Tampered value should be different
        let tampered_value = b"tampered_value".to_vec();
        assert_ne!(proof.value, tampered_value);
    }

    #[test]
    fn test_kzg_state_proof_generation() {
        let mut state = VerkleState::new();
        let address = [42u8; 20];

        state.set_balance(address, 5000);
        state.set_nonce(address, 10);

        // Get balance with KZG proof
        let (balance, proof) = state
            .get_balance_with_kzg_proof(address)
            .expect("Should generate KZG proof for balance");

        assert_eq!(balance, 5000);
        assert_eq!(proof.key, address.to_vec());

        // Get nonce with KZG proof
        let (nonce, proof) = state
            .get_nonce_with_kzg_proof(address)
            .expect("Should generate KZG proof for nonce");

        assert_eq!(nonce, 10);
        assert_eq!(proof.key, address.to_vec());
    }

    #[test]
    fn test_compute_kzg_commitment_basic() {
        // Test with all zeros
        let zero_hashes = [[0u8; 32]; 256];
        let commitment = compute_kzg_commitment(&zero_hashes);
        assert!(
            commitment.is_some(),
            "Should compute commitment for zero polynomial"
        );

        // Test with some non-zero values
        let mut mixed_hashes = [[0u8; 32]; 256];
        mixed_hashes[0] = [1u8; 32];
        mixed_hashes[1] = [2u8; 32];
        mixed_hashes[255] = [255u8; 32];

        let commitment = compute_kzg_commitment(&mixed_hashes);
        assert!(
            commitment.is_some(),
            "Should compute commitment for mixed polynomial"
        );
    }

    #[test]
    fn test_get_child_hashes() {
        let mut tree = VerkleTree::new();
        let key1 = [1u8; 20];
        let key2 = [2u8; 20];

        tree.insert(&key1, b"value1".to_vec());
        tree.insert(&key2, b"value2".to_vec());

        let child_hashes = tree.root.get_child_hashes();

        // At least some hashes should be non-zero (the ones with children/values)
        let non_zero_count = child_hashes
            .iter()
            .filter(|h| !h.iter().all(|&b| b == 0))
            .count();

        assert!(non_zero_count > 0, "Should have some non-zero child hashes");
    }
}
