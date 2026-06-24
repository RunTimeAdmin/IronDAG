//! Account Abstraction - Smart Contract Wallets as First-Class Accounts

pub mod batch;
pub mod factory;
pub mod multisig;
pub mod registry;
pub mod social_recovery;
pub mod wallet;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod integration_tests_phase5;

#[cfg(test)]
mod multisig_integration_tests;

pub use batch::{
    BatchManager, BatchOperation, BatchOperationResult, BatchStatus, BatchTransaction, GasEstimate,
};
pub use factory::WalletFactory;
pub use multisig::{
    MultiSigManager, MultiSigSignature, MultiSigTransaction, MultiSigValidationResult,
};
pub use registry::WalletRegistry;
pub use social_recovery::{RecoveryRequest, RecoveryStatus, SocialRecoveryManager};
pub use wallet::{
    AuthMethod, RecoveryConfig, SmartContractWallet, SpendingLimits, WalletConfig, WalletType,
};
