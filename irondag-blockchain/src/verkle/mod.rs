//! Verkle Tree Implementation for Stateless Mode
//!
//! This module provides Verkle tree-backed state management with proof generation
//! for light client verification, based on Ethereum's Verkle tree research.
//!
//! # Architecture Overview
//!
//! Verkle trees enable **efficient state proofs for light client verification** by
//! replacing Merkle-Patricia trees' logarithmic proof sizes with constant-size proofs
//! using polynomial commitments. This is critical for IronDAG's stateless client
//! architecture where nodes can verify state transitions without storing the full
//! blockchain state.
//!
//! ## Dual Commitment Pattern
//!
//! The module implements a **dual commitment pattern** that bridges backward
//! compatibility with zero-knowledge-friendly proofs:
//!
//! - **Keccak Commitment**: Preserves compatibility with existing Merkle-style
//!   verification. Used for standard light client proofs and backward-compatible
//!   state verification.
//! - **KZG Polynomial Commitment**: Enables O(1) proof size regardless of state
//!   tree depth. Critical for ZK circuits where proof verification must be
//!   computationally tractable within the constraint system.
//!
//! This dual approach allows IronDAG to support both traditional verification
//! (via Keccak-based proofs) and advanced ZK proving (via KZG commitments) from
//! the same Verkle tree structure.
//!
//! ## Stream C Integration
//!
//! Stream C (ZK proof mining) uses **Verkle state roots as public inputs** for
//! zero-knowledge proofs. The integration follows these principles:
//!
//! 1. Pre-state and post-state roots are wired into the ZK circuit as public inputs
//! 2. Balance proofs authenticate sender/receiver state via Verkle proofs
//! 3. In-circuit Verkle proof verification uses MiMC-based hash gadgets
//! 4. KZG-based Verkle proofs provide succinct (32-byte) commitments
//!
//! ## Key Components
//!
//! - [`VerkleTree`] / [`VerkleState`]: Core tree structure and state management
//! - [`KzgCommitment`] / [`KzgProof`]: KZG polynomial commitment primitives
//! - [`StateProof`] / [`ProofVerifier`]: Proof generation and verification
//! - [`DualCommitment`]: Keccak + KZG dual commitment (privacy feature)
//! - [`KzgVerkleProof`]: KZG-based Verkle proof structure
//!
//! ## Proof Efficiency
//!
//! Traditional Merkle proofs scale as O(log n) where n is tree depth, typically
//! requiring hundreds of hashes for a single proof. KZG polynomial commitments
//! enable **O(1) proof size** — a single 48-byte commitment regardless of tree depth.
//! This makes state proofs practical for:
//!
//! - Light client verification without full state sync
//! - Cross-shard state proofs in future sharding scenarios
//! - ZK circuit integration where constraint count is at a premium
//!
//! # Feature Flags
//!
//! - `privacy`: Enables `DualCommitment` and KZG-specific encoding functions
//!   for zero-knowledge proof integration

pub mod kzg;
pub mod proof;
pub mod tree;

pub use kzg::{KzgCommitment, KzgProof};
pub use proof::{ProofVerifier, StateProof};
pub use tree::{VerkleState, VerkleTree};

// Export KZG Verkle proof types (available in both privacy and non-privacy modes)
pub use tree::{KzgVerkleProof, KzgVerkleProofStep};

#[cfg(feature = "privacy")]
pub use tree::DualCommitment;
#[cfg(feature = "privacy")]
pub use tree::{compute_kzg_commitment, field_element_to_bytes, hash_to_field_element};
