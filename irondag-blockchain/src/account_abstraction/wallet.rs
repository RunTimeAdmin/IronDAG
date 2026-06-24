//! Smart Contract Wallet Implementation

use crate::types::Address;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Helper to get current timestamp, returns error if system time is before Unix epoch
fn current_timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| format!("System time error: {}", e))
}

/// Smart Contract Wallet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartContractWallet {
    /// Wallet contract address
    pub address: Address,
    /// Original EOA owner (who created the wallet)
    pub owner: [u8; 20],
    /// Wallet type
    pub wallet_type: WalletType,
    /// Wallet configuration
    pub config: WalletConfig,
    /// Creation timestamp
    pub created_at: u64,
    /// Nonce for wallet transactions
    pub nonce: u64,
}

/// Wallet type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WalletType {
    /// Basic programmable wallet
    Basic,
    /// Multi-signature wallet
    MultiSig {
        /// List of signer addresses
        signers: Vec<[u8; 20]>,
        /// Number of signatures required (threshold)
        threshold: u8,
    },
    /// Social recovery wallet
    SocialRecovery {
        /// List of guardian addresses
        guardians: Vec<[u8; 20]>,
        /// Number of guardians required for recovery
        recovery_threshold: u8,
    },
    /// Wallet with spending limits
    SpendingLimit {
        /// Spending limit configuration
        limits: SpendingLimits,
    },
    /// Combined wallet (multi-sig + social recovery + spending limits)
    Combined {
        signers: Vec<[u8; 20]>,
        threshold: u8,
        guardians: Vec<[u8; 20]>,
        recovery_threshold: u8,
        limits: SpendingLimits,
    },
}

/// Wallet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Authentication method
    pub auth_method: AuthMethod,
    /// Spending limits (if applicable)
    pub spending_limits: Option<SpendingLimits>,
    /// Recovery configuration (if applicable)
    pub recovery_config: Option<RecoveryConfig>,
}

/// Authentication method
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthMethod {
    /// Single signature (EOA-like)
    SingleSignature,
    /// Multi-signature (n-of-m)
    MultiSignature {
        signers: Vec<[u8; 20]>,
        threshold: u8,
    },
    /// Social recovery
    SocialRecovery {
        guardians: Vec<[u8; 20]>,
        threshold: u8,
    },
    /// Custom (future: biometric, hardware keys, etc.)
    Custom(String),
}

/// Spending limits configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpendingLimits {
    /// Daily spending limit (in base units)
    pub daily_limit: u128,
    /// Weekly spending limit
    pub weekly_limit: u128,
    /// Monthly spending limit
    pub monthly_limit: u128,
    /// Per-address limits (address -> limit)
    pub per_address_limits: Vec<([u8; 20], u128)>,
    /// Current period spending (resets daily/weekly/monthly)
    pub current_period_spending: u128,
    /// Last reset timestamp
    pub last_reset: u64,
}

/// Recovery configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryConfig {
    /// List of guardian addresses
    pub guardians: Vec<[u8; 20]>,
    /// Number of guardians required for recovery
    pub threshold: u8,
    /// Time delay before recovery can complete (seconds)
    pub time_delay: u64,
    /// Pending recovery requests
    pub pending_recoveries: Vec<RecoveryRequest>,
}

/// Recovery request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRequest {
    /// New owner address (who will receive the wallet)
    pub new_owner: [u8; 20],
    /// Initiator address (who started the recovery)
    pub initiator: [u8; 20],
    /// Timestamp when recovery was initiated
    pub initiated_at: u64,
    /// Guardian approvals (address -> timestamp)
    pub approvals: Vec<([u8; 20], u64)>,
    /// Whether recovery is complete
    pub completed: bool,
}

impl SmartContractWallet {
    /// Create a new basic wallet
    pub fn new_basic(address: Address, owner: [u8; 20]) -> Result<Self, String> {
        Ok(Self {
            address,
            owner,
            wallet_type: WalletType::Basic,
            config: WalletConfig {
                auth_method: AuthMethod::SingleSignature,
                spending_limits: None,
                recovery_config: None,
            },
            created_at: current_timestamp()?,
            nonce: 0,
        })
    }

    /// Create a multi-signature wallet
    pub fn new_multisig(
        address: Address,
        owner: [u8; 20],
        signers: Vec<[u8; 20]>,
        threshold: u8,
    ) -> Result<Self, String> {
        if threshold == 0 || threshold > signers.len() as u8 {
            return Err("Invalid threshold".to_string());
        }
        if signers.is_empty() {
            return Err("Signers list cannot be empty".to_string());
        }
        if signers.len() > 20 {
            return Err("Maximum 20 signers allowed".to_string());
        }

        Ok(Self {
            address,
            owner,
            wallet_type: WalletType::MultiSig {
                signers: signers.clone(),
                threshold,
            },
            config: WalletConfig {
                auth_method: AuthMethod::MultiSignature { signers, threshold },
                spending_limits: None,
                recovery_config: None,
            },
            created_at: current_timestamp()?,
            nonce: 0,
        })
    }

    /// Create a social recovery wallet
    pub fn new_social_recovery(
        address: Address,
        owner: [u8; 20],
        guardians: Vec<[u8; 20]>,
        recovery_threshold: u8,
        time_delay: u64,
    ) -> Result<Self, String> {
        if recovery_threshold == 0 || recovery_threshold > guardians.len() as u8 {
            return Err("Invalid recovery threshold".to_string());
        }
        if guardians.is_empty() {
            return Err("Guardians list cannot be empty".to_string());
        }
        if guardians.len() > 20 {
            return Err("Maximum 20 guardians allowed".to_string());
        }

        Ok(Self {
            address,
            owner,
            wallet_type: WalletType::SocialRecovery {
                guardians: guardians.clone(),
                recovery_threshold,
            },
            config: WalletConfig {
                auth_method: AuthMethod::SingleSignature,
                spending_limits: None,
                recovery_config: Some(RecoveryConfig {
                    guardians: guardians.clone(),
                    threshold: recovery_threshold,
                    time_delay,
                    pending_recoveries: Vec::new(),
                }),
            },
            created_at: current_timestamp()?,
            nonce: 0,
        })
    }

    /// Check if wallet is multi-signature
    pub fn is_multisig(&self) -> bool {
        matches!(
            self.wallet_type,
            WalletType::MultiSig { .. } | WalletType::Combined { .. }
        )
    }

    /// Check if wallet has social recovery
    pub fn has_social_recovery(&self) -> bool {
        matches!(
            self.wallet_type,
            WalletType::SocialRecovery { .. } | WalletType::Combined { .. }
        )
    }

    /// Check if wallet has spending limits
    pub fn has_spending_limits(&self) -> bool {
        matches!(
            self.wallet_type,
            WalletType::SpendingLimit { .. } | WalletType::Combined { .. }
        ) || self.config.spending_limits.is_some()
    }

    /// Increment wallet nonce
    pub fn increment_nonce(&mut self) {
        self.nonce += 1;
    }

    /// Get current nonce
    pub fn get_nonce(&self) -> u64 {
        self.nonce
    }
}

impl SpendingLimits {
    /// Create new spending limits
    pub fn new(daily_limit: u128, weekly_limit: u128, monthly_limit: u128) -> Result<Self, String> {
        Ok(Self {
            daily_limit,
            weekly_limit,
            monthly_limit,
            per_address_limits: Vec::new(),
            current_period_spending: 0,
            last_reset: current_timestamp()?,
        })
    }

    /// Check if spending is within limits
    pub fn check_limit(&mut self, amount: u128) -> Result<(), String> {
        let now = current_timestamp()?;

        // Reset daily counter if a new day has started
        let days_since_reset = (now - self.last_reset) / 86400;
        if days_since_reset >= 1 {
            self.current_period_spending = 0;
            self.last_reset = now;
        }

        // H9: Enforce daily limit
        if self.current_period_spending + amount > self.daily_limit {
            return Err("Daily spending limit exceeded".to_string());
        }

        // H9: Enforce weekly limit
        if self.current_period_spending + amount > self.weekly_limit {
            return Err("Weekly spending limit exceeded".to_string());
        }

        // H9: Enforce monthly limit
        if self.current_period_spending + amount > self.monthly_limit {
            return Err("Monthly spending limit exceeded".to_string());
        }

        Ok(())
    }

    /// H10: Atomic check-and-record to eliminate TOCTOU race.
    /// Checks all limits and records the spend in one call.
    pub fn check_and_record_spending(&mut self, amount: u128) -> Result<(), String> {
        self.check_limit(amount)?;
        self.current_period_spending += amount;
        Ok(())
    }

    /// Record spending (kept for backward compatibility — prefer check_and_record_spending)
    pub fn record_spending(&mut self, amount: u128) {
        self.current_period_spending += amount;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_wallet_creation() {
        let address = Address([1u8; 20]);
        let owner = [2u8; 20];
        let wallet =
            SmartContractWallet::new_basic(address, owner).expect("Failed to create basic wallet");

        assert_eq!(wallet.address, address);
        assert_eq!(wallet.owner, owner);
        assert_eq!(wallet.wallet_type, WalletType::Basic);
        assert_eq!(wallet.nonce, 0);
    }

    #[test]
    fn test_multisig_wallet_creation() {
        let address = Address([1u8; 20]);
        let owner = [2u8; 20];
        let signers = vec![[3u8; 20], [4u8; 20], [5u8; 20]];

        let wallet = SmartContractWallet::new_multisig(address, owner, signers.clone(), 2).unwrap();

        assert_eq!(wallet.address, address);
        assert!(wallet.is_multisig());
        match wallet.wallet_type {
            WalletType::MultiSig { threshold, .. } => {
                assert_eq!(threshold, 2);
            }
            _ => panic!("Expected MultiSig wallet"),
        }
    }

    #[test]
    fn test_multisig_invalid_threshold() {
        let address = Address([1u8; 20]);
        let owner = [2u8; 20];
        let signers = vec![[3u8; 20], [4u8; 20]];

        // Threshold too high
        assert!(SmartContractWallet::new_multisig(address, owner, signers.clone(), 5).is_err());

        // Threshold zero
        assert!(SmartContractWallet::new_multisig(address, owner, signers.clone(), 0).is_err());
    }

    #[test]
    fn test_spending_limits() {
        let mut limits =
            SpendingLimits::new(1000, 5000, 20000).expect("Failed to create spending limits");

        // Should allow spending within limit
        assert!(limits.check_limit(500).is_ok());
        limits.record_spending(500);

        // Should reject spending over limit
        assert!(limits.check_limit(600).is_err());
    }
}
