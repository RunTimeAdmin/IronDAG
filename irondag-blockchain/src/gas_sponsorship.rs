//! Programmable Gas Sponsorship
//!
//! Sponsors register a policy that governs which senders they'll cover gas fees for,
//! how much per transaction, and for how long. At validation time the node enforces
//! the policy; at processing time the sponsor's balance is debited and the spend
//! window is updated.

use crate::types::Address;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// ~1 day worth of blocks at the Stream B target (5s/block × 17,280 = 24 h).
const BLOCKS_PER_DAY: u64 = 17_280;

/// Rules a sponsor publishes to define when it will cover gas fees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsorPolicy {
    /// When false, the policy is paused and no new sponsorships are approved.
    pub active: bool,
    /// If Some, only these sender addresses are eligible. None = any sender.
    pub allowed_senders: Option<Vec<Address>>,
    /// Ceiling on the fee this sponsor covers per transaction (attoIDAG).
    /// None = no per-tx ceiling.
    pub max_fee_per_tx: Option<u128>,
    /// Block number after which the policy expires. None = never expires.
    pub expires_at_block: Option<u64>,
    /// Maximum cumulative fee covered within a rolling daily window (attoIDAG).
    /// None = no daily ceiling.
    pub daily_spend_limit: Option<u128>,
    // Tracking fields — updated by record_spend; included in serde so they survive
    // a future persistence layer, but not exposed in the "clean" JSON view.
    #[serde(default)]
    pub window_start_block: u64,
    #[serde(default)]
    pub spent_in_window: u128,
}

impl SponsorPolicy {
    /// True if the policy has passed its expiry block.
    pub fn is_expired(&self, current_block: u64) -> bool {
        self.expires_at_block.is_some_and(|exp| current_block > exp)
    }

    /// True if `sender` is permitted by the allowlist (or there is no allowlist).
    pub fn allows_sender(&self, sender: &Address) -> bool {
        match &self.allowed_senders {
            None => true,
            Some(list) => list.contains(sender),
        }
    }

    /// True if adding `fee` to the current window's spend stays within the daily limit.
    pub fn within_daily_limit(&self, fee: u128, current_block: u64) -> bool {
        let Some(limit) = self.daily_spend_limit else {
            return true;
        };
        let in_window = current_block.saturating_sub(self.window_start_block) < BLOCKS_PER_DAY;
        let spent = if in_window { self.spent_in_window } else { 0 };
        spent.saturating_add(fee) <= limit
    }

    /// Advance the spend window if needed, then add `fee` to the accumulator.
    pub fn record_spend(&mut self, fee: u128, current_block: u64) {
        if current_block.saturating_sub(self.window_start_block) >= BLOCKS_PER_DAY {
            self.window_start_block = current_block;
            self.spent_in_window = fee;
        } else {
            self.spent_in_window = self.spent_in_window.saturating_add(fee);
        }
    }
}

/// Thread-safe registry of sponsor policies.
///
/// Backed by DashMap so reads and writes are concurrent; no external lock needed.
/// Shared as `Arc<SponsorRegistry>` between `Blockchain` (validation + spend tracking)
/// and `RpcServer` (registration / query).
#[derive(Default)]
pub struct SponsorRegistry {
    policies: DashMap<Address, SponsorPolicy>,
}

impl SponsorRegistry {
    pub fn new() -> Self {
        Self {
            policies: DashMap::new(),
        }
    }

    /// Register or replace the policy for `sponsor`.
    pub fn register(&self, sponsor: Address, policy: SponsorPolicy) {
        self.policies.insert(sponsor, policy);
    }

    /// Remove the policy for `sponsor`. Returns true if one existed.
    pub fn deregister(&self, sponsor: &Address) -> bool {
        self.policies.remove(sponsor).is_some()
    }

    /// Return a snapshot of the policy for `sponsor`, if one exists.
    pub fn get(&self, sponsor: &Address) -> Option<SponsorPolicy> {
        self.policies.get(sponsor).map(|p| p.clone())
    }

    /// Validate a sponsorship request against the registered policy.
    ///
    /// Returns `Ok(())` if the policy permits the sponsorship, otherwise an
    /// error string describing the violation.
    pub fn check(
        &self,
        sponsor: &Address,
        sender: &Address,
        fee: u128,
        current_block: u64,
    ) -> Result<(), String> {
        let policy = self.policies.get(sponsor).ok_or_else(|| {
            "Sponsor has no registered policy — use irondag_registerSponsorPolicy first".to_string()
        })?;

        if !policy.active {
            return Err("Sponsor policy is paused".to_string());
        }
        if policy.is_expired(current_block) {
            return Err(format!(
                "Sponsor policy expired at block {}",
                policy.expires_at_block.unwrap()
            ));
        }
        if !policy.allows_sender(sender) {
            return Err("Sender address is not in the sponsor's allowlist".to_string());
        }
        if let Some(max) = policy.max_fee_per_tx {
            if fee > max {
                return Err(format!(
                    "Fee {} exceeds sponsor max_fee_per_tx {}",
                    fee, max
                ));
            }
        }
        if !policy.within_daily_limit(fee, current_block) {
            return Err("Sponsor daily spend limit would be exceeded".to_string());
        }

        Ok(())
    }

    /// Record a confirmed sponsorship spend after a block is committed.
    pub fn record_spend(&self, sponsor: &Address, fee: u128, current_block: u64) {
        if let Some(mut policy) = self.policies.get_mut(sponsor) {
            policy.record_spend(fee, current_block);
        }
    }

    /// Return all registered sponsors and their policies (for introspection).
    pub fn list_all(&self) -> Vec<(Address, SponsorPolicy)> {
        self.policies
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }
}
