//! Governance and Node Longevity System
//!
//! Implements node identity, hardware fingerprinting, longevity tracking,
//! and governance-scheduled protocol upgrades (Algorithm Rotation).

pub mod longevity;
pub mod node_identity;
pub mod registry;
pub mod scheduler;

#[cfg(test)]
mod tests;

pub use longevity::{ActivitySnapshot, NodeLongevity, ParticipationType};
pub use node_identity::{HardwareFingerprint, NodeIdentity, ZkUniquenessProof};
pub use registry::{LongevityTracker, NodeRegistry};
pub use scheduler::{GovernanceAction, GovernanceScheduler, ScheduledAction};
