//! Governance-scheduled protocol upgrades
//!
//! Governance actions activate at a specific block height. All nodes apply
//! them at the same height, so there is no per-node divergence in what
//! algorithm is "active" at any given point in the chain.
//!
//! Currently supported actions:
//!   - `SetHashAlgorithm(u8)` — changes the `hash_version` byte that miners
//!     must commit into every new block header's PoW hash.

use crate::blockchain::block::HASH_VERSION_BLAKE3;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// A protocol upgrade that takes effect at a specific block height.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAction {
    /// Block height at which this action becomes active (inclusive).
    pub activation_height: u64,
    pub action: GovernanceAction,
}

/// The set of protocol parameters that can be changed via governance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GovernanceAction {
    /// Switch the PoW hash algorithm. The `u8` is the new `hash_version` value
    /// committed into every BlockHeader from `activation_height` onwards.
    SetHashAlgorithm(u8),
}

/// Tracks scheduled governance actions and the currently active protocol parameters.
///
/// Lives inside `Blockchain` behind an `Arc<RwLock<_>>`. `apply_at_height` is
/// called once per committed block and is the only mutation site.
#[derive(Debug)]
pub struct GovernanceScheduler {
    /// Actions not yet activated, sorted ascending by activation_height.
    pending: Vec<ScheduledAction>,
    /// The hash algorithm version currently in effect.
    active_hash_version: u8,
}

impl Default for GovernanceScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl GovernanceScheduler {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            active_hash_version: HASH_VERSION_BLAKE3,
        }
    }

    /// The `hash_version` byte miners must include in new block headers.
    pub fn active_hash_version(&self) -> u8 {
        self.active_hash_version
    }

    /// Schedule an action for a future block height.
    ///
    /// Returns `Err` if `activation_height` is 0 or if an action of the same
    /// kind is already scheduled for that height (deduplication guard).
    pub fn schedule(&mut self, action: ScheduledAction) -> Result<(), String> {
        if action.activation_height == 0 {
            return Err("activation_height must be > 0".to_string());
        }
        // Prevent duplicate: same variant at same height
        let duplicate = self.pending.iter().any(|s| {
            s.activation_height == action.activation_height
                && std::mem::discriminant(&s.action) == std::mem::discriminant(&action.action)
        });
        if duplicate {
            return Err(format!(
                "A {:?} action is already scheduled at height {}",
                action.action, action.activation_height
            ));
        }

        self.pending.push(action);
        // Keep sorted so we can drain the front cheaply
        self.pending.sort_unstable_by_key(|s| s.activation_height);
        Ok(())
    }

    /// Called once for every committed block. Activates any actions whose
    /// `activation_height` ≤ `height` and removes them from the pending list.
    pub fn apply_at_height(&mut self, height: u64) {
        // Drain all actions that should have fired by now
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].activation_height <= height {
                let scheduled = self.pending.remove(i);
                self.apply(scheduled);
            } else {
                i += 1;
            }
        }
    }

    fn apply(&mut self, scheduled: ScheduledAction) {
        match scheduled.action {
            GovernanceAction::SetHashAlgorithm(version) => {
                if version == 0 {
                    warn!(
                        height = scheduled.activation_height,
                        "Ignoring SetHashAlgorithm(0) — 0x00 is reserved"
                    );
                    return;
                }
                info!(
                    height = scheduled.activation_height,
                    old = self.active_hash_version,
                    new = version,
                    "GovernanceAction::SetHashAlgorithm applied"
                );
                self.active_hash_version = version;
            }
        }
    }

    /// Pending actions (read-only, for RPC / status reporting).
    pub fn pending_actions(&self) -> &[ScheduledAction] {
        &self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_hash_version_is_blake3() {
        let scheduler = GovernanceScheduler::new();
        assert_eq!(scheduler.active_hash_version(), HASH_VERSION_BLAKE3);
    }

    #[test]
    fn test_action_not_applied_before_height() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 100,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        })
        .unwrap();
        s.apply_at_height(99);
        assert_eq!(s.active_hash_version(), HASH_VERSION_BLAKE3);
        assert_eq!(s.pending_actions().len(), 1);
    }

    #[test]
    fn test_action_applied_at_exact_height() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 100,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        })
        .unwrap();
        s.apply_at_height(100);
        assert_eq!(s.active_hash_version(), 0x02);
        assert_eq!(s.pending_actions().len(), 0);
    }

    #[test]
    fn test_action_applied_after_height() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 50,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        })
        .unwrap();
        // Block 200 catches up — action still fires
        s.apply_at_height(200);
        assert_eq!(s.active_hash_version(), 0x02);
    }

    #[test]
    fn test_duplicate_action_same_height_rejected() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 100,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        })
        .unwrap();
        let result = s.schedule(ScheduledAction {
            activation_height: 100,
            action: GovernanceAction::SetHashAlgorithm(0x03),
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_activation_height_rejected() {
        let mut s = GovernanceScheduler::new();
        let result = s.schedule(ScheduledAction {
            activation_height: 0,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_version_ignored() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 10,
            action: GovernanceAction::SetHashAlgorithm(0x00),
        })
        .unwrap();
        s.apply_at_height(10);
        // Should remain BLAKE3 — 0x00 is reserved
        assert_eq!(s.active_hash_version(), HASH_VERSION_BLAKE3);
    }

    #[test]
    fn test_sequential_upgrades() {
        let mut s = GovernanceScheduler::new();
        s.schedule(ScheduledAction {
            activation_height: 100,
            action: GovernanceAction::SetHashAlgorithm(0x02),
        })
        .unwrap();
        s.schedule(ScheduledAction {
            activation_height: 200,
            action: GovernanceAction::SetHashAlgorithm(0x03),
        })
        .unwrap();
        s.apply_at_height(100);
        assert_eq!(s.active_hash_version(), 0x02);
        s.apply_at_height(200);
        assert_eq!(s.active_hash_version(), 0x03);
    }
}
