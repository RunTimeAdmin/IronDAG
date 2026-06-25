//! Blockchain core implementation
//!
//! Copyright (c) 2024-2025 IronDAG Contributors
//! Licensed under the BUSL-1.1 License (see LICENSE file)

pub mod block;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_proptest;
#[cfg(test)]
mod tests_quick_wins;
// #[cfg(test)]
// mod tests_timestamp_validation; // TODO: Fix imports
pub use block::{Block, BlockHeader, EcdsaSignature, PublicKey, Transaction, TransactionSignature};

/// Maximum serialized block size (4MB). This is a fixed protocol constant —
/// all nodes must enforce the same limit for consensus compatibility.
pub const MAX_BLOCK_SIZE: usize = 4 * 1024 * 1024;

/// Maximum number of parent hashes in a block (DoS protection)
pub const MAX_PARENT_HASHES: usize = 10;

/// Maximum transaction data size in bytes (128KB)
pub const MAX_TX_DATA_SIZE: usize = 128 * 1024;

/// Maximum number of transactions per block (DoS protection)
/// This is per-stream maximum; BraidCore architecture has 3 parallel streams
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 10_000;

/// Maximum block timestamp drift allowed (15 seconds into the future)
/// Prevents timestamp manipulation attacks while allowing minor clock skew
pub const MAX_TIMESTAMP_DRIFT: u64 = 15;

/// Minimum acceptable block timestamp (January 1, 2020 00:00:00 UTC)
/// Prevents obviously invalid timestamps from early blockchain era
pub const MIN_TIMESTAMP: u64 = 1577836800;

/// Maximum parent block age (in blocks) for parent hash validation.
/// Prevents referencing truly ancient blocks as parents in GhostDAG.
/// Set high enough that snapshot-checkpoint blocks (installed during pruned-peer
/// sync catchup) remain valid parents until they fall out of the DAG naturally.
pub const MAX_PARENT_AGE: u64 = 100_000;

use crate::consensus::GhostDAG;
use crate::error::BlockValidationError;
use crate::storage::{make_key, Database};
use crate::types::{Address, StreamType, DEFAULT_CHAIN_ID};
use dashmap::DashMap; // Lock-free concurrent map
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock; // For fine-grained locking
use tracing::{debug, error, info, warn};

/// Chain ID for EIP-155 replay protection (default: 1338)
/// Uses the DEFAULT_CHAIN_ID from types.rs as the fallback value
#[allow(dead_code)]
const DEFAULT_CHAIN_ID_LOCAL: u64 = DEFAULT_CHAIN_ID;

/// Maximum nonce gap allowed for mempool acceptance
const MAX_NONCE_GAP: u64 = 16;

/// Maximum transaction value to prevent overflow in balance arithmetic
const MAX_TX_VALUE: u128 = u128::MAX / 2;

/// Maximum gas limit per transaction (30M gas)
const MAX_GAS_LIMIT: u64 = 30_000_000;

/// Finality depth for spent outputs pruning (blocks older than this are considered final)
const FINALITY_DEPTH: u64 = 100;

/// Persisted receipt for a processed transaction.
/// Stored in sled under RECEIPT key prefix, indexed by tx hash.
/// Covers both EVM contract calls and plain native transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxReceipt {
    pub success: bool,
    pub gas_used: u64,
    pub logs: Vec<crate::evm::EvmLog>,
    /// Set for contract deployments (tx.to is zero address).
    pub contract_address: Option<[u8; 20]>,
}

/// ARC-003/ARC-004: Global block processing lock to prevent TOCTOU race condition.
/// This lock serializes the entire block processing pipeline (validate → process → persist → commit)
/// to ensure that concurrent readers cannot see partially committed state.
/// Uses tokio::sync::Mutex so the guard is Send-safe across .await points in async spawned tasks.
static BLOCK_PROCESSING_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Main blockchain structure with fine-grained locking for high concurrency
///
/// ARCHITECTURE NOTES:
/// - `blocks` and `metadata` use RwLock for coordinated updates
/// - `accounts` (balances/nonces) use DashMap for lock-free concurrent reads
/// - This eliminates the single-lock bottleneck that caused RPC timeouts
pub struct Blockchain {
    // Storage (optional - None means in-memory only)
    database: Option<Arc<Database>>,

    // GhostDAG consensus engine (shared with MiningManager for parent selection)
    ghostdag: Option<Arc<RwLock<GhostDAG>>>,

    // FINE-GRAINED LOCKS: Separate locks for independent data
    // Blocks and block hashes - locked together for consistency
    blocks: Arc<RwLock<BlocksData>>,

    // Accounts (balances + nonces) - lock-free concurrent map
    // DashMap allows parallel reads without any locking!
    accounts: Arc<DashMap<Address, AccountState>>,

    // Lock-free cached latest block number (updated on add_block)
    // Eliminates need for read lock just to get block height
    cached_latest_block_number: Arc<AtomicU64>,

    // Lock-free cached total transaction count (updated on add_block)
    // Eliminates O(n) iteration in get_dag_stats() over the full block vec
    cached_total_tx_count: Arc<AtomicU64>,

    // Transaction replay protection: recent transaction hashes
    // Each entry in the VecDeque is a Vec of tx hashes from one block
    // Pruned when older than 1000 blocks to prevent unbounded growth
    recent_tx_hashes: Arc<RwLock<VecDeque<Vec<[u8; 32]>>>>,

    // DAG double-spend detection: tracks which (address, nonce) pairs have been spent
    // Maps (address, nonce) -> (block_hash, block_number) for conflict detection
    // Pruned after FINALITY_DEPTH blocks to prevent unbounded growth
    spent_outputs: Arc<RwLock<HashMap<(Address, u64), (crate::types::Hash, u64)>>>,

    // Verkle tree for stateless mode
    verkle_state: Option<crate::verkle::VerkleState>,

    pub evm_enabled: bool,
    pub evm_executor: Option<crate::evm::EvmTransactionExecutor>,
    /// Parallel EVM executor for parallel transaction execution
    pub parallel_evm_executor:
        Option<Arc<tokio::sync::RwLock<crate::evm::parallel::ParallelEvmExecutor>>>,

    // Account Abstraction: Wallet registry
    wallet_registry: Option<Arc<tokio::sync::RwLock<crate::account_abstraction::WalletRegistry>>>,

    // Oracle Network
    #[allow(dead_code)]
    oracle_registry: Option<Arc<tokio::sync::RwLock<crate::oracles::OracleRegistry>>>,
    #[allow(dead_code)]
    price_feed_manager: Option<Arc<tokio::sync::RwLock<crate::oracles::PriceFeedManager>>>,
    #[allow(dead_code)]
    vrf_manager: Option<Arc<tokio::sync::RwLock<crate::oracles::VrfManager>>>,
    #[allow(dead_code)]
    oracle_staking: Option<Arc<tokio::sync::RwLock<crate::oracles::OracleStaking>>>,

    // Programmable gas sponsorship
    pub(crate) sponsor_registry: Arc<crate::gas_sponsorship::SponsorRegistry>,

    // Recurring Transactions
    pub(crate) recurring_manager:
        Option<Arc<tokio::sync::RwLock<crate::recurring::RecurringTransactionManager>>>,

    // Built-in Privacy Pool
    pub(crate) privacy_pool: Arc<tokio::sync::RwLock<crate::privacy_pool::PrivacyPool>>,

    // Stop-Loss
    #[allow(dead_code)]
    stop_loss_manager: Option<Arc<tokio::sync::RwLock<crate::stop_loss::StopLossManager>>>,
    // Privacy Layer
    #[cfg(feature = "privacy")]
    privacy_manager: Option<Arc<tokio::sync::RwLock<crate::privacy::PrivacyManager>>>,

    /// Chain ID for EIP-155 replay protection
    chain_id: u64,

    /// ZK verifying key for state transition proof verification (Stream C blocks)
    #[cfg(feature = "privacy")]
    zk_verifying_key: Option<Arc<ark_groth16::VerifyingKey<ark_bn254::Bn254>>>,

    /// Whether to enforce ZK proof validation (hard rejection on invalid/missing proofs)
    /// Default: false (soft enforcement - log warnings only during rollout)
    zk_enforce: bool,

    /// Total fees burned (50% of transaction fees are burned per TOKENOMICS.md)
    /// This tracks the cumulative amount of IDAG removed from circulation via fee burning.
    total_fees_burned: Arc<tokio::sync::RwLock<u128>>,

    /// Startup synchronization barrier: prevents sync/mining before load_from_storage completes
    /// When sled has 5000 blocks but only 100 are loaded into memory, latest_block_number() returns 100
    /// Sync compares against peer height (5000), sees a massive gap, and triggers fork recovery
    /// This flag ensures sync/mining wait until all blocks are loaded from storage
    blockchain_ready: Arc<AtomicBool>,
}

/// Blocks and metadata grouped together (locked as a unit)
#[derive(Clone)]
struct BlocksData {
    blocks: Vec<Block>,
    block_hashes: HashSet<crate::types::Hash>,
    /// O(1) block lookup by hash (hash → Vec index)
    block_by_hash: HashMap<crate::types::Hash, usize>,
    /// O(1) block lookup by height (block_number → Vec index)
    block_by_number: HashMap<u64, usize>,
    /// O(1) transaction lookup: hash → (block_index, tx_index_in_block)
    tx_by_hash: HashMap<crate::types::Hash, (usize, usize)>,
}

/// Account state (balance + nonce) stored in lock-free map
#[derive(Clone, Copy, Debug)]
pub struct AccountState {
    pub balance: u128,
    pub nonce: u64,
}

impl Blockchain {
    /// Create new blockchain without storage (in-memory only)
    pub fn new() -> Self {
        let bc = Self {
            database: None,
            ghostdag: Some(Arc::new(RwLock::new(GhostDAG::new()))),
            blocks: Arc::new(RwLock::new(BlocksData {
                blocks: Vec::new(),
                block_hashes: HashSet::new(),
                block_by_hash: HashMap::new(),
                block_by_number: HashMap::new(),
                tx_by_hash: HashMap::new(),
            })),
            accounts: Arc::new(DashMap::new()),
            cached_latest_block_number: Arc::new(AtomicU64::new(0)),
            cached_total_tx_count: Arc::new(AtomicU64::new(0)),
            recent_tx_hashes: Arc::new(RwLock::new(VecDeque::new())),
            spent_outputs: Arc::new(RwLock::new(HashMap::new())),
            verkle_state: None,
            evm_enabled: false,
            evm_executor: None,
            parallel_evm_executor: None,
            wallet_registry: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            oracle_staking: None,
            sponsor_registry: Arc::new(crate::gas_sponsorship::SponsorRegistry::new()),
            recurring_manager: Some(Arc::new(tokio::sync::RwLock::new(
                crate::recurring::RecurringTransactionManager::new(),
            ))),
            privacy_pool: Arc::new(tokio::sync::RwLock::new(
                // 1 IDAG = 1_000_000_000_000_000_000 attoIDAG; require_proof=false (stub mode)
                crate::privacy_pool::PrivacyPool::new(1_000_000_000_000_000_000, false),
            )),
            stop_loss_manager: None,
            #[cfg(feature = "privacy")]
            privacy_manager: None,
            chain_id: DEFAULT_CHAIN_ID_LOCAL,
            #[cfg(feature = "privacy")]
            zk_verifying_key: None,
            zk_enforce: false,
            total_fees_burned: Arc::new(tokio::sync::RwLock::new(0)),
            blockchain_ready: Arc::new(AtomicBool::new(false)),
        };
        // In-memory blockchain is immediately ready (no storage to load)
        bc.set_ready();
        bc
    }

    /// Create new blockchain with Verkle tree (stateless mode)
    pub fn with_verkle() -> Self {
        let bc = Self {
            database: None,
            ghostdag: Some(Arc::new(RwLock::new(GhostDAG::new()))),
            blocks: Arc::new(RwLock::new(BlocksData {
                blocks: Vec::new(),
                block_hashes: HashSet::new(),
                block_by_hash: HashMap::new(),
                block_by_number: HashMap::new(),
                tx_by_hash: HashMap::new(),
            })),
            accounts: Arc::new(DashMap::new()),
            cached_latest_block_number: Arc::new(AtomicU64::new(0)),
            cached_total_tx_count: Arc::new(AtomicU64::new(0)),
            recent_tx_hashes: Arc::new(RwLock::new(VecDeque::new())),
            spent_outputs: Arc::new(RwLock::new(HashMap::new())),
            verkle_state: Some(crate::verkle::VerkleState::new()),
            evm_enabled: false,
            evm_executor: None,
            parallel_evm_executor: None,
            wallet_registry: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            oracle_staking: None,
            sponsor_registry: Arc::new(crate::gas_sponsorship::SponsorRegistry::new()),
            recurring_manager: Some(Arc::new(tokio::sync::RwLock::new(
                crate::recurring::RecurringTransactionManager::new(),
            ))),
            privacy_pool: Arc::new(tokio::sync::RwLock::new(
                // 1 IDAG = 1_000_000_000_000_000_000 attoIDAG; require_proof=false (stub mode)
                crate::privacy_pool::PrivacyPool::new(1_000_000_000_000_000_000, false),
            )),
            stop_loss_manager: None,
            #[cfg(feature = "privacy")]
            privacy_manager: None,
            chain_id: DEFAULT_CHAIN_ID_LOCAL,
            #[cfg(feature = "privacy")]
            zk_verifying_key: None,
            zk_enforce: false,
            total_fees_burned: Arc::new(tokio::sync::RwLock::new(0)),
            blockchain_ready: Arc::new(AtomicBool::new(false)),
        };
        // In-memory blockchain is immediately ready (no storage to load)
        bc.set_ready();
        bc
    }

    /// Create new blockchain with storage
    pub fn with_storage(database: Arc<Database>) -> crate::error::BlockchainResult<Self> {
        Self::with_storage_and_k(database, 4) // Default K=4
    }

    /// Create new blockchain with storage and specific GhostDAG K parameter
    ///
    /// # Arguments
    /// * `database` - Database for persistent storage
    /// * `ghostdag_k` - GhostDAG security parameter (1-64)
    ///
    /// # Panics
    /// Panics if ghostdag_k is not in range [1, 64]
    pub fn with_storage_and_k(
        database: Arc<Database>,
        ghostdag_k: usize,
    ) -> crate::error::BlockchainResult<Self> {
        // Create GhostDAG with database for hybrid storage
        let ghostdag = Some(Arc::new(RwLock::new(GhostDAG::with_database_and_k(
            database.clone(),
            ghostdag_k,
        ))));

        let mut bc = Self {
            database: Some(database),
            ghostdag,
            blocks: Arc::new(RwLock::new(BlocksData {
                blocks: Vec::new(),
                block_hashes: HashSet::new(),
                block_by_hash: HashMap::new(),
                block_by_number: HashMap::new(),
                tx_by_hash: HashMap::new(),
            })),
            accounts: Arc::new(DashMap::new()),
            cached_latest_block_number: Arc::new(AtomicU64::new(0)),
            cached_total_tx_count: Arc::new(AtomicU64::new(0)),
            recent_tx_hashes: Arc::new(RwLock::new(VecDeque::new())),
            spent_outputs: Arc::new(RwLock::new(HashMap::new())),
            verkle_state: None,
            evm_enabled: false,
            evm_executor: None,
            parallel_evm_executor: None,
            wallet_registry: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            oracle_staking: None,
            sponsor_registry: Arc::new(crate::gas_sponsorship::SponsorRegistry::new()),
            recurring_manager: Some(Arc::new(tokio::sync::RwLock::new(
                crate::recurring::RecurringTransactionManager::new(),
            ))),
            privacy_pool: Arc::new(tokio::sync::RwLock::new(
                // 1 IDAG = 1_000_000_000_000_000_000 attoIDAG; require_proof=false (stub mode)
                crate::privacy_pool::PrivacyPool::new(1_000_000_000_000_000_000, false),
            )),
            stop_loss_manager: None,
            #[cfg(feature = "privacy")]
            privacy_manager: None,
            chain_id: DEFAULT_CHAIN_ID_LOCAL,
            #[cfg(feature = "privacy")]
            zk_verifying_key: None,
            zk_enforce: false,
            total_fees_burned: Arc::new(tokio::sync::RwLock::new(0)),
            blockchain_ready: Arc::new(AtomicBool::new(false)),
        };

        // Load existing blocks and state from storage
        bc.load_from_storage()?;

        // Signal that blockchain is ready for sync/mining
        bc.set_ready();

        Ok(bc)
    }

    /// Create new blockchain with storage and Verkle tree
    pub fn with_storage_and_verkle(
        database: Arc<Database>,
    ) -> crate::error::BlockchainResult<Self> {
        Self::with_storage_verkle_and_k(database, 4) // Default K=4
    }

    /// Create new blockchain with storage, Verkle tree, and specific GhostDAG K parameter
    pub fn with_storage_verkle_and_k(
        database: Arc<Database>,
        ghostdag_k: usize,
    ) -> crate::error::BlockchainResult<Self> {
        // Create GhostDAG with database for hybrid storage
        let ghostdag = Some(Arc::new(RwLock::new(GhostDAG::with_database_and_k(
            database.clone(),
            ghostdag_k,
        ))));

        let mut bc = Self {
            database: Some(database),
            ghostdag,
            blocks: Arc::new(RwLock::new(BlocksData {
                blocks: Vec::new(),
                block_hashes: HashSet::new(),
                block_by_hash: HashMap::new(),
                block_by_number: HashMap::new(),
                tx_by_hash: HashMap::new(),
            })),
            accounts: Arc::new(DashMap::new()),
            cached_latest_block_number: Arc::new(AtomicU64::new(0)),
            cached_total_tx_count: Arc::new(AtomicU64::new(0)),
            recent_tx_hashes: Arc::new(RwLock::new(VecDeque::new())),
            spent_outputs: Arc::new(RwLock::new(HashMap::new())),
            verkle_state: Some(crate::verkle::VerkleState::new()),
            evm_enabled: false,
            evm_executor: None,
            parallel_evm_executor: None,
            wallet_registry: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            oracle_staking: None,
            sponsor_registry: Arc::new(crate::gas_sponsorship::SponsorRegistry::new()),
            recurring_manager: Some(Arc::new(tokio::sync::RwLock::new(
                crate::recurring::RecurringTransactionManager::new(),
            ))),
            privacy_pool: Arc::new(tokio::sync::RwLock::new(
                // 1 IDAG = 1_000_000_000_000_000_000 attoIDAG; require_proof=false (stub mode)
                crate::privacy_pool::PrivacyPool::new(1_000_000_000_000_000_000, false),
            )),
            stop_loss_manager: None,
            #[cfg(feature = "privacy")]
            privacy_manager: None,
            chain_id: DEFAULT_CHAIN_ID_LOCAL,
            #[cfg(feature = "privacy")]
            zk_verifying_key: None,
            zk_enforce: false,
            total_fees_burned: Arc::new(tokio::sync::RwLock::new(0)),
            blockchain_ready: Arc::new(AtomicBool::new(false)),
        };

        // Load existing blocks and state from storage
        bc.load_from_storage()?;

        // Signal that blockchain is ready for sync/mining
        bc.set_ready();

        Ok(bc)
    }

    pub fn with_evm(enable: bool) -> Self {
        let mut bc = Self::new();
        bc.evm_enabled = enable;
        if enable {
            // Create EVM executor with database if available
            if let Some(ref db) = bc.database {
                bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::with_database(
                    db.clone(),
                ));
            } else {
                bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::new());
            }
        }
        bc
    }

    fn blocks_read(&self) -> tokio::sync::RwLockReadGuard<'_, BlocksData> {
        if let Ok(guard) = self.blocks.try_read() {
            return guard;
        }
        self.blocks.blocking_read()
    }

    fn blocks_write(&self) -> tokio::sync::RwLockWriteGuard<'_, BlocksData> {
        if let Ok(guard) = self.blocks.try_write() {
            return guard;
        }
        self.blocks.blocking_write()
    }

    fn recent_tx_hashes_read(&self) -> tokio::sync::RwLockReadGuard<'_, VecDeque<Vec<[u8; 32]>>> {
        if let Ok(guard) = self.recent_tx_hashes.try_read() {
            return guard;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return tokio::task::block_in_place(|| self.recent_tx_hashes.blocking_read());
            }
        }

        self.recent_tx_hashes.blocking_read()
    }

    fn recent_tx_hashes_write(&self) -> tokio::sync::RwLockWriteGuard<'_, VecDeque<Vec<[u8; 32]>>> {
        if let Ok(guard) = self.recent_tx_hashes.try_write() {
            return guard;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return tokio::task::block_in_place(|| self.recent_tx_hashes.blocking_write());
            }
        }

        self.recent_tx_hashes.blocking_write()
    }

    /// Read lock for spent_outputs (DAG double-spend detection)
    fn spent_outputs_read(
        &self,
    ) -> tokio::sync::RwLockReadGuard<'_, HashMap<(Address, u64), (crate::types::Hash, u64)>> {
        if let Ok(guard) = self.spent_outputs.try_read() {
            return guard;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return tokio::task::block_in_place(|| self.spent_outputs.blocking_read());
            }
        }

        self.spent_outputs.blocking_read()
    }

    /// Write lock for spent_outputs (DAG double-spend detection)
    fn spent_outputs_write(
        &self,
    ) -> tokio::sync::RwLockWriteGuard<'_, HashMap<(Address, u64), (crate::types::Hash, u64)>> {
        if let Ok(guard) = self.spent_outputs.try_write() {
            return guard;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return tokio::task::block_in_place(|| self.spent_outputs.blocking_write());
            }
        }

        self.spent_outputs.blocking_write()
    }

    /// Helper for ghostdag write lock - handles both sync and async contexts
    fn with_ghostdag_write<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut crate::consensus::GhostDAG) -> R,
    {
        let ghostdag = self.ghostdag.as_ref()?;

        // Try non-blocking first
        if let Ok(mut guard) = ghostdag.try_write() {
            return Some(f(&mut guard));
        }

        // Handle async runtime contexts
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) {
                return Some(tokio::task::block_in_place(|| {
                    f(&mut ghostdag.blocking_write())
                }));
            }
        }

        // Fallback for non-async context
        Some(f(&mut ghostdag.blocking_write()))
    }

    /// Rollback spent_outputs entries for a specific block (used during reorg)
    /// Removes all (address, nonce) entries that were recorded in the given block
    pub fn rollback_spent_outputs(&self, block_hash: &crate::types::Hash) {
        let mut spent = self.spent_outputs_write();
        let before_count = spent.len();
        spent.retain(|_, (hash, _)| hash != block_hash);
        let after_count = spent.len();
        if before_count != after_count {
            debug!(
                "Rolled back spent_outputs for block {}: removed {} entries",
                hex::encode(block_hash),
                before_count - after_count
            );
        }
    }

    /// Prune spent_outputs entries older than FINALITY_DEPTH blocks
    pub fn prune_spent_outputs(&self, current_height: u64) {
        if current_height <= FINALITY_DEPTH {
            return;
        }
        let cutoff = current_height - FINALITY_DEPTH;
        let mut spent = self.spent_outputs_write();
        let before_count = spent.len();
        spent.retain(|_, (_, block_num)| *block_num > cutoff);
        let after_count = spent.len();
        if before_count != after_count {
            debug!(
                "Pruned spent_outputs: removed {} entries older than block {}",
                before_count - after_count,
                cutoff
            );
        }
    }

    /// Check if a transaction would cause a double-spend
    /// Returns Ok(()) if no conflict, Err if double-spend detected
    fn check_double_spend(
        &self,
        tx: &Transaction,
        current_block_hash: &crate::types::Hash,
    ) -> crate::error::BlockchainResult<()> {
        let spent = self.spent_outputs_read();
        if let Some((existing_block_hash, _)) = spent.get(&(tx.from, tx.nonce)) {
            // Same (address, nonce) was spent in another block
            if existing_block_hash != current_block_hash {
                warn!(
                    "Double-spend detected: address={}, nonce={}, existing_block={}, new_block={}",
                    hex::encode(tx.from),
                    tx.nonce,
                    hex::encode(existing_block_hash),
                    hex::encode(current_block_hash)
                );
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Double-spend detected: (address={}, nonce={}) already spent in block {}",
                    hex::encode(tx.from),
                    tx.nonce,
                    hex::encode(existing_block_hash)
                )));
            }
        }
        Ok(())
    }

    /// Record a spent output for double-spend detection
    fn record_spent_output(
        &self,
        tx: &Transaction,
        block_hash: crate::types::Hash,
        block_number: u64,
    ) {
        let mut spent = self.spent_outputs_write();
        spent.insert((tx.from, tx.nonce), (block_hash, block_number));
    }

    /// Clear blockchain for resync - used when adopting a peer's chain
    ///
    /// This clears BOTH in-memory blocks AND storage to allow syncing a completely
    /// new chain from genesis. This is necessary when a node detects it's on a fork
    /// and needs to adopt the peer's chain.
    pub fn clear_for_resync(&mut self) {
        debug!("Clearing local chain for full resync...");

        // 1. Clear in-memory data
        let mut blocks_data = self.blocks_write();
        let old_count = blocks_data.blocks.len();
        blocks_data.blocks.clear();
        blocks_data.block_hashes.clear();
        drop(blocks_data);

        // 2. Clear storage (CRITICAL: remove old genesis to accept peer's genesis)
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);

            // Clear all blockchain data from storage
            match block_store.clear_all() {
                Ok(cleared) => {
                    debug!("Cleared {} total entries from persistent storage (blocks, DAG metadata, account state)", cleared);
                }
                Err(e) => {
                    error!("Failed to clear storage: {}", e);
                }
            }

            // Force sled flush to ensure blocks are purged from disk before new blocks arrive
            // Without this, sled may keep stale data in its write buffer, causing
            // blocks from the old fork to reappear after restart or during sync.
            if let Err(e) = db.flush() {
                error!("Failed to flush database after clearing: {}", e);
            }
        }

        // 3. Reset cached block number and tx count
        self.cached_latest_block_number.store(0, Ordering::Release);
        self.cached_total_tx_count.store(0, Ordering::Release);

        // 4. Clear GhostDAG state
        self.with_ghostdag_write(|g| g.clear());

        // 5. Clear spent_outputs (double-spend detection state)
        {
            let mut spent = self.spent_outputs_write();
            let spent_count = spent.len();
            spent.clear();
            debug!("Cleared {} spent output entries", spent_count);
        }

        debug!("Cleared {} blocks from memory", old_count);
        debug!("Chain cleared - ready for full resync");
    }

    /// Load blocks and state from storage
    fn load_from_storage(&mut self) -> crate::error::BlockchainResult<()> {
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);

            // Load all blocks from disk
            let stored_blocks = block_store.get_all_blocks()?;

            if !stored_blocks.is_empty() {
                info!("Loading {} blocks from storage...", stored_blocks.len());

                // Sort blocks by block_number to ensure parents are processed before children.
                // This is a topological approximation since parents always have lower block numbers.
                let mut stored_blocks: Vec<_> = stored_blocks.into_iter().collect();
                stored_blocks.sort_by_key(|b| b.header.block_number);

                // Get mutable access to blocks data
                let mut blocks_data = self
                    .blocks
                    .try_write()
                    .expect("Failed to acquire lock during initialization");

                // Add blocks to in-memory cache and GhostDAG storage
                let mut max_block_number = 0;
                let mut total_tx_count = 0u64;
                for block in stored_blocks {
                    let hash = block.hash;
                    let block_num = block.header.block_number;
                    if block_num > max_block_number {
                        max_block_number = block_num;
                    }

                    blocks_data.block_hashes.insert(hash);
                    let block_idx = blocks_data.blocks.len();
                    blocks_data.block_by_hash.insert(hash, block_idx);
                    blocks_data.block_by_number.insert(block_num, block_idx);
                    for (tx_idx, tx) in block.transactions.iter().enumerate() {
                        blocks_data.tx_by_hash.insert(tx.hash, (block_idx, tx_idx));
                    }
                    total_tx_count += block.transactions.len() as u64;
                    blocks_data.blocks.push(block);
                }

                // Update atomic caches with values from storage
                self.cached_latest_block_number
                    .store(max_block_number, Ordering::Release);
                self.cached_total_tx_count
                    .store(total_tx_count, Ordering::Release);

                info!("Loaded {} blocks from storage", blocks_data.blocks.len());

                // SAFETY NET: Re-persist all loaded blocks to fill any gaps in sled storage.
                // This ensures that even if blocks were previously in memory but not persisted
                // (due to a bug or crash), they will be persisted now.
                // This is redundant but safe - BlockStore::put() is idempotent.
                let mut persisted_count = 0;
                for block in &blocks_data.blocks {
                    if let Err(e) = block_store.put(block) {
                        warn!(
                            "Failed to re-persist block #{} during startup: {}",
                            block.header.block_number, e
                        );
                    } else {
                        persisted_count += 1;
                    }
                }
                if persisted_count > 0 {
                    info!(
                        "Safety net: re-persisted {} blocks to sled storage",
                        persisted_count
                    );
                }
            } else {
                info!("No blocks found in storage (starting fresh)");
            }

            // Note: Accounts (balances/nonces) are loaded on-demand for faster startup
            // DashMap will cache them as they're accessed
        }

        Ok(())
    }

    /// Add a locally-mined block whose PoW hash was already verified outside the write lock.
    /// Call this from process_blocks after pre-verifying the hash via spawn_blocking to avoid
    /// holding the blockchain write lock during the ~8.8ms B3MemHash computation.
    pub async fn add_block_pre_verified(
        &mut self,
        block: Block,
    ) -> crate::error::BlockchainResult<()> {
        self.add_block_impl(block, true).await
    }

    /// Add a block to the blockchain with full validation and transaction processing
    pub async fn add_block(&mut self, block: Block) -> crate::error::BlockchainResult<()> {
        self.add_block_impl(block, false).await
    }

    async fn add_block_impl(
        &mut self,
        block: Block,
        skip_pow_verify: bool,
    ) -> crate::error::BlockchainResult<()> {
        // ARC-003/ARC-004: Acquire block processing lock to prevent TOCTOU race condition.
        // This serializes the entire block processing pipeline (validate → process → persist → commit)
        // to ensure that concurrent readers cannot see partially committed state.
        // Even though this function takes &mut self, the Blockchain is typically accessed through
        // Arc<RwLock<Blockchain>>, allowing multiple threads to acquire write locks concurrently.
        let _processing_guard = BLOCK_PROCESSING_LOCK.lock().await;

        debug!(
            "Committing block #{} (txs: {}, hash: {})",
            block.header.block_number,
            block.transactions.len(),
            hex::encode(block.hash)
        );
        if let Some(first) = block.transactions.first() {
            debug!("First tx {}", hex::encode(first.hash));
        }

        // 1. Validate block structure (acquires read lock temporarily)
        self.validate_block_structure(&block)?;

        // 2. Validate block hash (SKIP for genesis block or when pre-verified outside the lock)
        if block.header.block_number > 0 && !skip_pow_verify {
            // CRITICAL FIX: Use the correct hashing algorithm based on stream type
            // Stream A uses Blake3, Stream B uses B3MemHash, Stream C uses Keccak256
            let calculated_hash = match block.header.stream_type {
                StreamType::StreamA => {
                    let tx_hashes: Vec<crate::types::Hash> =
                        block.transactions.iter().map(|tx| tx.hash).collect();
                    let transactions_root: crate::types::Hash =
                        crate::pow::calculate_transactions_root(&tx_hashes);
                    crate::pow::hash_blake3(&block.header, &transactions_root)
                }
                StreamType::StreamB => {
                    let tx_hashes: Vec<crate::types::Hash> =
                        block.transactions.iter().map(|tx| tx.hash).collect();
                    let transactions_root: crate::types::Hash =
                        crate::pow::calculate_transactions_root(&tx_hashes);
                    crate::pow::hash_b3memhash(&block.header, &transactions_root)
                }
                _ => block.calculate_hash(), // Stream C and others use default Keccak256
            };

            if block.hash != calculated_hash {
                return Err(crate::error::BlockchainError::InvalidBlock(format!(
                    "Invalid block hash. Expected: {}, Got: {}",
                    hex::encode(calculated_hash),
                    hex::encode(block.hash)
                )));
            }
        }

        // 3. ZK proof verification for Stream C blocks (privacy feature)
        #[cfg(feature = "privacy")]
        {
            if block.header.stream_type == StreamType::StreamC {
                if let Some(ref proof_bytes) = block.zk_proof {
                    if let Some(ref vk) = self.zk_verifying_key {
                        use crate::types::keccak256;
                        use crate::zk::verify_state_transition;
                        use ark_bn254::Fr;
                        use ark_ff::PrimeField;

                        // Reconstruct public inputs (same as prover side in mining.rs)
                        let tx_hashes: Vec<[u8; 32]> =
                            block.transactions.iter().map(|tx| tx.hash).collect();
                        let transactions_root = crate::pow::calculate_transactions_root(&tx_hashes);

                        // Create placeholder state hashes (hash of "pre" and "post" strings)
                        let pre_state_hash = keccak256(b"pre");
                        let post_state_hash = keccak256(b"post");

                        // Convert hashes to field elements
                        let pre_state_fr = Fr::from_be_bytes_mod_order(&pre_state_hash);
                        let post_state_fr = Fr::from_be_bytes_mod_order(&post_state_hash);
                        let tx_root_fr = Fr::from_be_bytes_mod_order(&transactions_root);
                        let public_inputs = vec![pre_state_fr, post_state_fr, tx_root_fr];

                        match verify_state_transition(vk, proof_bytes, &public_inputs) {
                            Ok(true) => {
                                tracing::debug!(
                                    "ZK proof verified for block {}",
                                    block.header.block_number
                                );
                            }
                            Ok(false) => {
                                tracing::warn!(
                                    "ZK proof INVALID for block {}",
                                    block.header.block_number
                                );
                                if self.zk_enforce {
                                    return Err(crate::error::BlockchainError::InvalidBlock(
                                        "ZK proof verification failed".to_string(),
                                    ));
                                }
                            }
                            Err(e) => {
                                tracing::warn!("ZK verification error: {}", e);
                                if self.zk_enforce {
                                    return Err(crate::error::BlockchainError::InvalidBlock(
                                        format!("ZK verification error: {}", e),
                                    ));
                                }
                            }
                        }
                    } else {
                        tracing::debug!(
                            "No ZK verifying key loaded, skipping proof verification for block {}",
                            block.header.block_number
                        );
                    }
                } else {
                    tracing::warn!(
                        "Stream C block {} missing ZK proof",
                        block.header.block_number
                    );
                    if self.zk_enforce {
                        return Err(crate::error::BlockchainError::InvalidBlock(
                            "Stream C block missing ZK proof".to_string(),
                        ));
                    }
                }
            }
        }

        // 4. Check for duplicate block (read lock)
        {
            let blocks_data = self.blocks_read();
            if blocks_data.block_hashes.contains(&block.hash) {
                return Err(crate::error::BlockchainError::InvalidBlock(
                    "Block already exists".to_string(),
                ));
            }
        }

        // 4. Validate parent hashes (for DAG support)
        self.validate_parent_hashes(&block)?;

        // 5. Validate and process transactions
        self.validate_and_process_transactions(&block).await?;

        // 6. Add block to GhostDAG for consensus ordering
        // PERF (PER-003): Pass &Block reference instead of cloning
        // PERF (PER-002): GhostDAG storage handles disk persistence internally
        // to avoid redundant I/O operations in the hot path.
        if let Some(result) = self.with_ghostdag_write(|g| g.add_block(&block)) {
            result?;
        }

        // 6.5. CRITICAL: Ensure block is persisted to sled storage
        // This is a safety net in case GhostDAG's HybridDagStorage doesn't have a database.
        // Without this, blocks may be lost on restart if GhostDAG was created without a database.
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            if let Err(e) = block_store.put(&block) {
                error!(
                    "Failed to persist block #{} to sled: {}",
                    block.header.block_number, e
                );
                // Don't fail the entire operation - block is still in memory and GhostDAG
            }
        }

        // Collect tx hashes BEFORE moving block
        let tx_hashes_for_replay: Vec<[u8; 32]> = block
            .transactions
            .iter()
            .map(|tx| tx.hash.as_ref().try_into().unwrap())
            .collect();

        // 8. Add block to chain (write lock - short critical section)
        let block_number = block.header.block_number;
        let block_timestamp = block.header.timestamp;
        let block_hash = block.hash; // SEC-013: Capture before move for EVM ring buffer

        // SEC-013: Update EVM block hash ring buffer for BLOCKHASH opcode
        if let Some(ref executor) = self.evm_executor {
            executor.update_block_hash(block_number, block_hash.as_ref().try_into().unwrap());
        }

        let mut blocks_data = self.blocks_write();
        let block_idx = blocks_data.blocks.len();
        blocks_data.block_hashes.insert(block.hash);
        blocks_data.block_by_hash.insert(block.hash, block_idx);
        blocks_data.block_by_number.insert(block_number, block_idx);
        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            blocks_data.tx_by_hash.insert(tx.hash, (block_idx, tx_idx));
        }
        let block_tx_count = block.transactions.len() as u64;
        blocks_data.blocks.push(block);
        drop(blocks_data); // Release write lock immediately

        // 9. Update cached block number and tx count (lock-free, visible to RPC immediately).
        // DAG blocks may arrive out of order; keep the max height, not the last write.
        self.cached_latest_block_number
            .fetch_max(block_number, Ordering::Release);
        self.cached_total_tx_count
            .fetch_add(block_tx_count, Ordering::Relaxed);

        // 10. Record tx hashes for replay protection and prune old entries
        {
            let mut recent_hashes = self.recent_tx_hashes_write();
            recent_hashes.push_back(tx_hashes_for_replay);

            // Prune entries older than 1000 blocks (prevent unbounded growth)
            const MAX_RECENT_BLOCKS: usize = 1000;
            while recent_hashes.len() > MAX_RECENT_BLOCKS {
                recent_hashes.pop_front();
            }
        }

        // 11. Periodic pruning: every 100 blocks, prune spent outputs and red blocks
        if block_number % 100 == 0 {
            self.prune_spent_outputs(block_number);
            self.with_ghostdag_write(|g| g.prune_old_blocks(block_number));
        }

        // 12. Execute any recurring transactions that are due at this block's timestamp.
        self.process_recurring_transactions(block_number, block_timestamp)
            .await;

        Ok(())
    }

    /// Execute due recurring transactions for the given block timestamp.
    ///
    /// Finds all active recurring transactions whose next_execution <= current_timestamp,
    /// transfers value from sender to receiver, and advances the schedule.
    /// Zero fee is charged (the setup RPC call is the user's authorization; the node
    /// executes on their behalf without requiring a new signature each time).
    async fn process_recurring_transactions(&mut self, current_block: u64, current_timestamp: u64) {
        let manager_arc = match &self.recurring_manager {
            Some(m) => Arc::clone(m),
            None => return,
        };

        // Collect due transactions while holding only the read lock.
        let ready: Vec<crate::recurring::RecurringTransaction> = {
            let mgr = manager_arc.read().await;
            mgr.get_ready_to_execute(current_timestamp)
                .into_iter()
                .cloned()
                .collect()
        };

        if ready.is_empty() {
            return;
        }

        for recurring in ready {
            let from_balance = self.get_balance(recurring.from);
            if from_balance < recurring.value {
                warn!(
                    recurring_id = %hex::encode(recurring.recurring_tx_id),
                    from = %hex::encode(recurring.from),
                    needed = recurring.value,
                    have = from_balance,
                    "Recurring tx skipped: insufficient balance",
                );
                let mut mgr = manager_arc.write().await;
                let _ = mgr.mark_failed(&recurring.recurring_tx_id);
                continue;
            }

            // Deduct from sender, credit receiver.
            let from_nonce = self.get_nonce(recurring.from);
            let to_balance = self.get_balance(recurring.to);
            self.accounts.insert(
                recurring.from,
                AccountState {
                    balance: from_balance - recurring.value,
                    nonce: from_nonce + 1,
                },
            );
            self.accounts.insert(
                recurring.to,
                AccountState {
                    balance: to_balance + recurring.value,
                    nonce: self.get_nonce(recurring.to),
                },
            );

            // Derive a deterministic hash for this execution.
            let exec_hash = {
                use sha3::{Digest, Keccak256};
                let mut h = Keccak256::new();
                h.update(recurring.recurring_tx_id.as_ref());
                h.update(&recurring.execution_count.to_le_bytes());
                h.update(&current_block.to_le_bytes());
                crate::types::Hash(h.finalize().into())
            };

            {
                let mut mgr = manager_arc.write().await;
                let _ = mgr.mark_executed(&recurring.recurring_tx_id, exec_hash, current_timestamp);
            }

            info!(
                recurring_id = %hex::encode(recurring.recurring_tx_id),
                from = %hex::encode(recurring.from),
                to = %hex::encode(recurring.to),
                value = recurring.value,
                execution = recurring.execution_count + 1,
                block = current_block,
                "Recurring transaction executed",
            );
        }
    }

    /// Add a block during sync - relaxes timestamp validation for historical blocks.
    /// Returns Ok(true) if the block was newly added, Ok(false) if it was already present (duplicate).
    ///
    /// SECURITY: This function runs enhanced validation for P2P-received blocks:
    /// - Transaction root verification
    /// - Parent hash chain validation
    /// - Duplicate transaction detection
    /// - Block timestamp, number, and size validation
    pub async fn add_block_for_sync(
        &mut self,
        block: Block,
    ) -> crate::error::BlockchainResult<bool> {
        // ARC-003/ARC-004: Acquire block processing lock to prevent TOCTOU race condition.
        // This serializes the entire block processing pipeline (validate → process → persist → commit)
        // to ensure that concurrent readers cannot see partially committed state.
        // Even though this function takes &mut self, the Blockchain is typically accessed through
        // Arc<RwLock<Blockchain>>, allowing multiple threads to acquire write locks concurrently.
        let _processing_guard = BLOCK_PROCESSING_LOCK.lock().await;

        // Collect tx hashes BEFORE moving block
        let tx_hashes_for_replay: Vec<[u8; 32]> = block
            .transactions
            .iter()
            .map(|tx| tx.hash.as_ref().try_into().unwrap())
            .collect();
        let _block_num = block.header.block_number;

        // ENHANCED VALIDATION for P2P-received blocks
        // Run all security checks before accepting the block
        if let Err(validation_err) = self.validate_block_enhanced(&block) {
            warn!(
                "Block #{} failed enhanced validation: {}",
                block.header.block_number, validation_err
            );
            return Err(crate::error::BlockchainError::Validation(
                validation_err.to_string(),
            ));
        }

        // Check for duplicate block
        {
            let blocks_data = self.blocks_read();
            if blocks_data.block_hashes.contains(&block.hash) {
                return Ok(false);
            }
        }

        // Validate and process transactions
        self.validate_and_process_transactions(&block).await?;

        // 6. Add block to GhostDAG
        // PERF (PER-003): Pass &Block reference instead of cloning
        // PERF (PER-002): GhostDAG storage handles disk persistence internally
        // Use with_ghostdag_write to properly handle async runtime context
        if let Some(result) = self.with_ghostdag_write(|gd| gd.add_block(&block)) {
            result?;
        }

        // 6.5. CRITICAL: Ensure block is persisted to sled storage
        // This is a safety net in case GhostDAG's HybridDagStorage doesn't have a database.
        // Without this, blocks may be lost on restart if GhostDAG was created without a database.
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            if let Err(e) = block_store.put(&block) {
                error!(
                    "Failed to persist block #{} to sled: {}",
                    block.header.block_number, e
                );
                // Don't fail the entire operation - block is still in memory and GhostDAG
            }
        }

        // 8. Add block to chain
        let block_number = block.header.block_number;
        let block_hash = block.hash;

        // SEC-013: Update EVM block hash ring buffer for BLOCKHASH opcode
        if let Some(ref executor) = self.evm_executor {
            executor.update_block_hash(block_number, block_hash.as_ref().try_into().unwrap());
        }

        let mut blocks_data = self.blocks_write();
        let block_idx = blocks_data.blocks.len();
        blocks_data.block_hashes.insert(block_hash);
        blocks_data.block_by_hash.insert(block_hash, block_idx);
        blocks_data.block_by_number.insert(block_number, block_idx);
        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            blocks_data.tx_by_hash.insert(tx.hash, (block_idx, tx_idx));
        }
        let block_tx_count = block.transactions.len() as u64;
        blocks_data.blocks.push(block);
        drop(blocks_data);

        // 9. Update cached block number and tx count (max height; sync batches are not strictly ordered).
        self.cached_latest_block_number
            .fetch_max(block_number, Ordering::Release);
        self.cached_total_tx_count
            .fetch_add(block_tx_count, Ordering::Relaxed);

        // 10. Record tx hashes for replay protection and prune old entries
        {
            let mut recent_hashes = self.recent_tx_hashes_write();
            recent_hashes.push_back(tx_hashes_for_replay);

            // Prune entries older than 1000 blocks (prevent unbounded growth)
            const MAX_RECENT_BLOCKS: usize = 1000;
            while recent_hashes.len() > MAX_RECENT_BLOCKS {
                recent_hashes.pop_front();
            }
        }

        Ok(true)
    }

    /// Install a block as a trusted sync checkpoint, bypassing all validation.
    /// Used when a peer has pruned block history — registers the block's hash
    /// so subsequent blocks can resolve it as their parent without needing the
    /// full historical chain.
    pub fn install_sync_checkpoint(&mut self, block: Block) -> bool {
        let block_hash = block.hash;
        let block_number = block.header.block_number;

        let already_known = {
            let blocks_data = self.blocks_read();
            blocks_data.block_hashes.contains(&block_hash)
        };
        if already_known {
            return false;
        }

        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            let _ = block_store.put(&block);
        }

        let mut blocks_data = self.blocks_write();
        let block_idx = blocks_data.blocks.len();
        blocks_data.block_hashes.insert(block_hash);
        blocks_data.block_by_hash.insert(block_hash, block_idx);
        blocks_data.block_by_number.insert(block_number, block_idx);
        let block_tx_count = block.transactions.len() as u64;
        blocks_data.blocks.push(block);
        drop(blocks_data);

        self.cached_latest_block_number
            .fetch_max(block_number, Ordering::Release);
        self.cached_total_tx_count
            .fetch_add(block_tx_count, Ordering::Relaxed);

        true
    }

    /// Validate block structure for sync - skips median timestamp check
    #[allow(dead_code)]
    fn validate_block_structure_for_sync(
        &self,
        block: &Block,
    ) -> crate::error::BlockchainResult<()> {
        // Check block size without allocating the full buffer (DoS protection)
        let block_size = bincode::serialized_size(block)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?
            as usize;
        if block_size > MAX_BLOCK_SIZE {
            return Err(crate::error::BlockchainError::InvalidBlock(format!(
                "Block size {} exceeds maximum {}",
                block_size, MAX_BLOCK_SIZE
            )));
        }

        // Check parent hash count (DoS protection)
        if block.header.parent_hashes.len() > MAX_PARENT_HASHES {
            return Err(crate::error::BlockchainError::InvalidBlock(format!(
                "Too many parent hashes: {} (max: {})",
                block.header.parent_hashes.len(),
                MAX_PARENT_HASHES
            )));
        }

        // For genesis block, allow empty parent hashes
        if block.header.block_number == 0 {
            debug!("Genesis block detected, checking chain state...");
            let blocks_data = self.blocks_read();
            let block_count = blocks_data.blocks.len();
            debug!("Current chain has {} blocks", block_count);
            if !blocks_data.blocks.is_empty() {
                // During sync, we might receive genesis again - ignore it
                debug!("Genesis already exists in chain - will skip silently");
                return Ok(());
            }
            debug!("Genesis block allowed (chain is empty)");
            return Ok(());
        }

        // For non-genesis blocks, must have at least one parent
        if block.header.parent_hashes.is_empty() {
            return Err(crate::error::BlockchainError::InvalidBlock(
                "Non-genesis block must have at least one parent".to_string(),
            ));
        }

        // Validate timestamp - allow historical blocks but reject obviously invalid ones
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Allow 10 minutes into the future (clock skew)
        if block.header.timestamp > current_time + 600 {
            return Err(crate::error::BlockchainError::Validation(
                "Block timestamp too far in future".to_string(),
            ));
        }

        // Must be after 2020
        if block.header.timestamp < 1577836800 {
            return Err(crate::error::BlockchainError::Validation(
                "Block timestamp too old".to_string(),
            ));
        }

        // NOTE: During sync, we SKIP the median timestamp check
        // This allows syncing historical blocks from peers
        // The PoW verification ensures block integrity

        Ok(())
    }

    /// Validate block structure (number, timestamp, etc.)
    fn validate_block_structure(&self, block: &Block) -> crate::error::BlockchainResult<()> {
        // Check block size without allocating the full buffer (DoS protection)
        let block_size = bincode::serialized_size(block)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?
            as usize;
        if block_size > MAX_BLOCK_SIZE {
            return Err(crate::error::BlockchainError::InvalidBlock(format!(
                "Block size {} exceeds maximum {}",
                block_size, MAX_BLOCK_SIZE
            )));
        }

        // Check parent hash count (DoS protection)
        if block.header.parent_hashes.len() > MAX_PARENT_HASHES {
            return Err(crate::error::BlockchainError::InvalidBlock(format!(
                "Too many parent hashes: {} (max: {})",
                block.header.parent_hashes.len(),
                MAX_PARENT_HASHES
            )));
        }

        // For genesis block (block_number 0), allow empty parent hashes
        if block.header.block_number == 0 {
            let blocks_data = self.blocks_read();
            if !blocks_data.blocks.is_empty() {
                return Err(crate::error::BlockchainError::InvalidBlock(
                    "Genesis block must be first".to_string(),
                ));
            }
            return Ok(());
        }

        // For non-genesis blocks, must have at least one parent
        if block.header.parent_hashes.is_empty() {
            return Err(crate::error::BlockchainError::InvalidBlock(
                "Non-genesis block must have at least one parent".to_string(),
            ));
        }

        // Validate timestamp using median of recent blocks (prevents timestamp manipulation)
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Allow some clock skew (10 minutes) - prevents future timestamp attacks
        if block.header.timestamp > current_time + 600 {
            return Err(crate::error::BlockchainError::Validation(
                "Block timestamp too far in future".to_string(),
            ));
        }

        // Check timestamp is reasonable (not before 2020)
        if block.header.timestamp < 1577836800 {
            return Err(crate::error::BlockchainError::Validation(
                "Block timestamp too old".to_string(),
            ));
        }

        // CRITICAL: Validate timestamp against median of last 11 blocks
        // This prevents miners from manipulating timestamps to game difficulty
        let blocks_data = self.blocks_read();
        if !blocks_data.blocks.is_empty() {
            // Get timestamps from recent blocks (up to last 11)
            let recent_blocks: Vec<&Block> = blocks_data.blocks.iter().rev().take(11).collect();

            if !recent_blocks.is_empty() {
                let mut timestamps: Vec<u64> =
                    recent_blocks.iter().map(|b| b.header.timestamp).collect();

                // Calculate median timestamp
                timestamps.sort();
                let median_idx = timestamps.len() / 2;
                let median_timestamp = timestamps[median_idx];

                // New block timestamp must be >= median (with 2s tolerance for out-of-order / clock skew)
                const MEDIAN_TOLERANCE_SECS: u64 = 2;
                if block.header.timestamp + MEDIAN_TOLERANCE_SECS < median_timestamp {
                    return Err(crate::error::BlockchainError::Validation(format!(
                        "Block timestamp {} is before median timestamp {} of recent blocks",
                        block.header.timestamp, median_timestamp
                    )));
                }
            }
        }

        Ok(())
    }

    /// Validate parent hashes exist in the blockchain (DAG support)
    fn validate_parent_hashes(&self, block: &Block) -> crate::error::BlockchainResult<()> {
        if block.header.block_number == 0 {
            // Genesis block - no parents needed
            return Ok(());
        }

        // Check that at least one parent exists
        let blocks_data = self.blocks_read();
        let mut found_parent = false;
        for parent_hash in &block.header.parent_hashes {
            if blocks_data.block_hashes.contains(parent_hash) {
                found_parent = true;
                break;
            }
        }

        if !found_parent {
            return Err(crate::error::BlockchainError::InvalidBlock(
                "No valid parent hash found".to_string(),
            ));
        }

        Ok(())
    }

    /// Enhanced parent hash validation for BlockDAG (GhostDAG).
    /// Requires AT LEAST ONE parent to be known and within MAX_PARENT_AGE.
    /// Unknown secondary parents are allowed because a snapshot-synced node
    /// may not have the full historical DAG; as long as the block is reachable
    /// from one known ancestor it is safe to accept.
    fn validate_parent_hashes_enhanced(&self, block: &Block) -> Result<(), BlockValidationError> {
        if block.header.block_number == 0 {
            return Ok(());
        }

        if block.header.parent_hashes.is_empty() {
            warn!(
                "Block #{} rejected: non-genesis block has no parents",
                block.header.block_number
            );
            return Err(BlockValidationError::NoParents);
        }

        if block.header.parent_hashes.len() > MAX_PARENT_HASHES {
            warn!(
                "Block #{} rejected: too many parent hashes ({} > max {})",
                block.header.block_number,
                block.header.parent_hashes.len(),
                MAX_PARENT_HASHES
            );
            return Err(BlockValidationError::TooManyParentHashes {
                count: block.header.parent_hashes.len(),
                max: MAX_PARENT_HASHES,
            });
        }

        let blocks_data = self.blocks_read();
        let current_block_number = block.header.block_number;

        // Accept if ANY parent is known and not too ancient.
        // In a BlockDAG a block legitimately references multiple tips; a
        // snapshot-synced node may only have a subset of them.
        let mut any_valid_parent = false;
        for parent_hash in &block.header.parent_hashes {
            if !blocks_data.block_hashes.contains(parent_hash) {
                continue; // Unknown parent — skip, not an error
            }

            // Genesis parent is always valid regardless of age
            if let Some(parent_block) = blocks_data.blocks.iter().find(|b| &b.hash == parent_hash) {
                if parent_block.header.block_number == 0 {
                    any_valid_parent = true;
                    break;
                }
                let parent_age =
                    current_block_number.saturating_sub(parent_block.header.block_number);
                if parent_age <= MAX_PARENT_AGE {
                    any_valid_parent = true;
                    break;
                }
                // This particular parent is too ancient — keep looking
            } else {
                // Parent hash is in block_hashes but not in blocks vec
                // (e.g. checkpoint installed directly) — accept it
                any_valid_parent = true;
                break;
            }
        }

        if !any_valid_parent {
            warn!(
                "Block #{} rejected: no valid parent found (all parents unknown or too ancient)",
                block.header.block_number
            );
            return Err(BlockValidationError::UnknownParent(
                "no valid parent in local chain".to_string(),
            ));
        }

        Ok(())
    }

    /// Verify transaction root (Merkle root) of the block
    /// Ensures transactions haven't been tampered with
    fn verify_tx_root(&self, block: &Block) -> Result<(), BlockValidationError> {
        // Compute the transaction root from the block's transactions
        let tx_hashes: Vec<crate::types::Hash> =
            block.transactions.iter().map(|tx| tx.hash).collect();
        let computed_root = crate::pow::calculate_transactions_root(&tx_hashes);

        // Recompute the block hash to verify it matches
        // The block hash includes the transaction root via the PoW functions
        let expected_hash = match block.header.stream_type {
            StreamType::StreamA => crate::pow::hash_blake3(&block.header, &computed_root),
            StreamType::StreamB => crate::pow::hash_b3memhash(&block.header, &computed_root),
            _ => {
                // For Stream C and others, compute using standard Keccak256
                use sha3::{Digest, Keccak256};
                let mut hasher = Keccak256::new();
                hasher.update(&block.header.calculate_header_hash());
                for tx in &block.transactions {
                    hasher.update(tx.hash.as_ref());
                }
                let result = hasher.finalize();
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&result);
                crate::types::Hash(hash)
            }
        };

        if block.hash != expected_hash {
            warn!(
                "Block #{} rejected: hash mismatch (possible tx_root tampering)",
                block.header.block_number
            );
            return Err(BlockValidationError::InvalidBlockHash {
                expected: hex::encode(&block.hash),
                computed: hex::encode(expected_hash),
            });
        }

        Ok(())
    }

    /// Check for duplicate transactions within a block
    fn check_duplicate_transactions(&self, block: &Block) -> Result<(), BlockValidationError> {
        let mut seen_tx_hashes = HashSet::new();

        for tx in &block.transactions {
            let tx_hash = tx.hash;
            if !seen_tx_hashes.insert(tx_hash) {
                warn!(
                    "Block #{} rejected: duplicate transaction {}",
                    block.header.block_number,
                    hex::encode(&tx_hash)
                );
                return Err(BlockValidationError::DuplicateTransaction(hex::encode(
                    &tx_hash,
                )));
            }
        }

        Ok(())
    }

    /// Validate block timestamp is within acceptable range
    fn validate_block_timestamp(&self, block: &Block) -> Result<(), BlockValidationError> {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Check timestamp is not too far in the future
        if block.header.timestamp > current_time + MAX_TIMESTAMP_DRIFT {
            warn!(
                "Block #{} rejected: timestamp {} is too far in future (current: {}, max drift: {})",
                block.header.block_number,
                block.header.timestamp,
                current_time,
                MAX_TIMESTAMP_DRIFT
            );
            return Err(BlockValidationError::TimestampTooFarInFuture {
                timestamp: block.header.timestamp,
                current: current_time,
                max_future: MAX_TIMESTAMP_DRIFT,
            });
        }

        // Check timestamp is not too old
        if block.header.timestamp < MIN_TIMESTAMP {
            warn!(
                "Block #{} rejected: timestamp {} is too old (minimum: {})",
                block.header.block_number, block.header.timestamp, MIN_TIMESTAMP
            );
            return Err(BlockValidationError::TimestampTooOld {
                timestamp: block.header.timestamp,
                minimum: MIN_TIMESTAMP,
            });
        }

        Ok(())
    }

    /// Validate block number consistency with parent
    fn validate_block_number(&self, block: &Block) -> Result<(), BlockValidationError> {
        if block.header.block_number == 0 {
            // Genesis block is always valid
            return Ok(());
        }

        // For non-genesis blocks, verify the block number is consistent
        // The block number should be parent's number + 1 for single-parent chains
        // For GhostDAG with multiple parents, we check that at least one parent has block_number - 1
        let blocks_data = self.blocks_read();
        let mut valid_parent_found = false;

        for parent_hash in &block.header.parent_hashes {
            if let Some(parent_block) = blocks_data.blocks.iter().find(|b| &b.hash == parent_hash) {
                // Allow blocks to reference parents with same or previous block number
                // This accommodates both linear chains and DAG structures
                if parent_block.header.block_number == block.header.block_number.saturating_sub(1) {
                    valid_parent_found = true;
                    break;
                }
            }
        }

        if !valid_parent_found && !block.header.parent_hashes.is_empty() {
            // In GhostDAG, blocks can have multiple parents at different heights
            // We just log a warning but don't reject - this is expected DAG behavior
            debug!(
                "Block #{} has parents at non-consecutive heights (expected DAG behavior)",
                block.header.block_number
            );
        }

        Ok(())
    }

    /// Validate block size and transaction count limits
    fn validate_block_limits(&self, block: &Block) -> Result<(), BlockValidationError> {
        // Check transaction count
        if block.transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
            warn!(
                "Block #{} rejected: too many transactions ({} > max {})",
                block.header.block_number,
                block.transactions.len(),
                MAX_TRANSACTIONS_PER_BLOCK
            );
            return Err(BlockValidationError::MaxTransactionsExceeded {
                count: block.transactions.len(),
                max: MAX_TRANSACTIONS_PER_BLOCK,
            });
        }

        // Check serialized block size
        match bincode::serialize(block) {
            Ok(serialized) => {
                if serialized.len() > MAX_BLOCK_SIZE {
                    warn!(
                        "Block #{} rejected: size {} bytes exceeds max {} bytes",
                        block.header.block_number,
                        serialized.len(),
                        MAX_BLOCK_SIZE
                    );
                    return Err(BlockValidationError::BlockSizeExceeded {
                        size: serialized.len(),
                        max: MAX_BLOCK_SIZE,
                    });
                }
            }
            Err(e) => {
                warn!(
                    "Block #{} rejected: serialization error: {}",
                    block.header.block_number, e
                );
                return Err(BlockValidationError::BlockSizeExceeded {
                    size: usize::MAX,
                    max: MAX_BLOCK_SIZE,
                });
            }
        }

        Ok(())
    }

    /// Run all enhanced validations on a block
    /// This is called for blocks received from P2P (not self-mined)
    fn validate_block_enhanced(&self, block: &Block) -> Result<(), BlockValidationError> {
        // 1. Validate block limits (size, tx count)
        self.validate_block_limits(block)?;

        // 2. Validate timestamp
        self.validate_block_timestamp(block)?;

        // 3. Validate parent hashes
        self.validate_parent_hashes_enhanced(block)?;

        // 4. Validate block number consistency
        self.validate_block_number(block)?;

        // 5. Check for duplicate transactions
        self.check_duplicate_transactions(block)?;

        // 6. Verify transaction root (via block hash recomputation)
        self.verify_tx_root(block)?;

        // 7. Verify PQ signature if present (optional, additive security)
        self.validate_pq_signature(block);

        Ok(())
    }

    /// Validate post-quantum signature on block header (if present)
    /// PQ signature is OPTIONAL — blocks without PQ signatures remain valid
    /// Verification failures are logged as warnings, not errors
    fn validate_pq_signature(&self, block: &Block) {
        // Check if block has a PQ signature
        if let Some(ref pq_sig) = block.header.pq_signature {
            // Need the miner's PQ public key
            if let Some(ref miner_pubkey) = block.header.miner_pq_pubkey {
                // Calculate the signing hash (header without PQ signature)
                let signing_hash = block.header.calculate_signing_hash();

                // Verify using the PqSignature verification
                if crate::pqc::PqAccount::verify_signature(&signing_hash, pq_sig) {
                    debug!(
                        "Block {} has valid Dilithium3 PQ signature from miner {}",
                        block.header.block_number,
                        hex::encode(&miner_pubkey[..8.min(miner_pubkey.len())])
                    );
                } else {
                    warn!(
                        "Block {} PQ signature verification failed — signature invalid",
                        block.header.block_number
                    );
                    // Don't reject the block — PQ is additive, not required
                }
            } else {
                warn!(
                    "Block {} has PQ signature but no miner_pq_pubkey — cannot verify",
                    block.header.block_number
                );
            }
        }
        // If no PQ signature present, that's fine — PQ is optional for backward compatibility
    }

    /// Validate and process all transactions in the block
    async fn validate_and_process_transactions(
        &mut self,
        block: &Block,
    ) -> crate::error::BlockchainResult<()> {
        let current_block = block.header.block_number;
        let current_timestamp = block.header.timestamp;
        let block_hash = block.hash;

        // Parallel stateless pre-pass: signature verification, hash check, fee/size/gas/chain_id
        // bounds. Runs on the rayon thread pool without holding any blockchain locks.
        // Only covers regular (non-privacy, non-multisig) transactions.
        {
            use rayon::prelude::*;
            let chain_id = self.chain_id;
            let evm_enabled = self.evm_enabled;
            block
                .transactions
                .par_iter()
                .filter(|tx| {
                    if tx.multisig_signatures.is_some() {
                        return false;
                    }
                    #[cfg(feature = "privacy")]
                    if tx.privacy_data.is_some() {
                        return false;
                    }
                    true
                })
                .try_for_each(|tx| {
                    Self::verify_tx_stateless(tx, chain_id, evm_enabled, current_block)
                })?;
        }

        for tx in &block.transactions {
            if !tx.is_ready_to_execute(current_block, current_timestamp) {
                return Err(crate::error::BlockchainError::InvalidTransaction(
                    format!(
                        "Time-locked transaction not ready: execute_at_block={:?}, execute_at_timestamp={:?}, current_block={}, current_timestamp={}",
                        tx.execute_at_block, tx.execute_at_timestamp, current_block, current_timestamp
                    )
                ));
            }

            // DAG DOUBLE-SPEND DETECTION: Check for conflicting (address, nonce) pairs
            self.check_double_spend(tx, &block_hash)?;

            // Privacy/multisig txs: full validation (stateless + stateful).
            // Regular txs: stateful-only — stateless was done in parallel above.
            let is_special = tx.multisig_signatures.is_some() || {
                #[cfg(feature = "privacy")]
                {
                    tx.privacy_data.is_some()
                }
                #[cfg(not(feature = "privacy"))]
                {
                    false
                }
            };

            if is_special {
                self.validate_transaction(tx, current_block, current_timestamp)
                    .await?;
            } else {
                self.validate_tx_stateful_only(tx, current_block).await?;
            }

            self.process_transaction(tx).await?;
            self.record_spent_output(tx, block_hash, current_block);

            // Update the sponsor's rolling spend window after the tx is committed.
            if let Some(ref sponsor) = tx.sponsor {
                self.sponsor_registry
                    .record_spend(sponsor, tx.fee, current_block);
            }
        }

        self.prune_spent_outputs(current_block);

        Ok(())
    }

    /// Stateless checks for a regular (non-privacy, non-multisig) transaction.
    /// All checks here are pure functions of the tx fields — no blockchain state needed.
    /// Called in parallel by validate_and_process_transactions before the write lock.
    fn verify_tx_stateless(
        tx: &Transaction,
        chain_id: u64,
        evm_enabled: bool,
        current_block: u64,
    ) -> crate::error::BlockchainResult<()> {
        if !tx.verify_signature(current_block).unwrap_or(false) {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Invalid transaction signature".to_string(),
            ));
        }

        if tx.fee == 0 && !tx.from.iter().all(|&b| b == 0) {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction fee must be greater than zero".to_string(),
            ));
        }

        if tx.data.len() > MAX_TX_DATA_SIZE {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "transaction data size {} exceeds maximum {}",
                tx.data.len(),
                MAX_TX_DATA_SIZE
            )));
        }

        if tx.value > MAX_TX_VALUE {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction value exceeds maximum allowed".to_string(),
            ));
        }

        if tx.value.checked_add(tx.fee).is_none() {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction value + fee overflows".to_string(),
            ));
        }

        if tx.gas_limit > MAX_GAS_LIMIT {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "gas_limit {} exceeds maximum {}",
                tx.gas_limit, MAX_GAS_LIMIT
            )));
        }

        if tx.gas_limit == 0 {
            return Err(crate::error::BlockchainError::Validation(
                "Gas limit cannot be zero".to_string(),
            ));
        }

        if let Some(tx_chain_id) = tx.chain_id {
            if tx_chain_id != chain_id {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Invalid chain ID: expected {}, got {}",
                    chain_id, tx_chain_id
                )));
            }
        }

        let calculated_hash = tx.calculate_hash();
        if tx.hash != calculated_hash {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Invalid transaction hash".to_string(),
            ));
        }

        if evm_enabled && !tx.data.is_empty() && tx.gas_limit < 21_000 {
            return Err(crate::error::BlockchainError::Validation(
                "Gas limit too low for contract interaction".to_string(),
            ));
        }

        Ok(())
    }

    /// Stateful-only validation for regular transactions.
    /// Assumes stateless checks (sig, hash, fee, bounds) were already done by verify_tx_stateless.
    async fn validate_tx_stateful_only(
        &self,
        tx: &Transaction,
        current_block: u64,
    ) -> crate::error::BlockchainResult<()> {
        // Enforce the sponsor's registered policy before checking balances.
        if let Some(ref sponsor) = tx.sponsor {
            self.sponsor_registry
                .check(sponsor, &tx.from, tx.fee, current_block)
                .map_err(|e| {
                    crate::error::BlockchainError::InvalidTransaction(format!(
                        "Sponsor policy violation: {}",
                        e
                    ))
                })?;
        }

        let current_nonce = if let Some(ref wallet_registry) = self.wallet_registry {
            if let Ok(registry) = wallet_registry.try_read() {
                if let Some(wallet) = registry.get_wallet(&tx.from) {
                    wallet.get_nonce()
                } else {
                    self.get_nonce(tx.from)
                }
            } else {
                self.get_nonce(tx.from)
            }
        } else {
            self.get_nonce(tx.from)
        };

        if tx.nonce != current_nonce {
            if tx.nonce > current_nonce.saturating_add(MAX_NONCE_GAP) {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Nonce gap too large: tx nonce {} but account nonce is {}. Max gap is {}.",
                    tx.nonce, current_nonce, MAX_NONCE_GAP
                )));
            }
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "Invalid nonce: expected {}, got {}",
                current_nonce, tx.nonce
            )));
        }

        if let Some(ref wallet_registry) = self.wallet_registry {
            if let Ok(registry) = wallet_registry.try_read() {
                if let Some(wallet) = registry.get_wallet(&tx.from) {
                    if wallet.has_spending_limits() {
                        if let Some(ref limits) = wallet.config.spending_limits {
                            let mut limits_check = limits.clone();
                            if let Err(e) = limits_check.check_limit(tx.value) {
                                return Err(crate::error::BlockchainError::InvalidTransaction(
                                    format!("Spending limit exceeded: {}", e),
                                ));
                            }
                        }
                    }
                }
            }
        }

        let sender_balance = self.get_balance(tx.from);
        if sender_balance < tx.value {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "Insufficient balance: have {}, need {} (value)",
                sender_balance, tx.value
            )));
        }

        if let Some(sponsor) = tx.sponsor {
            let sponsor_balance = self.get_balance(sponsor);
            if sponsor_balance < tx.fee {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Insufficient sponsor balance: sponsor has {}, needs {} (fee)",
                    sponsor_balance, tx.fee
                )));
            }
        } else {
            let total_required = tx.value.saturating_add(tx.fee);
            if sender_balance < total_required {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Insufficient balance: have {}, need {} (value: {} + fee: {})",
                    sender_balance, total_required, tx.value, tx.fee
                )));
            }
        }

        Ok(())
    }

    /// Validate a single transaction
    async fn validate_transaction(
        &self,
        tx: &Transaction,
        _current_block: u64,
        _current_timestamp: u64,
    ) -> crate::error::BlockchainResult<()> {
        // For privacy transactions, validate zk-SNARK proof instead of signature
        #[cfg(feature = "privacy")]
        {
            if let Some(ref privacy_tx) = tx.privacy_data {
                return self.validate_privacy_transaction(tx, privacy_tx).await;
            }
        }

        // For multi-signature transactions, validate multi-sig instead of single signature
        if let Some(ref multisig_sigs) = tx.multisig_signatures {
            // This is a multi-sig transaction - validate multi-sig
            if let Some(ref wallet_registry) = self.wallet_registry {
                if let Ok(registry) = wallet_registry.try_read() {
                    if let Some(wallet) = registry.get_wallet(&tx.from) {
                        if wallet.is_multisig() {
                            // Validate multi-sig transaction
                            match &wallet.wallet_type {
                                crate::account_abstraction::WalletType::MultiSig {
                                    signers,
                                    threshold,
                                }
                                | crate::account_abstraction::WalletType::Combined {
                                    signers,
                                    threshold,
                                    ..
                                } => {
                                    // Check we have enough signatures
                                    if multisig_sigs.len() < *threshold as usize {
                                        return Err(
                                            crate::error::BlockchainError::InvalidTransaction(
                                                format!(
                                                    "Insufficient signatures: need {}, have {}",
                                                    threshold,
                                                    multisig_sigs.len()
                                                ),
                                            ),
                                        );
                                    }

                                    // Check all signers are in expected list
                                    let signed_by: Vec<Address> =
                                        multisig_sigs.iter().map(|(addr, _, _)| *addr).collect();
                                    let signers_set: HashSet<Address> =
                                        signers.iter().copied().map(Into::into).collect();
                                    for signer in &signed_by {
                                        if !signers_set.contains(signer) {
                                            return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Unknown signer: {}",
                                                        hex::encode(*signer)
                                                    ),
                                                ),
                                            );
                                        }
                                    }

                                    // Check for duplicate signers
                                    use std::collections::HashSet;
                                    let mut seen = HashSet::new();
                                    for signer in &signed_by {
                                        if seen.contains(signer) {
                                            return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Duplicate signer: {}",
                                                        hex::encode(*signer)
                                                    ),
                                                ),
                                            );
                                        }
                                        seen.insert(*signer);
                                    }

                                    // Verify cryptographic signatures (SEC-001)
                                    // Each signature in multisig_sigs is (address, signature_bytes, public_key_bytes)
                                    let mut valid_signatures = 0;
                                    for (signer_addr, sig_bytes, pub_key_bytes) in
                                        multisig_sigs.iter()
                                    {
                                        // Validate signature length (Ed25519 signatures are 64 bytes)
                                        if sig_bytes.len() != 64 {
                                            return Err(crate::error::BlockchainError::InvalidTransaction(
                                                format!("Invalid signature length for signer {}: expected 64, got {}",
                                                    hex::encode(signer_addr), sig_bytes.len())
                                            ));
                                        }

                                        // Validate public key length (Ed25519 public keys are 32 bytes)
                                        if pub_key_bytes.len() != 32 {
                                            return Err(crate::error::BlockchainError::InvalidTransaction(
                                                format!("Invalid public key length for signer {}: expected 32, got {}",
                                                    hex::encode(signer_addr), pub_key_bytes.len())
                                            ));
                                        }

                                        // Verify the signature cryptographically
                                        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

                                        let pub_key_array: [u8; 32] = match pub_key_bytes
                                            .as_slice()
                                            .try_into()
                                        {
                                            Ok(b) => b,
                                            Err(_) => return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Invalid public key format for signer {}",
                                                        hex::encode(signer_addr)
                                                    ),
                                                ),
                                            ),
                                        };

                                        let verifying_key = match VerifyingKey::from_bytes(
                                            &pub_key_array,
                                        ) {
                                            Ok(key) => key,
                                            Err(_) => return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Invalid public key for signer {}",
                                                        hex::encode(signer_addr)
                                                    ),
                                                ),
                                            ),
                                        };

                                        let sig_array: [u8; 64] = match sig_bytes
                                            .as_slice()
                                            .try_into()
                                        {
                                            Ok(b) => b,
                                            Err(_) => return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Invalid signature format for signer {}",
                                                        hex::encode(signer_addr)
                                                    ),
                                                ),
                                            ),
                                        };

                                        let signature = match Signature::try_from(&sig_array[..]) {
                                            Ok(s) => s,
                                            Err(_) => return Err(
                                                crate::error::BlockchainError::InvalidTransaction(
                                                    format!(
                                                        "Invalid signature for signer {}",
                                                        hex::encode(signer_addr)
                                                    ),
                                                ),
                                            ),
                                        };

                                        // Verify signature against transaction hash
                                        match verifying_key.verify(&tx.hash, &signature) {
                                            Ok(_) => {
                                                // Also verify that the public key matches the signer address
                                                // Address should be last 20 bytes of Keccak256(public_key)
                                                use crate::types::derive_eth_address;
                                                let derived_address =
                                                    derive_eth_address(&pub_key_array);

                                                if derived_address != *signer_addr {
                                                    return Err(crate::error::BlockchainError::InvalidTransaction(
                                                        format!("Public key does not match signer address for {}",
                                                            hex::encode(signer_addr))
                                                    ));
                                                }

                                                valid_signatures += 1;
                                            }
                                            Err(_) => {
                                                return Err(crate::error::BlockchainError::InvalidTransaction(
                                                    format!("Invalid signature for signer {}", hex::encode(signer_addr))
                                                ));
                                            }
                                        }
                                    }

                                    // Ensure we have at least threshold valid signatures
                                    if valid_signatures < *threshold as usize {
                                        return Err(crate::error::BlockchainError::InvalidTransaction(
                                            format!("Insufficient valid signatures: need {}, have {}",
                                                threshold, valid_signatures)
                                        ));
                                    }
                                }
                                _ => {
                                    return Err(crate::error::BlockchainError::InvalidTransaction(
                                        "Multi-sig signatures provided but wallet is not multi-sig"
                                            .to_string(),
                                    ));
                                }
                            }
                        } else {
                            return Err(crate::error::BlockchainError::InvalidTransaction(
                                "Multi-sig signatures provided but wallet is not multi-sig"
                                    .to_string(),
                            ));
                        }
                    } else {
                        return Err(crate::error::BlockchainError::InvalidTransaction(
                            "Multi-sig transaction from non-contract wallet".to_string(),
                        ));
                    }
                }
            }
        } else {
            // Regular transaction - verify single signature
            if !tx.verify_signature(_current_block).unwrap_or(false) {
                return Err(crate::error::BlockchainError::InvalidTransaction(
                    "Invalid transaction signature".to_string(),
                ));
            }
        }

        // Reject zero-fee transactions (prevent block space abuse)
        // Allow zero-fee only from zero address (system/genesis transactions)
        if tx.fee == 0 && !tx.from.iter().all(|&b| b == 0) {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction fee must be greater than zero".to_string(),
            ));
        }

        // Check transaction data size (DoS protection)
        if tx.data.len() > MAX_TX_DATA_SIZE {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "transaction data size {} exceeds maximum {}",
                tx.data.len(),
                MAX_TX_DATA_SIZE
            )));
        }

        // Maximum value sanity check to prevent overflow in balance arithmetic
        if tx.value > MAX_TX_VALUE {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction value exceeds maximum allowed".to_string(),
            ));
        }

        // Fee + value overflow check
        if tx.value.checked_add(tx.fee).is_none() {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "transaction value + fee overflows".to_string(),
            ));
        }

        // Gas limit reasonableness check
        if tx.gas_limit > MAX_GAS_LIMIT {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "gas_limit {} exceeds maximum {}",
                tx.gas_limit, MAX_GAS_LIMIT
            )));
        }

        // Verify chain_id for EIP-155 replay protection
        // If chain_id is set, it must match our expected chain ID
        if let Some(tx_chain_id) = tx.chain_id {
            if tx_chain_id != self.chain_id {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Invalid chain ID: expected {}, got {}",
                    self.chain_id, tx_chain_id
                )));
            }
        }

        // Check transaction hash
        let calculated_hash = tx.calculate_hash();
        if tx.hash != calculated_hash {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Invalid transaction hash".to_string(),
            ));
        }

        // Check nonce (must be exactly equal to current nonce for strict sequential ordering)
        // For contract wallets, use wallet nonce; for EOA, use account nonce
        let current_nonce = if let Some(ref wallet_registry) = self.wallet_registry {
            // Check if sender is a contract wallet
            // Note: Using try_read() for non-blocking access in sync context
            // In production, this would be handled differently (async validation or sync registry)
            if let Ok(registry) = wallet_registry.try_read() {
                if let Some(wallet) = registry.get_wallet(&tx.from) {
                    wallet.get_nonce() // Use wallet nonce
                } else {
                    self.get_nonce(tx.from) // Use account nonce for EOA
                }
            } else {
                // If we can't acquire the lock, fall back to account nonce
                // This is a temporary solution - in production, validation should be async
                self.get_nonce(tx.from)
            }
        } else {
            self.get_nonce(tx.from) // Fallback to account nonce if no registry
        };

        if tx.nonce != current_nonce {
            // Reject nonces too far in the future (nonce gap protection for mempool)
            if tx.nonce > current_nonce.saturating_add(MAX_NONCE_GAP) {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Nonce gap too large: tx nonce {} but account nonce is {}. Max gap is {}.",
                    tx.nonce, current_nonce, MAX_NONCE_GAP
                )));
            }
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "Invalid nonce: expected {}, got {}",
                current_nonce, tx.nonce
            )));
        }

        // For contract wallets, check spending limits if applicable
        if let Some(ref wallet_registry) = self.wallet_registry {
            if let Ok(registry) = wallet_registry.try_read() {
                if let Some(wallet) = registry.get_wallet(&tx.from) {
                    if wallet.has_spending_limits() {
                        // Check spending limits
                        if let Some(ref limits) = wallet.config.spending_limits {
                            // Clone limits to allow mutation
                            let mut limits_check = limits.clone();
                            if let Err(e) = limits_check.check_limit(tx.value) {
                                return Err(crate::error::BlockchainError::InvalidTransaction(
                                    format!("Spending limit exceeded: {}", e),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Check balance: For gasless transactions, sponsor pays fee; sender pays value
        // For regular transactions, sender pays both value and fee
        let sender_balance = self.get_balance(tx.from);
        let sender_required = tx.value; // Sender always pays the value

        if sender_balance < sender_required {
            return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                "Insufficient balance: have {}, need {} (value)",
                sender_balance, sender_required
            )));
        }

        // Check sponsor balance for gasless transactions
        if let Some(sponsor) = tx.sponsor {
            let sponsor_balance = self.get_balance(sponsor);
            if sponsor_balance < tx.fee {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Insufficient sponsor balance: sponsor has {}, needs {} (fee)",
                    sponsor_balance, tx.fee
                )));
            }
        } else {
            // Regular transaction: sender pays both value and fee
            let total_required = tx.value.saturating_add(tx.fee);
            if sender_balance < total_required {
                return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                    "Insufficient balance: have {}, need {} (value: {} + fee: {})",
                    sender_balance, total_required, tx.value, tx.fee
                )));
            }
        }

        // Validate gas limit (must be reasonable)
        if tx.gas_limit == 0 {
            return Err(crate::error::BlockchainError::Validation(
                "Gas limit cannot be zero".to_string(),
            ));
        }

        // For EVM transactions, validate data
        if self.evm_enabled && !tx.data.is_empty() {
            if tx.gas_limit < 21_000 {
                return Err(crate::error::BlockchainError::Validation(
                    "Gas limit too low for contract interaction".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Validate privacy transaction (zk-SNARK proof)
    #[cfg(feature = "privacy")]
    async fn validate_privacy_transaction(
        &self,
        _tx: &Transaction,
        privacy_tx: &crate::privacy::PrivacyTransaction,
    ) -> crate::error::BlockchainResult<()> {
        // Check if privacy manager is available
        let privacy_manager = self.privacy_manager.as_ref().ok_or_else(|| {
            crate::error::BlockchainError::InvalidTransaction(
                "Privacy manager not available".to_string(),
            )
        })?;

        // Verify zk-SNARK proof
        if privacy_tx.proof.is_empty() {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Privacy transaction missing proof".to_string(),
            ));
        }

        // Deserialize proof from transaction
        let proof =
            crate::privacy::PrivacyVerifier::deserialize_proof(&privacy_tx.proof).map_err(|e| {
                crate::error::BlockchainError::InvalidTransaction(format!(
                    "Failed to deserialize privacy proof: {}",
                    e
                ))
            })?;

        // Parse public inputs from transaction (convert bytes to field elements)
        use ark_bn254::Fr;
        use ark_ff::PrimeField;
        let public_inputs: Vec<Fr> = privacy_tx
            .public_inputs
            .iter()
            .map(|input_bytes| {
                let mut bytes = [0u8; 32];
                if input_bytes.len() >= 32 {
                    bytes.copy_from_slice(&input_bytes[..32]);
                } else {
                    bytes[..input_bytes.len()].copy_from_slice(input_bytes);
                }
                Fr::from_le_bytes_mod_order(&bytes)
            })
            .collect();

        // Verify the proof using the privacy manager
        let proof_valid = {
            let manager = privacy_manager.read().await;
            manager.verify_proof(&proof, &public_inputs).await
        };

        if !proof_valid {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Privacy proof verification failed".to_string(),
            ));
        }

        // Extract nullifier from public inputs
        let nullifier =
            crate::privacy::PrivacyManager::extract_nullifier(privacy_tx).ok_or_else(|| {
                crate::error::BlockchainError::InvalidTransaction(
                    "Could not extract nullifier from privacy transaction".to_string(),
                )
            })?;

        // Check if nullifier is already spent (prevent double-spending)
        let nullifier_spent = {
            let manager = privacy_manager.read().await;
            manager.is_nullifier_spent(&nullifier).await
        };

        if nullifier_spent {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Nullifier already spent (double-spend attempt)".to_string(),
            ));
        }

        Ok(())
    }

    /// Process a transaction and update state
    async fn process_transaction(
        &mut self,
        tx: &Transaction,
    ) -> crate::error::BlockchainResult<()> {
        // === REPLAY PROTECTION: Check for duplicate transaction hash ===
        {
            let recent_hashes = self.recent_tx_hashes_read();
            let tx_hash = tx.hash;
            // Check if this tx hash has already been processed
            for block_hashes in recent_hashes.iter() {
                if block_hashes.contains(&tx_hash.0) {
                    return Err(crate::error::BlockchainError::InvalidTransaction(
                        "Transaction already processed (replay detected)".to_string(),
                    ));
                }
            }
        }

        // Handle privacy transactions differently
        #[cfg(feature = "privacy")]
        {
            if let Some(ref privacy_tx) = tx.privacy_data {
                return self.process_privacy_transaction(tx, privacy_tx).await;
            }
        }

        // Handle EVM transactions (deployments + calls)
        let data_not_empty = !tx.data.is_empty();
        let evm_enabled = self.evm_enabled;
        // SEC-010: Track if EVM handled the transaction to avoid dual-state divergence
        let mut evm_handled = false;
        let mut evm_receipt: Option<TxReceipt> = None;
        if data_not_empty && evm_enabled {
            debug!("EVM data detected - executing transaction");
            if let Some(ref executor) = self.evm_executor {
                debug!("EVM executor found, executing...");
                // Get current block context
                let blocks_data = self.blocks_read();
                let block_number = blocks_data
                    .blocks
                    .iter()
                    .map(|b| b.header.block_number)
                    .max()
                    .unwrap_or(0);
                let block_timestamp = if let Some(latest_block) = blocks_data.blocks.last() {
                    latest_block.header.timestamp
                } else {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                };
                drop(blocks_data); // Release read lock early

                // Execute EVM transaction (create or call)
                match executor.execute_transaction(tx, block_number, block_timestamp) {
                    Ok(result) => {
                        debug!("EVM execution result: success={}", result.success);
                        // INSTRUMENTATION: Log EVM execution details
                        debug!("EVM execution completed:");
                        debug!("   Success: {}", result.success);
                        debug!("   Gas used: {}", result.gas_used);
                        debug!("   Output length: {}", result.output.len());

                        // INSTRUMENTATION: Check if this was a storage write transaction
                        if !tx.data.is_empty() && tx.data.len() >= 4 {
                            let selector = &tx.data[0..4];
                            if selector == [0x60, 0xfe, 0x47, 0xb1] {
                                // setValue(uint256)
                                debug!("Detected setValue transaction - checking storage commit");
                                // Verify the storage was actually committed by reading it back
                                let storage_key = [0u8; 32];
                                if let Some(stored_value) =
                                    executor.get_contract_storage(tx.to, &storage_key)
                                {
                                    debug!("Post-execution storage check:");
                                    debug!("   Stored value: 0x{}", hex::encode(&stored_value));
                                    if stored_value.len() >= 32 {
                                        let mut value_bytes = [0u8; 32];
                                        value_bytes.copy_from_slice(&stored_value[0..32]);
                                        let stored_u256 = u128::from_be_bytes([
                                            value_bytes[16],
                                            value_bytes[17],
                                            value_bytes[18],
                                            value_bytes[19],
                                            value_bytes[20],
                                            value_bytes[21],
                                            value_bytes[22],
                                            value_bytes[23],
                                            value_bytes[24],
                                            value_bytes[25],
                                            value_bytes[26],
                                            value_bytes[27],
                                            value_bytes[28],
                                            value_bytes[29],
                                            value_bytes[30],
                                            value_bytes[31],
                                        ]);
                                        debug!("   Interpreted as u128: {}", stored_u256);
                                    }
                                } else {
                                    debug!("Post-execution storage check: NO VALUE FOUND");
                                }
                            }
                        }

                        if !result.success {
                            return Err(crate::error::BlockchainError::Evm(format!(
                                "EVM execution failed: {:?}",
                                result.output
                            )));
                        }
                        // Capture receipt before result is consumed.
                        // For contract deployments (tx.to is zero), output is the deployed address.
                        let contract_addr = if tx.to.is_zero()
                            && !tx.data.is_empty()
                            && result.output.len() >= 20
                        {
                            let mut addr = [0u8; 20];
                            addr.copy_from_slice(&result.output[result.output.len() - 20..]);
                            Some(addr)
                        } else {
                            None
                        };
                        evm_receipt = Some(TxReceipt {
                            success: true,
                            gas_used: result.gas_used,
                            logs: result.logs.clone(),
                            contract_address: contract_addr,
                        });
                        // SEC-010: EVM executed successfully - balance/nonce changes persisted
                        // Set flag to skip native transfer logic and avoid dual-state divergence
                        evm_handled = true;
                        debug!("EVM execution handled balance/nonce updates - skipping native transfer logic");
                    }
                    Err(e) => {
                        debug!("EVM execution error: {}", e);
                        return Err(crate::error::BlockchainError::Evm(format!(
                            "EVM execution error: {}",
                            e
                        )));
                    }
                }
            } else {
                debug!("EVM executor is None!");
            }
        } else {
            debug!("EVM routing skipped (no data or EVM disabled)");
        }

        // SEC-010: Native balance/nonce update logic
        // Skip this for EVM transactions to avoid dual-state divergence
        // EVM already handles: tx.value transfer, internal transfers, nonce increment
        // We still need to handle: gas fee deduction for EVM transactions
        if evm_handled {
            // EVM handled the value transfer and nonce increment
            // But we still need to deduct the gas fee from the sender
            debug!("EVM handled value transfer - processing gas fee deduction only");

            if tx.fee > 0 {
                let sender_balance = self.get_balance(tx.from);
                if sender_balance < tx.fee {
                    return Err(crate::error::BlockchainError::InvalidTransaction(
                        "Insufficient balance for transaction fee after EVM execution".to_string(),
                    ));
                }
                let new_balance = sender_balance - tx.fee;

                // Persist fee deduction
                if let Some(db) = &self.database {
                    use crate::storage::StateStore;
                    let state_store = StateStore::new(db);
                    state_store.put_balance(&tx.from, new_balance)?;
                }

                // Update in-memory cache
                if let Some(ref mut verkle) = self.verkle_state {
                    verkle.set_balance(tx.from, new_balance);
                } else {
                    let current_nonce = self.get_nonce(tx.from);
                    self.accounts.insert(
                        tx.from,
                        AccountState {
                            balance: new_balance,
                            nonce: current_nonce,
                        },
                    );
                }
                debug!(
                    "Deducted gas fee {} from sender after EVM execution",
                    tx.fee
                );
            }

            // Note: tx hash recording and spent output tracking happen at block level
            // in validate_and_process_transactions()

            // Persist the EVM execution receipt
            if let Some(ref receipt) = evm_receipt {
                self.store_receipt(&tx.hash, receipt);
            }
            return Ok(());
        }

        // Handle gasless transactions: sponsor pays fee, sender pays value
        // Handle regular transactions: sender pays both value and fee

        // === ATOMIC BALANCE + NONCE UPDATE ===
        // Compute new nonce upfront for atomic update
        let current_nonce = self.get_nonce(tx.from);
        let new_nonce = current_nonce.saturating_add(1);

        let from_balance = self.get_balance(tx.from);

        // Deduct value from sender (always)
        if from_balance < tx.value {
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Insufficient balance for transaction value".to_string(),
            ));
        }

        let new_from_balance = from_balance - tx.value;

        // Deduct fee from sponsor (if gasless) or sender (if regular)
        if let Some(sponsor) = tx.sponsor {
            // Gasless transaction: sponsor pays fee
            let sponsor_balance = self.get_balance(sponsor);
            if sponsor_balance < tx.fee {
                return Err(crate::error::BlockchainError::InvalidTransaction(
                    "Insufficient sponsor balance for transaction fee".to_string(),
                ));
            }

            let new_sponsor_balance = sponsor_balance - tx.fee;

            // UPDATE 1: Write-through cache - write to database FIRST
            // Persist sponsor balance change FIRST
            if let Some(db) = &self.database {
                use crate::storage::StateStore;
                let state_store = StateStore::new(db);

                // WRITE-THROUGH: Database write comes first
                debug!(
                    "Writing sponsor balance to DB first: {} -> {}",
                    hex::encode(sponsor),
                    new_sponsor_balance
                );
                state_store.put_balance(&sponsor, new_sponsor_balance)?;
            }

            // Update sponsor balance in memory AFTER database write
            if let Some(ref mut verkle) = self.verkle_state {
                verkle.set_balance(sponsor, new_sponsor_balance);
            } else {
                self.accounts.insert(
                    sponsor,
                    AccountState {
                        balance: new_sponsor_balance,
                        nonce: self.get_nonce(sponsor),
                    },
                );
            }

            // For gasless transactions, only value was deducted from sender (fee paid by sponsor)
            // UPDATE 2: Write-through for sender balance
            if let Some(db) = &self.database {
                use crate::storage::StateStore;
                let state_store = StateStore::new(db);

                // WRITE-THROUGH: Database write comes first
                debug!(
                    "Writing sender balance to DB first: {} -> {}",
                    hex::encode(tx.from),
                    new_from_balance
                );
                state_store.put_balance(&tx.from, new_from_balance)?;
            }

            // Update sender balance in memory AFTER database write
            // ATOMIC: Include new nonce with balance update
            if let Some(ref mut verkle) = self.verkle_state {
                verkle.set_balance(tx.from, new_from_balance);
            } else {
                self.accounts.insert(
                    tx.from,
                    AccountState {
                        balance: new_from_balance,
                        nonce: new_nonce, // ATOMIC: Use pre-computed new nonce
                    },
                );
            }
        } else {
            // Regular transaction: sender also pays fee
            if new_from_balance < tx.fee {
                return Err(crate::error::BlockchainError::InvalidTransaction(
                    "Insufficient balance for transaction fee".to_string(),
                ));
            }

            let new_from_balance_after_fee = new_from_balance - tx.fee;

            // UPDATE 3: Write-through for regular transaction sender
            // Write to database FIRST
            if let Some(db) = &self.database {
                use crate::storage::StateStore;
                let state_store = StateStore::new(db);

                // WRITE-THROUGH: Database write comes first
                debug!(
                    "Writing regular tx sender balance to DB first: {} -> {}",
                    hex::encode(tx.from),
                    new_from_balance_after_fee
                );
                state_store.put_balance(&tx.from, new_from_balance_after_fee)?;
            }

            // Update sender balance in memory AFTER database write
            // ATOMIC: Include new nonce with balance update
            if let Some(ref mut verkle) = self.verkle_state {
                verkle.set_balance(tx.from, new_from_balance_after_fee);
            } else {
                self.accounts.insert(
                    tx.from,
                    AccountState {
                        balance: new_from_balance_after_fee,
                        nonce: new_nonce, // ATOMIC: Use pre-computed new nonce
                    },
                );
            }
        }

        // UPDATE 4: Write-through for receiver balance
        if tx.to != Address([0u8; 20]) {
            let new_to_balance = self.get_balance(tx.to) + tx.value;

            // Write to database FIRST
            if let Some(db) = &self.database {
                use crate::storage::StateStore;
                let state_store = StateStore::new(db);

                // WRITE-THROUGH: Database write comes first
                debug!(
                    "Writing receiver balance to DB first: {} -> {}",
                    hex::encode(tx.to),
                    new_to_balance
                );
                state_store.put_balance(&tx.to, new_to_balance)?;
            }

            // Update Verkle tree if enabled (canonical source)
            if let Some(ref mut verkle) = self.verkle_state {
                verkle.set_balance(tx.to, new_to_balance);
                // Don't update in-memory cache when Verkle is enabled
            } else {
                // Verkle not enabled - update in-memory cache AFTER database write
                self.accounts.insert(
                    tx.to,
                    AccountState {
                        balance: new_to_balance,
                        nonce: self.get_nonce(tx.to),
                    },
                );
            }
        }

        // Privacy pool: if the recipient is the pool address, register the commitment.
        // The balance transfer above already credited the pool address; we just need
        // to record the commitment so the depositor can withdraw later.
        if tx.to == crate::privacy_pool::PRIVACY_POOL_ADDRESS && tx.data.len() == 32 {
            let pool = Arc::clone(&self.privacy_pool);
            let mut commitment_bytes = [0u8; 32];
            commitment_bytes.copy_from_slice(&tx.data[0..32]);
            let commitment = crate::types::Hash(commitment_bytes);
            let current_block = self.latest_block_number();
            let mut pool_guard = pool.write().await;
            match pool_guard.deposit(commitment, current_block) {
                Ok(leaf_index) => {
                    info!(
                        commitment = %commitment,
                        leaf_index,
                        "Privacy pool deposit registered"
                    );
                }
                Err(e) => {
                    warn!(
                        commitment = %commitment,
                        error = %e,
                        "Privacy pool deposit rejected (balance still credited)"
                    );
                }
            }
        }

        // Update nonce (transaction was already validated to have correct nonce)
        // For contract wallets, update wallet nonce; for EOA, update account nonce
        if let Some(ref wallet_registry) = self.wallet_registry {
            if let Ok(mut registry) = wallet_registry.try_write() {
                if registry.is_contract_wallet(&tx.from) {
                    // Update wallet nonce
                    if let Err(e) = registry.update_wallet_nonce(&tx.from) {
                        return Err(crate::error::BlockchainError::InvalidTransaction(format!(
                            "Failed to update wallet nonce: {}",
                            e
                        )));
                    }

                    // Update spending limits if applicable
                    if let Some(wallet) = registry.get_wallet_mut(&tx.from) {
                        if wallet.has_spending_limits() {
                            if let Some(ref mut limits) = wallet.config.spending_limits {
                                limits.record_spending(tx.value);
                            }
                        }
                    }
                } else {
                    // Regular EOA: persist nonce to database
                    // NOTE: In-memory nonce was already updated atomically with balance above
                    // WRITE-THROUGH STRATEGY: Write to database FIRST
                    if let Some(db) = &self.database {
                        use crate::storage::StateStore;
                        let state_store = StateStore::new(db);

                        debug!(
                            "Writing nonce to DB first: {} -> {}",
                            hex::encode(tx.from),
                            new_nonce
                        );
                        state_store.put_nonce(&tx.from, new_nonce).map_err(|e| {
                            crate::error::BlockchainError::Storage(format!(
                                "Failed to persist nonce: {}",
                                e
                            ))
                        })?;
                    }

                    // Update Verkle tree if enabled (nonce only - balance was updated above)
                    // NOTE: For non-Verkle mode, nonce was already set atomically with balance
                    if let Some(ref mut verkle) = self.verkle_state {
                        verkle.set_nonce(tx.from, new_nonce);
                    }
                    // Non-Verkle: No separate nonce update needed - already done atomically with balance
                }
            } else {
                // If we can't acquire the lock, fall back to account nonce
                // NOTE: In-memory nonce was already updated atomically with balance above
                // WRITE-THROUGH STRATEGY: Write to database FIRST
                if let Some(db) = &self.database {
                    use crate::storage::StateStore;
                    let state_store = StateStore::new(db);

                    debug!(
                        "Writing fallback nonce to DB first: {} -> {}",
                        hex::encode(tx.from),
                        new_nonce
                    );
                    state_store.put_nonce(&tx.from, new_nonce).map_err(|e| {
                        crate::error::BlockchainError::Storage(format!(
                            "Failed to persist fallback nonce: {}",
                            e
                        ))
                    })?;
                }

                // Update Verkle tree if enabled (nonce only - balance was updated above)
                // NOTE: For non-Verkle mode, nonce was already set atomically with balance
                if let Some(ref mut verkle) = self.verkle_state {
                    verkle.set_nonce(tx.from, new_nonce);
                }
            }
        } else {
            // Fallback: persist nonce to database
            // NOTE: In-memory nonce was already updated atomically with balance above
            // WRITE-THROUGH STRATEGY: Write to database FIRST
            if let Some(db) = &self.database {
                use crate::storage::StateStore;
                let state_store = StateStore::new(db);

                debug!(
                    "Writing final fallback nonce to DB first: {} -> {}",
                    hex::encode(tx.from),
                    new_nonce
                );
                state_store.put_nonce(&tx.from, new_nonce).map_err(|e| {
                    crate::error::BlockchainError::Storage(format!(
                        "Failed to persist final fallback nonce: {}",
                        e
                    ))
                })?;
            }

            // Update Verkle tree if enabled (nonce only - balance was updated above)
            // NOTE: For non-Verkle mode, nonce was already set atomically with balance
            if let Some(ref mut verkle) = self.verkle_state {
                verkle.set_nonce(tx.from, new_nonce);
            }
        }

        // NOTE: Nonce persistence now happens in write-through strategy above
        // This section was moved to ensure database writes happen BEFORE memory updates

        // CONSISTENCY GUARD: Verify state consistency after successful transaction processing
        self.verify_post_transaction_consistency(tx)?;

        // Persist receipt for native transfer (21,000 gas, no logs)
        self.store_receipt(
            &tx.hash,
            &TxReceipt {
                success: true,
                gas_used: 21_000,
                logs: vec![],
                contract_address: None,
            },
        );

        Ok(())
    }

    /// Process privacy transaction
    #[cfg(feature = "privacy")]
    async fn process_privacy_transaction(
        &mut self,
        _tx: &Transaction,
        privacy_tx: &crate::privacy::PrivacyTransaction,
    ) -> crate::error::BlockchainResult<()> {
        // Privacy transactions hide sender, receiver, and amount
        // We only process the nullifier (mark as spent) and commitment (add to tree)

        if let Some(ref privacy_manager) = self.privacy_manager {
            // Extract nullifier from public inputs
            if let Some(nullifier) = crate::privacy::PrivacyManager::extract_nullifier(privacy_tx) {
                // Use tokio runtime to handle async call from sync context
                {
                    // Add nullifier to nullifier set (mark as spent to prevent double-spend)
                    let _ = {
                        let manager = privacy_manager.read().await;
                        manager.add_nullifier(nullifier).await
                    };
                    // Note: If adding nullifier fails, validation already passed
                    // This could indicate a race condition which should be rare
                }
            }

            // Privacy transactions don't update balances directly
            // Balances are managed through commitments and nullifiers
        }

        Ok(())
    }

    /// Get the latest block number (lock-free via atomic cache)
    pub fn latest_block_number(&self) -> u64 {
        self.cached_latest_block_number.load(Ordering::Acquire)
    }

    /// Get the chain ID for EIP-155 replay protection
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Clone of the lock-free accounts map for direct RPC balance/nonce queries.
    /// Callers MUST NOT use this to make decisions that affect consensus — for
    /// authoritative state during block validation use `get_balance` / `get_nonce`
    /// with the write lock already held.
    pub fn accounts_arc(&self) -> Arc<DashMap<Address, AccountState>> {
        Arc::clone(&self.accounts)
    }

    /// Shared handle to the gas sponsorship registry.
    /// RPC server uses this to register/query policies without taking a write lock.
    pub fn sponsor_registry(&self) -> Arc<crate::gas_sponsorship::SponsorRegistry> {
        Arc::clone(&self.sponsor_registry)
    }

    /// Shared handle to the privacy pool.
    pub fn privacy_pool(&self) -> Arc<tokio::sync::RwLock<crate::privacy_pool::PrivacyPool>> {
        Arc::clone(&self.privacy_pool)
    }

    /// Execute a privacy pool withdrawal outside the normal tx pipeline.
    ///
    /// Returns the amount transferred to `recipient` on success.
    pub async fn pool_withdraw(
        &self,
        nullifier: crate::types::Hash,
        recipient: crate::types::Address,
        proof: Option<Vec<u8>>,
    ) -> crate::error::BlockchainResult<u128> {
        let amount = {
            let mut pool = self.privacy_pool.write().await;
            pool.withdraw(nullifier, proof.as_deref()).map_err(|e| {
                crate::error::BlockchainError::InvalidTransaction(format!(
                    "Privacy pool withdrawal failed: {}",
                    e
                ))
            })?
        };

        // Transfer denomination from pool address to recipient
        let pool_addr = crate::privacy_pool::PRIVACY_POOL_ADDRESS;
        let pool_balance = self.get_balance(pool_addr);
        if pool_balance < amount {
            // Re-insert nullifier to undo state change — pool was already updated,
            // so we surface an error to the caller who should discard this result.
            return Err(crate::error::BlockchainError::InvalidTransaction(
                "Privacy pool account balance inconsistent with pool state".to_string(),
            ));
        }
        self.accounts.insert(
            pool_addr,
            AccountState {
                balance: pool_balance - amount,
                nonce: self.get_nonce(pool_addr),
            },
        );
        let recipient_balance = self.get_balance(recipient);
        self.accounts.insert(
            recipient,
            AccountState {
                balance: recipient_balance + amount,
                nonce: self.get_nonce(recipient),
            },
        );

        info!(
            nullifier = %nullifier,
            recipient = %hex::encode(recipient.0),
            amount,
            "Privacy pool withdrawal executed"
        );
        Ok(amount)
    }

    /// Set the chain ID for EIP-155 replay protection
    pub fn set_chain_id(&mut self, chain_id: u64) {
        self.chain_id = chain_id;
    }

    /// Set the ZK verifying key for state transition proof verification
    #[cfg(feature = "privacy")]
    pub fn set_zk_verifying_key(&mut self, vk: Arc<ark_groth16::VerifyingKey<ark_bn254::Bn254>>) {
        self.zk_verifying_key = Some(vk);
    }

    /// Set whether to enforce ZK proof validation (hard rejection on invalid/missing proofs)
    pub fn set_zk_enforce(&mut self, enforce: bool) {
        self.zk_enforce = enforce;
    }

    /// Flush the database to disk (for graceful shutdown)
    /// Returns Ok(()) if flush succeeded or if no database is configured
    pub fn flush_database(&self) -> crate::error::BlockchainResult<()> {
        if let Some(db) = &self.database {
            info!("Flushing blockchain database to disk...");
            db.flush()?;
            info!("Database flush complete");
        }
        Ok(())
    }

    /// Get direct access to the atomic cached block number (for lock-free RPC access)
    pub fn get_cached_block_number_arc(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.cached_latest_block_number)
    }

    /// Get block by hash — O(1) via index.
    pub fn get_block_by_hash(&self, hash: &crate::types::Hash) -> Option<Block> {
        let blocks_data = self.blocks_read();
        if let Some(&idx) = blocks_data.block_by_hash.get(hash) {
            return Some(blocks_data.blocks[idx].clone());
        }
        drop(blocks_data);

        if let Some(db) = &self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            if let Ok(Some(block)) = block_store.get(hash) {
                return Some(block);
            }
        }

        None
    }

    /// Get the latest block (returns owned Block)
    pub fn get_latest_block(&self) -> Option<Block> {
        let blocks_data = self.blocks_read();
        blocks_data.blocks.last().cloned()
    }

    /// Get block by number — O(1) via index.
    pub fn get_block_by_number(&self, number: u64) -> Option<Block> {
        let blocks_data = self.blocks_read();
        blocks_data
            .block_by_number
            .get(&number)
            .map(|&idx| blocks_data.blocks[idx].clone())
    }

    /// Get all blocks (safe to call from both async and sync contexts)
    pub fn get_blocks(&self) -> Vec<Block> {
        let blocks_data = self.blocks_read();
        blocks_data.blocks.clone()
    }

    /// Get block count without cloning (efficient for hot paths)
    pub fn get_block_count(&self) -> usize {
        let blocks_data = self.blocks_read();
        blocks_data.blocks.len()
    }

    /// Check if a block hash exists in the hot-block set (O(1), no clone)
    pub fn has_block_hash(&self, hash: &crate::types::Hash) -> bool {
        self.blocks_read().block_hashes.contains(hash)
    }

    /// Signal that blockchain has finished loading from storage and is ready for sync/mining
    /// Must be called after load_from_storage() completes
    pub fn set_ready(&self) {
        self.blockchain_ready.store(true, Ordering::Release);
    }

    /// Check if blockchain is ready for sync/mining operations
    /// Returns false during initial load_from_storage()
    pub fn is_ready(&self) -> bool {
        self.blockchain_ready.load(Ordering::Acquire)
    }

    /// Get max block number using the atomic cache (true maximum, O(1), no lock).
    /// blocks.last() was NOT used here because GhostDAG commits can arrive out of order,
    /// making the last-pushed block an unreliable height indicator.
    pub fn get_max_block_number(&self) -> u64 {
        self.cached_latest_block_number.load(Ordering::Acquire)
    }

    /// Execute a closure with a reference to the blocks slice (efficient for filtering/iterating)
    /// The read lock is held only for the duration of the closure
    pub fn with_blocks<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[Block]) -> R,
    {
        let blocks_data = self.blocks_read();
        f(&blocks_data.blocks)
    }

    /// Get the last `n` blocks (clones only the tail — never the full chain).
    pub fn get_blocks_tail(&self, n: usize) -> Vec<Block> {
        let blocks_data = self.blocks_read();
        let len = blocks_data.blocks.len();
        let start = len.saturating_sub(n);
        blocks_data.blocks[start..].to_vec()
    }

    /// Get transaction by hash — O(1) via index.
    /// Returns (block, transaction, tx_index_in_block).
    pub fn get_transaction_by_hash(
        &self,
        hash: &crate::types::Hash,
    ) -> Option<(Block, Transaction, usize)> {
        let blocks_data = self.blocks_read();
        let &(block_idx, tx_idx) = blocks_data.tx_by_hash.get(hash)?;
        let block = blocks_data.blocks[block_idx].clone();
        let tx = block.transactions[tx_idx].clone();
        Some((block, tx, tx_idx))
    }

    /// Get the latest block for a specific stream type — O(N) but only called on fee changes.
    pub fn get_latest_block_for_stream(
        &self,
        stream_type: crate::types::StreamType,
    ) -> Option<Block> {
        let blocks_data = self.blocks_read();
        blocks_data
            .blocks
            .iter()
            .filter(|b| b.header.stream_type == stream_type)
            .max_by_key(|b| b.header.block_number)
            .cloned()
    }

    /// Get blocks from a specific index onwards (clones only the tail, not full chain)
    pub fn get_blocks_from(&self, start_index: usize) -> Vec<Block> {
        let blocks_data = self.blocks_read();
        blocks_data.blocks[start_index..].to_vec()
    }

    /// Get blocks from storage with block_number >= from_block, sorted by block_number, limited to count
    ///
    /// This is used by the sync server to serve historical blocks that may not be in the in-memory cache.
    /// It queries the persistent storage (sled) directly to get blocks by number range.
    pub fn get_blocks_from_storage(&self, from_block: u64, count: usize) -> Vec<Block> {
        if let Some(db) = &self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            match block_store.get_blocks_from_number(from_block, count) {
                Ok(blocks) => return blocks,
                Err(e) => {
                    error!("Failed to get blocks from storage: {}", e);
                    return Vec::new();
                }
            }
        }
        // If no storage, fall back to in-memory (shouldn't happen in production)
        let blocks_data = self.blocks_read();
        blocks_data
            .blocks
            .iter()
            .filter(|b| b.header.block_number >= from_block)
            .take(count)
            .cloned()
            .collect()
    }

    /// Check if persistent storage (sled) has any blocks
    ///
    /// This is used during genesis creation to prevent recreating genesis
    /// when in-memory blocks are empty but sled still has data.
    /// Returns true if sled storage exists and contains at least one block.
    pub fn has_blocks_in_storage(&self) -> bool {
        if let Some(db) = &self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            match block_store.get_all_blocks() {
                Ok(blocks) => return !blocks.is_empty(),
                Err(e) => {
                    error!("Failed to check blocks in storage: {}", e);
                    return false;
                }
            }
        }
        false
    }

    /// Get the number of blocks in persistent storage (sled)
    ///
    /// This is used during fork detection to verify that sled storage
    /// has finished loading before clearing the chain for resync.
    /// Returns 0 if sled storage doesn't exist or on error.
    pub fn get_storage_block_count(&self) -> usize {
        if let Some(db) = &self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            match block_store.get_all_blocks() {
                Ok(blocks) => return blocks.len(),
                Err(e) => {
                    error!("Failed to count blocks in storage: {}", e);
                    return 0;
                }
            }
        }
        0
    }

    /// Get transaction count (synchronous, uses block_in_place)
    pub fn transaction_count(&self) -> usize {
        let blocks_data = self.blocks_read();
        blocks_data
            .blocks
            .iter()
            .map(|b| b.transactions.len())
            .sum()
    }

    /// Set balance for an address (uses DashMap for lock-free writes)
    pub fn set_balance<A: Into<Address>>(
        &mut self,
        address: A,
        balance: u128,
    ) -> crate::error::BlockchainResult<()> {
        let address = address.into();
        // If Verkle is enabled, it is the canonical source - update it first
        if let Some(ref mut verkle) = self.verkle_state {
            verkle.set_balance(address, balance);
        } else {
            // Verkle not enabled - use DashMap
            // Get existing nonce or default to 0
            let nonce = self.accounts.get(&address).map(|a| a.nonce).unwrap_or(0);
            self.accounts
                .insert(address, AccountState { balance, nonce });
        }

        // Persist balance (for recovery and non-Verkle mode)
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            state_store.put_balance(&address, balance)?;
        }

        Ok(())
    }

    /// Apply genesis allocations from config (only on fresh chain with block height 0)
    /// Each allocation is a hex address and balance in base units
    /// Allocations must be sorted by address for deterministic state hash
    pub fn apply_genesis_allocations(
        &mut self,
        allocations: &[crate::types::GenesisAllocation],
    ) -> Result<(), String> {
        // Track total allocation for supply validation
        let mut total_allocation: u128 = 0;

        for alloc in allocations {
            // Use the validation method from GenesisAllocation
            if let Err(e) = alloc.validate() {
                return Err(format!("Validation failed for '{}': {}", alloc.address, e));
            }

            // Decode hex address
            let address = alloc
                .address_bytes()
                .map_err(|e| format!("Failed to parse address '{}': {}", alloc.address, e))?;

            // Balance is already u128, no parsing needed
            let balance: u128 = alloc.balance;

            // Accumulate total allocation
            total_allocation = total_allocation
                .checked_add(balance)
                .ok_or_else(|| "Total allocation overflow".to_string())?;

            // Set the balance using existing method
            self.set_balance(address, balance)
                .map_err(|e| format!("Failed to set genesis balance: {}", e))?;

            // Log with IDAG formatting (18 decimals)
            let idag_balance = balance / 1_000_000_000_000_000_000u128;
            info!(
                "Genesis allocation: 0x{} -> {} IDAG",
                alloc.normalized_address(),
                idag_balance
            );
        }

        // Log total allocation
        let total_idag = total_allocation / 1_000_000_000_000_000_000u128;
        info!(
            "Total genesis allocation: {} IDAG to {} address(es)",
            total_idag,
            allocations.len()
        );

        // Compute and log genesis state hash for verification
        let state_hash = self.compute_genesis_state_hash(allocations);
        info!("Genesis state hash: 0x{}", hex::encode(state_hash));

        Ok(())
    }

    /// Compute a deterministic hash of the genesis state for verification
    /// This ensures all nodes with the same allocations produce the same state hash
    fn compute_genesis_state_hash(
        &self,
        allocations: &[crate::types::GenesisAllocation],
    ) -> [u8; 32] {
        use crate::types::keccak256;

        let mut hasher_data = Vec::new();

        // Include number of allocations
        hasher_data.extend_from_slice(&allocations.len().to_le_bytes());

        // Include each allocation (address and balance)
        // Allocations should already be sorted for determinism
        for alloc in allocations {
            let addr_bytes = alloc.address_bytes().unwrap_or([0u8; 20].into());
            hasher_data.extend_from_slice(&addr_bytes);
            hasher_data.extend_from_slice(&alloc.balance.to_le_bytes());
        }

        keccak256(&hasher_data).0
    }

    /// Set nonce for an address
    /// Set privacy manager
    #[cfg(feature = "privacy")]
    pub fn set_privacy_manager(
        &mut self,
        manager: Arc<tokio::sync::RwLock<crate::privacy::PrivacyManager>>,
    ) {
        self.privacy_manager = Some(manager);
    }

    /// Set nonce for an address (uses DashMap for lock-free writes)
    pub fn set_nonce(
        &mut self,
        address: Address,
        nonce: u64,
    ) -> crate::error::BlockchainResult<()> {
        // If Verkle is enabled, it is the canonical source - update it first
        if let Some(ref mut verkle) = self.verkle_state {
            verkle.set_nonce(address, nonce);
        } else {
            // Verkle not enabled - use DashMap
            // Get existing balance or default to 0
            let balance = self.accounts.get(&address).map(|a| a.balance).unwrap_or(0);
            self.accounts
                .insert(address, AccountState { balance, nonce });
        }

        // Persist nonce (for recovery and non-Verkle mode)
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            state_store.put_nonce(&address, nonce)?;
        }

        Ok(())
    }

    /// Get balance for an address (CACHE POLICY UPDATE)
    /// NEW: Prefer database as authoritative source, cache in memory
    /// This method can be called by unlimited concurrent readers without any locking!
    pub fn get_balance<A: Into<Address>>(&self, address: A) -> u128 {
        let address = address.into();
        // If Verkle is enabled, it is the canonical source of truth
        if let Some(ref verkle) = self.verkle_state {
            return verkle.get_balance(address);
        }

        // NEW CACHE POLICY: Prefer database as authoritative source
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(balance)) = state_store.get_balance(&address) {
                // Update cache for performance (but database is source of truth)
                if let Some(mut account) = self.accounts.get_mut(&address) {
                    account.balance = balance;
                } else {
                    // Insert new cache entry
                    self.accounts.insert(
                        address,
                        AccountState {
                            balance,
                            nonce: self.get_nonce(address),
                        },
                    );
                }
                return balance;
            }
        }

        // FALLBACK: Check in-memory cache if database unavailable
        if let Some(account) = self.accounts.get(&address) {
            return account.balance;
        }

        0
    }

    /// Get nonce for an address (CACHE POLICY UPDATE)
    /// NEW: Prefer database as authoritative source, cache in memory
    pub fn get_nonce<A: Into<Address>>(&self, address: A) -> u64 {
        let address = address.into();
        // If Verkle is enabled, it is the canonical source of truth
        if let Some(ref verkle) = self.verkle_state {
            return verkle.get_nonce(address);
        }

        // NEW CACHE POLICY: Prefer database as authoritative source
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(nonce)) = state_store.get_nonce(&address) {
                // Update cache for performance (but database is source of truth)
                if let Some(mut account) = self.accounts.get_mut(&address) {
                    account.nonce = nonce;
                } else {
                    // Insert new cache entry
                    let balance = state_store
                        .get_balance(&address)
                        .ok()
                        .flatten()
                        .unwrap_or(0);
                    self.accounts
                        .insert(address, AccountState { balance, nonce });
                }
                return nonce;
            }
        }

        // FALLBACK: Check in-memory cache if database unavailable
        if let Some(account) = self.accounts.get(&address) {
            return account.nonce;
        }

        0
    }

    /// Get total fees burned (cumulative)
    /// Returns the total amount of IDAG that has been removed from circulation via the 50% fee burn mechanism.
    pub fn get_total_fees_burned(&self) -> u128 {
        // Use blocking_read since this is a sync method
        *self.total_fees_burned.blocking_read()
    }

    /// Add burned fees to the cumulative total
    /// This is called by the mining module when a block is finalized with fees.
    pub async fn add_burned_fees(&self, amount: u128) {
        let mut total = self.total_fees_burned.write().await;
        *total = total.saturating_add(amount);
    }

    /// Get all account states for snapshot
    /// Returns a map of address to (balance, nonce)
    pub fn get_all_accounts(&self) -> std::collections::HashMap<Address, (u128, u64)> {
        let mut result = std::collections::HashMap::new();

        // If Verkle is enabled, get from Verkle state
        if let Some(ref verkle) = self.verkle_state {
            for (addr, (balance, nonce)) in verkle.get_all_accounts() {
                result.insert(addr, (balance, nonce));
            }
            return result;
        }

        // Otherwise, iterate through DashMap (in-memory accounts)
        for entry in self.accounts.iter() {
            result.insert(*entry.key(), (entry.value().balance, entry.value().nonce));
        }

        // Also check database for accounts not in cache
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);

            // Scan database for balance: prefixed keys
            for (key, _) in db.scan_prefix("balance:") {
                let key_str = String::from_utf8_lossy(&key);
                if let Some(addr_hex) = key_str.strip_prefix("balance:") {
                    if let Ok(addr_bytes) = hex::decode(addr_hex) {
                        if addr_bytes.len() == 20 {
                            let mut addr_arr = [0u8; 20];
                            addr_arr.copy_from_slice(&addr_bytes);
                            let addr = Address(addr_arr);
                            if !result.contains_key(&addr) {
                                let balance =
                                    state_store.get_balance(&addr).ok().flatten().unwrap_or(0);
                                let nonce =
                                    state_store.get_nonce(&addr).ok().flatten().unwrap_or(0);
                                result.insert(addr, (balance, nonce));
                            }
                        }
                    }
                }
            }
        }

        result
    }

    pub fn evm_executor(&self) -> Option<&crate::evm::EvmTransactionExecutor> {
        self.evm_executor.as_ref()
    }

    /// Get GhostDAG consensus engine (as Arc clone for sharing)
    pub fn ghostdag(&self) -> Option<Arc<RwLock<GhostDAG>>> {
        self.ghostdag.clone()
    }

    /// Set GhostDAG consensus engine (for sharing with MiningManager)
    pub fn set_ghostdag(&mut self, ghostdag: Arc<RwLock<GhostDAG>>) {
        self.ghostdag = Some(ghostdag);
    }

    /// Get blocks in consensus order (from GhostDAG)
    pub fn get_ordered_blocks(&self) -> crate::error::BlockchainResult<Vec<Block>> {
        if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.get_ordered_blocks()
        } else {
            Ok(Vec::new())
        }
    }

    /// Get DAG statistics for RPC / dashboards.
    ///
    /// **Totals** (`total_blocks`, transaction counts, averages) come from the canonical
    /// in-memory chain (`blocks`), not GhostDAG's hot cache (~1000 blocks). Otherwise
    /// `irondag_getDagStats` appears frozen once the hot cache is full.
    ///
    /// **Blue / red** counts still reflect GhostDAG's current consensus state (may be
    /// windowed after checkpoint pruning; they need not sum to `total_blocks`).
    pub fn get_dag_stats(&self) -> crate::consensus::DAGStats {
        // O(1): read cached atomics instead of iterating all blocks.
        // cached_total_tx_count is updated on every block push.
        let chain_total = self.blocks_read().blocks.len();
        let chain_txs = self.cached_total_tx_count.load(Ordering::Relaxed) as usize;
        let chain_size = chain_total * std::mem::size_of::<Block>()
            + chain_txs * std::mem::size_of::<crate::blockchain::Transaction>();

        let mut stats = if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.get_stats()
        } else {
            crate::consensus::DAGStats {
                total_blocks: 0,
                blue_blocks: 0,
                red_blocks: 0,
                total_transactions: 0,
                total_size_bytes: 0,
                avg_block_size: 0,
                avg_txs_per_block: 0.0,
            }
        };

        stats.total_blocks = chain_total;
        stats.total_transactions = chain_txs;
        stats.total_size_bytes = chain_size;
        stats.avg_block_size = chain_size.checked_div(chain_total).unwrap_or(0);
        stats.avg_txs_per_block = if chain_total == 0 {
            0.0
        } else {
            chain_txs as f64 / chain_total as f64
        };
        stats
    }

    /// Get transactions per second
    pub fn get_tps(&self, duration_seconds: u64) -> f64 {
        if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.get_tps(duration_seconds)
        } else {
            0.0
        }
    }

    /// Check if block is in blue set (consensus selected)
    pub fn is_blue_block(&self, hash: &crate::types::Hash) -> bool {
        if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.is_blue(hash)
        } else {
            false
        }
    }

    /// Block number of the finalized tip (GHOST; for RPC "finalized" / "safe" block tag).
    pub fn get_finalized_block_number(&self) -> crate::error::BlockchainResult<Option<u64>> {
        if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.get_finalized_block_number()
        } else {
            Ok(None)
        }
    }

    /// Hash of the finalized tip block.
    pub fn get_finalized_block_hash(
        &self,
    ) -> crate::error::BlockchainResult<Option<crate::types::Hash>> {
        if let Some(ref ghostdag) = self.ghostdag {
            let dag = if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if matches!(
                    handle.runtime_flavor(),
                    tokio::runtime::RuntimeFlavor::MultiThread
                ) {
                    tokio::task::block_in_place(|| ghostdag.blocking_read())
                } else {
                    ghostdag.blocking_read()
                }
            } else {
                ghostdag.blocking_read()
            };
            dag.get_finalized_block_hash()
        } else {
            Ok(None)
        }
    }

    /// Get a reference to the database (for sled-level block deletion during pruning)
    pub fn database(&self) -> Option<Arc<Database>> {
        self.database.clone()
    }

    /// Delete a block from sled storage by hash (for red block pruning)
    /// Returns Ok(true) if block was deleted, Ok(false) if not found
    pub fn delete_block_from_storage(
        &self,
        hash: &crate::types::Hash,
    ) -> crate::error::BlockchainResult<bool> {
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            block_store.delete_block(hash)
        } else {
            Ok(false)
        }
    }

    /// Get state root (Verkle tree root hash)
    pub fn state_root(&self) -> Option<crate::types::Hash> {
        self.verkle_state.as_ref().map(|v| v.state_root())
    }

    /// Get balance with proof (for light clients)
    pub fn get_balance_with_proof(
        &self,
        address: Address,
    ) -> Option<(u128, crate::verkle::StateProof)> {
        self.verkle_state.as_ref().and_then(|verkle| {
            let (balance, proof, root) = verkle.get_balance_with_proof(address);
            let mut value = Vec::with_capacity(24);
            value.extend_from_slice(&balance.to_le_bytes());
            value.extend_from_slice(&verkle.get_nonce(address).to_le_bytes());
            Some((
                balance,
                crate::verkle::StateProof::new(address, value, proof, root),
            ))
        })
    }

    /// Get nonce with proof (for light clients)
    pub fn get_nonce_with_proof(
        &self,
        address: Address,
    ) -> Option<(u64, crate::verkle::StateProof)> {
        self.verkle_state.as_ref().and_then(|verkle| {
            let (nonce, proof, root) = verkle.get_nonce_with_proof(address);
            let mut value = Vec::with_capacity(24);
            value.extend_from_slice(&verkle.get_balance(address).to_le_bytes());
            value.extend_from_slice(&nonce.to_le_bytes());
            Some((
                nonce,
                crate::verkle::StateProof::new(address, value, proof, root),
            ))
        })
    }

    /// Check if Verkle tree is enabled
    pub fn is_verkle_enabled(&self) -> bool {
        self.verkle_state.is_some()
    }

    /// CONSISTENCY GUARD: Verify state consistency after transaction processing
    /// This method checks that database and memory state are consistent
    fn verify_post_transaction_consistency(
        &mut self,
        tx: &Transaction,
    ) -> crate::error::BlockchainResult<()> {
        // Only verify if we have database connection
        if let Some(db) = &self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);

            debug!(
                "Verifying post-transaction state consistency for tx: 0x{}",
                hex::encode(tx.hash)
            );

            // Verify sender balance consistency
            let db_balance_from = state_store
                .get_balance(&tx.from)
                .map_err(|e| {
                    crate::error::BlockchainError::Storage(format!(
                        "Failed to read sender balance from DB: {}",
                        e
                    ))
                })?
                .unwrap_or(0);
            let mem_balance_from = self.get_balance(tx.from);

            if db_balance_from != mem_balance_from {
                error!("BALANCE MISMATCH for sender {}:", hex::encode(tx.from));
                error!("   Database: {}", db_balance_from);
                error!("   Memory:   {}", mem_balance_from);
                return Err(crate::error::BlockchainError::Storage(format!(
                    "Balance mismatch for sender {}: DB={} MEM={}",
                    hex::encode(tx.from),
                    db_balance_from,
                    mem_balance_from
                )));
            }

            // Verify sender nonce consistency
            let db_nonce_from = state_store
                .get_nonce(&tx.from)
                .map_err(|e| {
                    crate::error::BlockchainError::Storage(format!(
                        "Failed to read sender nonce from DB: {}",
                        e
                    ))
                })?
                .unwrap_or(0);
            let mem_nonce_from = self.get_nonce(tx.from);

            if db_nonce_from != mem_nonce_from {
                error!("NONCE MISMATCH for sender {}:", hex::encode(tx.from));
                error!("   Database: {}", db_nonce_from);
                error!("   Memory:   {}", mem_nonce_from);
                return Err(crate::error::BlockchainError::Storage(format!(
                    "Nonce mismatch for sender {}: DB={} MEM={}",
                    hex::encode(tx.from),
                    db_nonce_from,
                    mem_nonce_from
                )));
            }

            // Verify receiver balance consistency (if not zero address)
            if tx.to != Address([0u8; 20]) {
                let db_balance_to = state_store
                    .get_balance(&tx.to)
                    .map_err(|e| {
                        crate::error::BlockchainError::Storage(format!(
                            "Failed to read receiver balance from DB: {}",
                            e
                        ))
                    })?
                    .unwrap_or(0);
                let mem_balance_to = self.get_balance(tx.to);

                if db_balance_to != mem_balance_to {
                    error!("BALANCE MISMATCH for receiver {}:", hex::encode(tx.to));
                    error!("   Database: {}", db_balance_to);
                    error!("   Memory:   {}", mem_balance_to);
                    return Err(crate::error::BlockchainError::Storage(format!(
                        "Balance mismatch for receiver {}: DB={} MEM={}",
                        hex::encode(tx.to),
                        db_balance_to,
                        mem_balance_to
                    )));
                }
            }

            // For EVM transactions with setValue calls, verify storage consistency
            if !tx.data.is_empty() && tx.data.len() >= 4 {
                let selector = &tx.data[0..4];
                if selector == [0x60, 0xfe, 0x47, 0xb1] {
                    // setValue(uint256)
                    debug!("Checking storage consistency for setValue transaction");

                    if let Some(ref executor) = self.evm_executor {
                        let storage_key = [0u8; 32];
                        if let Some(stored_value) =
                            executor.get_contract_storage(tx.to, &storage_key)
                        {
                            debug!("Storage value found: 0x{}", hex::encode(&stored_value));
                            if stored_value.len() >= 32 {
                                let mut value_bytes = [0u8; 32];
                                value_bytes.copy_from_slice(&stored_value[0..32]);
                                let stored_u256 = u128::from_be_bytes([
                                    value_bytes[16],
                                    value_bytes[17],
                                    value_bytes[18],
                                    value_bytes[19],
                                    value_bytes[20],
                                    value_bytes[21],
                                    value_bytes[22],
                                    value_bytes[23],
                                    value_bytes[24],
                                    value_bytes[25],
                                    value_bytes[26],
                                    value_bytes[27],
                                    value_bytes[28],
                                    value_bytes[29],
                                    value_bytes[30],
                                    value_bytes[31],
                                ]);
                                debug!("Interpreted storage value: {}", stored_u256);
                            }
                        } else {
                            debug!("No storage value found for contract {}", hex::encode(tx.to));
                        }
                    }
                }
            }

            debug!("All state consistency checks PASSED");
        }

        Ok(())
    }

    /// Persist a transaction receipt to sled (keyed by tx hash).
    /// Called after each transaction is processed in a block.
    fn store_receipt(&self, tx_hash: &crate::types::Hash, receipt: &TxReceipt) {
        if let Some(db) = &self.database {
            match bincode::serialize(receipt) {
                Ok(encoded) => {
                    let key = make_key(crate::storage::key_prefix::RECEIPT, &tx_hash.0);
                    if let Err(e) = db.insert_raw(key, encoded) {
                        warn!(
                            "Failed to persist receipt for tx 0x{}: {}",
                            hex::encode(tx_hash),
                            e
                        );
                    }
                }
                Err(e) => warn!(
                    "Failed to serialize receipt for tx 0x{}: {}",
                    hex::encode(tx_hash),
                    e
                ),
            }
        }
    }

    /// Retrieve a persisted transaction receipt from sled.
    pub fn get_tx_receipt(&self, tx_hash: &crate::types::Hash) -> Option<TxReceipt> {
        let db = self.database.as_ref()?;
        let key = make_key(crate::storage::key_prefix::RECEIPT, &tx_hash.0);
        match db.get_raw(&key) {
            Ok(Some(bytes)) => bincode::deserialize::<TxReceipt>(&bytes).ok(),
            _ => None,
        }
    }
}
