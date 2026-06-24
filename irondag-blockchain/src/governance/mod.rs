//! Governance and Node Longevity System
//!
//! Implements node identity, hardware fingerprinting, and longevity tracking
//! for governance voting in Algorithm Rotation proposals.

pub mod longevity;
pub mod node_identity;
pub mod registry;

#[cfg(test)]
mod tests;

pub use longevity::{ActivitySnapshot, NodeLongevity, ParticipationType};
pub use node_identity::{HardwareFingerprint, NodeIdentity, ZkUniquenessProof};
pub use registry::{LongevityTracker, NodeRegistry};
