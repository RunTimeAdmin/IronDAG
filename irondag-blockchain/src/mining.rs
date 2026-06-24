//! BraidCore Mining Architecture
//!
//! Implements three parallel mining streams:
//! - Stream A: ASIC mining (Blake3), 10s blocks, 10,000 txs/block, 50 IDAG reward
//! - Stream B: CPU mining (B3MemHash), 5s blocks, 5,000 txs/block, 25 IDAG reward
//!   (GPU mining via OpenCL is planned but not yet implemented)
//! - Stream C: ZK proofs, 100ms blocks, 1,000 txs/block, 0 IDAG (fee-based only)
//!
//! EIP-1559: Dynamic base fee mechanism is implemented for fee market optimization.
//! Transactions are ordered by effective priority fee (tip) after base fee is deducted.

pub mod fairness;
pub mod ordering;

use crate::blockchain::{Block, BlockHeader, Blockchain, Transaction, MAX_BLOCK_SIZE};
use crate::consensus::GhostDAG;
use crate::pow;
use crate::sharding::ShardManager;
use crate::types::{Address, Hash, StreamType};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, Duration, Instant};
// QUA-004: Removed unused SegQueue import - pools now use RwLock<Vec>
use tracing::{debug, error, info, warn};

#[cfg(feature = "privacy")]
use crate::zk::{prove_state_transition, StateTransitionCircuit, VerklePathWitness};
#[cfg(feature = "privacy")]
use ark_bn254::Fr;
#[cfg(feature = "privacy")]
use ark_ff::PrimeField;
#[cfg(feature = "privacy")]
use sha3::{Digest, Keccak256};

/// Block rewards for each stream (in base units, 1 IDAG = 1_000_000_000_000_000_000 base units)
pub const STREAM_A_REWARD: u128 = 50_000_000_000_000_000_000; // 50 IDAG
pub const STREAM_B_REWARD: u128 = 25_000_000_000_000_000_000; // 25 IDAG
pub const STREAM_C_REWARD: u128 = 0; // Fee-based only

/// Halving interval: approximately 4 years at 10-second average block time
/// 4 years * 365 days * 24 hours * 3600 seconds / 10 seconds = 12,614,400 blocks
pub const HALVING_INTERVAL: u64 = 12_614_400;

/// Maximum supply: 10 billion IDAG (in base units, 18 decimals)
pub const MAX_SUPPLY: u128 = 10_000_000_000_000_000_000_000_000_000;

/// Calculate block reward based on height and halving schedule
pub fn get_block_reward(block_height: u64, stream: StreamType) -> u128 {
    let era = block_height / HALVING_INTERVAL;
    let base_reward = match stream {
        StreamType::StreamA => STREAM_A_REWARD,
        StreamType::StreamB => STREAM_B_REWARD,
        StreamType::StreamC => return 0, // Fee-only stream
    };
    if era >= 64 {
        return 0; // Reward exhausted after 64 halvings (~256 years)
    }
    base_reward >> era
}

/// Maximum transactions per block for each stream
pub const STREAM_A_MAX_TXS: usize = 10_000;
pub const STREAM_B_MAX_TXS: usize = 5_000;
pub const STREAM_C_MAX_TXS: usize = 1_000;

/// Block times for each stream
pub const STREAM_A_BLOCK_TIME: Duration = Duration::from_secs(10);
/// Stream B: use pow::STREAM_B_TARGET_TIME (5s) for sleep in mine_stream_b
pub const STREAM_B_BLOCK_TIME: Duration = Duration::from_secs(5);
/// Stream C: 1s (was 100ms) to reduce lock churn and CPU on 4-core nodes
pub const STREAM_C_BLOCK_TIME: Duration = Duration::from_secs(1);

/// Maximum transaction pool size (DoS protection - prevents memory exhaustion)
/// When limit is reached, oldest transactions are evicted (FIFO)
pub const MAX_TX_POOL_SIZE: usize = 100_000; // 100k transactions max

/// Per-stream hard caps (must sum to <= MAX_TX_POOL_SIZE)
/// These prevent any single stream from monopolizing the pool
pub const MAX_STREAM_A_POOL_SIZE: usize = 60_000; // 60% of global
pub const MAX_STREAM_B_POOL_SIZE: usize = 30_000; // 30% of global
pub const MAX_STREAM_C_POOL_SIZE: usize = 10_000; // 10% of global

/// ERR-005: Transaction time-to-live in seconds (10 minutes)
/// Transactions older than this are expired and removed from the pool
const TX_TTL_SECS: u64 = 600;

// =============================================================================
// EIP-1559: Dynamic Base Fee Mechanism
// =============================================================================

/// Initial base fee at genesis (1 Gwei = 1_000_000_000 base units)
pub const BASE_FEE_INITIAL: u128 = 1_000_000_000;

/// Maximum change denominator: base fee can change by at most 12.5% per block
pub const BASE_FEE_MAX_CHANGE_DENOMINATOR: u128 = 8;

/// Elasticity multiplier: target gas usage is 50% of gas limit
pub const ELASTICITY_MULTIPLIER: u64 = 2;

/// Gas limits per stream (for EIP-1559 gas target calculations)
pub const STREAM_A_GAS_LIMIT: u64 = 30_000_000; // 30M gas per block
pub const STREAM_B_GAS_LIMIT: u64 = 15_000_000; // 15M gas per block
pub const STREAM_C_GAS_LIMIT: u64 = 3_000_000; // 3M gas per block

/// Calculate the base fee for the next block based on parent block gas usage.
///
/// This implements the EIP-1559 base fee adjustment formula:
/// - If gas used > target: increase base fee (up to 12.5% max)
/// - If gas used < target: decrease base fee (up to 12.5% max)
/// - If gas used == target: base fee stays the same
///
/// # Arguments
/// * `parent_base_fee` - The base fee from the parent block
/// * `parent_gas_used` - The actual gas used in the parent block
/// * `parent_gas_limit` - The gas limit of the parent block
///
/// # Returns
/// The calculated base fee for the next block
pub fn calculate_base_fee(
    parent_base_fee: u128,
    parent_gas_used: u64,
    parent_gas_limit: u64,
) -> u128 {
    let parent_gas_target = parent_gas_limit / ELASTICITY_MULTIPLIER;

    if parent_gas_used == parent_gas_target {
        return parent_base_fee;
    }

    if parent_gas_used > parent_gas_target {
        // Increase base fee
        let gas_used_delta = parent_gas_used - parent_gas_target;
        let base_fee_delta = std::cmp::max(
            parent_base_fee * gas_used_delta as u128
                / parent_gas_target as u128
                / BASE_FEE_MAX_CHANGE_DENOMINATOR,
            1,
        );
        parent_base_fee + base_fee_delta
    } else {
        // Decrease base fee
        let gas_used_delta = parent_gas_target - parent_gas_used;
        let base_fee_delta = parent_base_fee * gas_used_delta as u128
            / parent_gas_target as u128
            / BASE_FEE_MAX_CHANGE_DENOMINATOR;
        parent_base_fee.saturating_sub(base_fee_delta)
    }
}

/// Calculate effective priority fee (tip) for a transaction given the base fee.
///
/// For EIP-1559 transactions:
/// - effective_tip = min(max_priority_fee, max_fee_per_gas - base_fee)
///
/// For legacy transactions:
/// - effective_tip = gas_price - base_fee (saturating)
///
/// # Arguments
/// * `tx` - The transaction
/// * `base_fee` - The current base fee per gas
///
/// # Returns
/// The effective priority fee (tip) that goes to the miner
pub fn calculate_effective_tip(tx: &Transaction, base_fee: u128) -> u128 {
    if let Some(max_fee) = tx.max_fee_per_gas {
        // EIP-1559 transaction
        let max_priority = tx.max_priority_fee_per_gas.unwrap_or(0);
        // Tip is capped by what the user is willing to pay above base fee
        let available_tip = max_fee.saturating_sub(base_fee);
        std::cmp::min(max_priority, available_tip)
    } else {
        // Legacy transaction: fee = gas_price * gas_limit
        // So gas_price = fee / gas_limit
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / tx.gas_limit as u128
        } else {
            0
        };
        gas_price.saturating_sub(base_fee)
    }
}

/// Calculate the total fee (base + priority) for a transaction.
///
/// # Arguments
/// * `tx` - The transaction
/// * `base_fee` - The current base fee per gas
///
/// # Returns
/// The total fee to be paid by the transaction sender
pub fn calculate_total_fee(tx: &Transaction, base_fee: u128) -> u128 {
    if let Some(max_fee) = tx.max_fee_per_gas {
        // EIP-1559: fee = min(max_fee_per_gas, base_fee + max_priority_fee) * gas_used
        // For mempool purposes, we use the full gas_limit
        let priority_fee = tx.max_priority_fee_per_gas.unwrap_or(0);
        let effective_gas_price = std::cmp::min(max_fee, base_fee + priority_fee);
        effective_gas_price * tx.gas_limit as u128
    } else {
        // Legacy: fee is already calculated
        tx.fee
    }
}

/// Check if a transaction can afford the current base fee.
///
/// # Arguments
/// * `tx` - The transaction to validate
/// * `base_fee` - The current base fee per gas
///
/// # Returns
/// `true` if the transaction can pay the base fee, `false` otherwise
pub fn can_afford_base_fee(tx: &Transaction, base_fee: u128) -> bool {
    if let Some(max_fee) = tx.max_fee_per_gas {
        // EIP-1559: max_fee_per_gas must be >= base_fee
        max_fee >= base_fee
    } else {
        // Legacy: gas_price (fee / gas_limit) must be >= base_fee
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / tx.gas_limit as u128
        } else {
            0
        };
        gas_price >= base_fee
    }
}

/// QUA-009: Maximum nonce gap for future-nonce transactions
/// Transactions with nonce > current_nonce + this limit are rejected
const MAX_FUTURE_NONCE_GAP: u64 = 16;

/// QUA-009: Maximum number of future-nonce transactions per sender
/// Prevents memory abuse from a single sender
const MAX_FUTURE_TXS_PER_SENDER: usize = 16;

/// QUA-009: TTL for future-nonce transactions in seconds (5 minutes)
/// Future transactions older than this are evicted
const FUTURE_TX_TTL_SECS: u64 = 300;

/// ARC-008: Shared helper to trim block transactions to fit within MAX_BLOCK_SIZE
///
/// This function efficiently calculates the maximum number of transactions that can fit
/// in a block without exceeding MAX_BLOCK_SIZE. It uses a binary search approach
/// instead of the previous O(n^2) loop that removed transactions one at a time.
///
/// # Arguments
/// * `header_template` - The block header template (used for serialization)
/// * `transactions` - The full list of candidate transactions
/// * `stream_name` - Name of the stream for logging (e.g., "Stream A")
///
/// # Returns
/// A Vec of transactions that fit within the block size limit
fn trim_block_transactions(
    header_template: &crate::blockchain::BlockHeader,
    transactions: &[Transaction],
    stream_name: &str,
) -> Vec<Transaction> {
    if transactions.is_empty() {
        return Vec::new();
    }

    // Try to serialize with all transactions first
    let temp_block = crate::blockchain::Block::new(header_template.clone(), transactions.to_vec());
    match bincode::serialize(&temp_block) {
        Ok(bytes) if bytes.len() <= MAX_BLOCK_SIZE => {
            // All transactions fit, return the full list
            return transactions.to_vec();
        }
        Ok(bytes) => {
            // Need to trim - use binary search to find the right size efficiently
            tracing::warn!(
                "{}: Assembled block exceeds MAX_BLOCK_SIZE ({} > {}), trimming transactions",
                stream_name,
                bytes.len(),
                MAX_BLOCK_SIZE
            );

            // Estimate average tx size from the full block
            let avg_tx_size = bytes.len() / transactions.len();
            let header_size = bytes.len() - (avg_tx_size * transactions.len());
            let available_space = MAX_BLOCK_SIZE.saturating_sub(header_size);
            let estimated_count = (available_space / avg_tx_size.max(1)).max(1);

            // Binary search for the exact count that fits
            let mut low = 1usize;
            let mut high = estimated_count.min(transactions.len());
            let mut best_fit = low;

            while low <= high {
                let mid = (low + high) / 2;
                let test_block = crate::blockchain::Block::new(
                    header_template.clone(),
                    transactions[..mid].to_vec(),
                );

                match bincode::serialize(&test_block) {
                    Ok(test_bytes) if test_bytes.len() <= MAX_BLOCK_SIZE => {
                        best_fit = mid;
                        low = mid + 1; // Try to fit more
                    }
                    Ok(_) => {
                        high = mid.saturating_sub(1); // Too big, try fewer
                    }
                    Err(e) => {
                        tracing::error!("{}: Failed to serialize test block: {}", stream_name, e);
                        break;
                    }
                }
            }

            let trimmed = transactions[..best_fit].to_vec();
            let final_block =
                crate::blockchain::Block::new(header_template.clone(), trimmed.clone());
            if let Ok(final_bytes) = bincode::serialize(&final_block) {
                tracing::info!(
                    "{}: Trimmed block to {} transactions ({} bytes)",
                    stream_name,
                    trimmed.len(),
                    final_bytes.len()
                );
            }
            trimmed
        }
        Err(e) => {
            // Serialization failed, fall back to half
            tracing::error!(
                "{}: Failed to serialize block for size check: {}",
                stream_name,
                e
            );
            let half_count = transactions.len() / 2;
            let fallback = transactions[..half_count.max(1)].to_vec();
            tracing::warn!(
                "{}: Using fallback trim to {} transactions due to serialization error",
                stream_name,
                fallback.len()
            );
            fallback
        }
    }
}

/// ERR-005: Wrapper for pooled transactions with submission timestamp
/// Used to implement transaction TTL expiry
#[derive(Clone)]
struct PooledTransaction {
    tx: Transaction,
    submitted_at: Instant,
}

impl PooledTransaction {
    fn new(tx: Transaction) -> Self {
        Self {
            tx,
            submitted_at: Instant::now(),
        }
    }

    /// Check if this transaction has expired (ERR-005)
    fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.submitted_at).as_secs() > TX_TTL_SECS
    }
}

/// Block submission message for channel-based processing
#[allow(dead_code)]
struct BlockSubmission {
    block: Block,
    stream_type: StreamType,
    block_number: u64,
    reward: u128,
    fees: u128,
}

/// Stream priority for competition rules (A > B > C by reward value)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamPriority {
    StreamA = 3, // Highest (ASIC, 50 IDAG)
    StreamB = 2, // Medium (CPU, 25 IDAG) — GPU planned
    StreamC = 1, // Lowest (ZK, fee-based)
}

impl StreamPriority {
    pub fn from_stream_type(s: StreamType) -> Self {
        match s {
            StreamType::StreamA => StreamPriority::StreamA,
            StreamType::StreamB => StreamPriority::StreamB,
            StreamType::StreamC => StreamPriority::StreamC,
        }
    }
}

/// Block number allocator: reserve before validation, confirm on success, release on failure.
/// Prevents block number gaps when validation fails.
pub struct BlockNumberAllocator {
    next_available: Arc<AtomicU64>,
    pending_reservations: Arc<RwLock<HashMap<u64, (StreamType, std::time::Instant)>>>,
    failed_reservations: Arc<AtomicU64>,
    /// Free-list of released block numbers available for reuse (SEC-015)
    free_list: Arc<Mutex<BinaryHeap<Reverse<u64>>>>,
}

impl BlockNumberAllocator {
    pub fn new(starting_block: u64) -> Self {
        Self {
            next_available: Arc::new(AtomicU64::new(starting_block)),
            pending_reservations: Arc::new(RwLock::new(HashMap::new())),
            failed_reservations: Arc::new(AtomicU64::new(0)),
            free_list: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    /// Set next available block number (e.g. from blockchain height at startup).
    pub fn set_next_available(&self, n: u64) {
        self.next_available.store(n, Ordering::SeqCst);
    }

    /// Reset allocator to a specific block number, clearing the free-list.
    /// Used when the gap between allocated numbers and committed height grows too large.
    pub async fn reset_to(&self, n: u64) {
        self.next_available.store(n, Ordering::SeqCst);
        self.free_list.lock().await.clear();
        self.pending_reservations.write().await.clear();
        warn!("BlockAllocator reset to block number {}", n);
    }

    /// Reserve a block number (tentative; release if validation fails).
    pub async fn reserve(&self, stream_type: StreamType) -> u64 {
        // SEC-015: Try to reuse a released block number from free-list first
        let recycled = self.free_list.lock().await.pop();
        if let Some(Reverse(num)) = recycled {
            self.pending_reservations
                .write()
                .await
                .insert(num, (stream_type, std::time::Instant::now()));
            return num;
        }
        // No recycled numbers available — allocate new
        let mut pending = self.pending_reservations.write().await;
        let num = self.next_available.fetch_add(1, Ordering::SeqCst);
        pending.insert(num, (stream_type, std::time::Instant::now()));
        num
    }

    /// Confirm reservation (block validated successfully).
    pub async fn confirm(&self, block_number: u64) -> Result<(), String> {
        let mut pending = self.pending_reservations.write().await;
        if pending.remove(&block_number).is_none() {
            return Err(format!(
                "Block number {} was not reserved or already confirmed",
                block_number
            ));
        }
        Ok(())
    }

    /// Release reservation (block validation failed; number is abandoned as a gap).
    /// Not pushed back to free-list — cross-stream recycling caused spurious
    /// "too far ahead" resets when A picked up B's released low numbers.
    pub async fn release(&self, block_number: u64, _stream_type: StreamType) {
        if self
            .pending_reservations
            .write()
            .await
            .remove(&block_number)
            .is_some()
        {
            self.failed_reservations.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Clean up stale reservations that have been pending for too long.
    pub async fn cleanup_stale_reservations(&self) -> Vec<u64> {
        let mut pending = self.pending_reservations.write().await;
        let timeout = std::time::Duration::from_secs(300);
        let now = std::time::Instant::now();
        let stale: Vec<u64> = pending
            .iter()
            .filter(|(_, (_, created_at))| now.duration_since(*created_at) > timeout)
            .map(|(num, _)| *num)
            .collect();
        for num in &stale {
            warn!("Releasing stale block number reservation: {}", num);
            pending.remove(num);
        }
        stale
    }

    pub async fn get_stats(&self) -> (u64, usize, u64) {
        let next = self.next_available.load(Ordering::SeqCst);
        let pending_count = self.pending_reservations.read().await.len();
        let failed = self.failed_reservations.load(Ordering::SeqCst);
        (next, pending_count, failed)
    }

    /// Start a background task for periodic cleanup of stale reservations.
    /// Runs every 60 seconds and removes reservations older than 60 seconds.
    pub fn start_periodic_cleanup(self: &Arc<Self>) {
        let reservations = self.pending_reservations.clone();
        let failed_counter = self.failed_reservations.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
            loop {
                interval.tick().await;
                let mut pending = reservations.write().await;
                let timeout = std::time::Duration::from_secs(300);
                let now = std::time::Instant::now();
                let stale_count = pending
                    .iter()
                    .filter(|(_, (_, created_at))| now.duration_since(*created_at) > timeout)
                    .count();
                if stale_count > 0 {
                    let before = pending.len();
                    pending.retain(|_, (_, created_at)| now.duration_since(*created_at) <= timeout);
                    failed_counter.fetch_add(stale_count as u64, Ordering::SeqCst);
                    warn!("[PERIODIC_CLEANUP] Cleaned up {} stale block number reservations ({} → {})",
                             stale_count, before, pending.len());
                }
            }
        });
    }
}

/// Parent hash coordinator to reduce race conditions between streams.
pub struct ParentHashCoordinator {
    coordinator_lock: Arc<Mutex<()>>,
}

impl ParentHashCoordinator {
    pub fn new() -> Self {
        Self {
            coordinator_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Atomically select parent hashes for a stream.
    pub async fn select_parents(
        &self,
        blockchain: &Arc<RwLock<Blockchain>>,
        _stream_type: StreamType,
    ) -> Vec<Hash> {
        let _lock = self.coordinator_lock.lock().await;
        blockchain.read().await.with_blocks(|blocks| {
            if blocks.is_empty() {
                return Vec::new();
            }
            let start = blocks.len().saturating_sub(3);
            blocks[start..].iter().map(|b| b.hash).collect()
        })
    }

    /// Check if parent hashes still exist in blockchain.
    pub async fn are_parents_valid(
        &self,
        blockchain: &Arc<RwLock<Blockchain>>,
        parents: &[Hash],
    ) -> bool {
        if parents.is_empty() {
            return true;
        }
        // Use pre-built block_hashes set (O(1) per lookup) instead of cloning all blocks (O(N))
        let bc = blockchain.read().await;
        parents.iter().all(|p| bc.has_block_hash(p))
    }
}

impl Default for ParentHashCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Mining manager for BraidCore architecture
pub struct MiningManager {
    blockchain: Arc<RwLock<Blockchain>>,
    tx_pool: Arc<RwLock<Vec<PooledTransaction>>>, // Legacy shared pool (kept for compatibility)
    tx_pool_size: Arc<AtomicUsize>,               // Legacy shared pool size counter
    // Per-stream transaction pools (Task 50: Prevent contention between streams)
    // QUA-004: Changed from SegQueue to RwLock<Vec> for non-destructive iteration
    stream_a_pool: Arc<RwLock<Vec<PooledTransaction>>>,
    stream_b_pool: Arc<RwLock<Vec<PooledTransaction>>>,
    stream_c_pool: Arc<RwLock<Vec<PooledTransaction>>>,
    // Per-stream pool size counters
    stream_a_pool_size: Arc<AtomicUsize>,
    stream_b_pool_size: Arc<AtomicUsize>,
    stream_c_pool_size: Arc<AtomicUsize>,
    // Round-robin distribution counter for fair TX allocation
    tx_distribution_counter: Arc<AtomicU64>,
    // Single-stream mode flag (all TX go to Stream A)
    single_stream_mode: Arc<std::sync::atomic::AtomicBool>,
    block_allocator: Arc<BlockNumberAllocator>,
    parent_coordinator: Arc<ParentHashCoordinator>,
    miner_address: Address, // Address that receives block rewards
    is_mining: Arc<RwLock<bool>>,
    shard_manager: Option<Arc<ShardManager>>, // Optional shard manager
    local_shard_id: Option<usize>, // Shard ID this node is assigned to (when sharding enabled)
    fairness_analyzer: Arc<tokio::sync::RwLock<fairness::FairnessAnalyzer>>, // Fairness metrics
    ordering_policy: Arc<RwLock<ordering::OrderingPolicy>>, // Transaction ordering policy
    ordering_context: Arc<RwLock<ordering::OrderingContext>>, // Ordering context
    last_update_time: AtomicU64,   // Unix millis - atomic timestamp for ordering (PER-003)
    metrics: Option<crate::metrics::MetricsHandle>, // Optional metrics
    block_sender: mpsc::UnboundedSender<BlockSubmission>, // Channel sender for block submissions
    node_registry: Option<Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>>, // Optional node registry for participation tracking
    node_identity: Option<crate::governance::NodeIdentity>, // Node identity for participation tracking
    network: Arc<RwLock<Option<Arc<crate::network::NetworkManager>>>>, // Network manager for block broadcasting (set after construction)
    /// MiningBackend is shared across mining threads via Arc.
    /// Implementations must be Send + Sync. The trait is read-only
    /// during mining — state mutations go through the blockchain.
    mining_backend: Arc<dyn pow::MiningBackend>, // Mining backend for Stream B (CPU-only; GPU planned)
    // Pruning configuration
    prune_interval_secs: u64,
    keep_red_blocks: bool,
    prune_batch_size: usize,
    // GhostDAG consensus engine for parent selection
    ghostdag: Option<Arc<RwLock<GhostDAG>>>,
    // ZK proving key for Stream C state transition proofs
    #[cfg(feature = "privacy")]
    zk_proving_key: Option<Arc<ark_groth16::ProvingKey<ark_bn254::Bn254>>>,
    // BPR-004: Shutdown token for graceful shutdown
    shutdown_token: Arc<AtomicBool>,
    /// SYNC-001: Syncing flag to pause mining during IBD (Initial Block Download)
    /// When true, mining loops skip block production to prevent DAG tip contamination
    syncing: Arc<AtomicBool>,
    /// SEC-015: Transactions currently being assembled into blocks across all streams.
    /// Prevents the same transaction from being included in multiple stream blocks.
    in_flight_txs: Arc<RwLock<HashSet<Hash>>>, // tx hashes currently in block assembly
    /// QUA-003: Set of transaction hashes currently in any pool. Prevents duplicate insertion.
    pool_tx_hashes: Arc<RwLock<HashSet<Hash>>>,
    /// QUA-009: Future-nonce transactions waiting to become executable.
    /// Key: sender address, Value: BTreeMap of nonce -> transaction (sorted by nonce)
    future_txs: Arc<RwLock<HashMap<Address, BTreeMap<u64, PooledTransaction>>>>,
    /// Dilithium3 keypair for post-quantum block header signing
    /// When present, blocks mined by this node will include a Dilithium3 signature
    dilithium_keypair: Option<crate::pqc::PqAccount>,
}

impl MiningManager {
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, miner_address: Address) -> Self {
        // Create channel for block submissions (serializes block additions)
        let (block_sender, block_receiver) = mpsc::unbounded_channel();

        // Start block processor task
        let blockchain_processor = blockchain.clone();
        let miner_address_processor = miner_address;
        let fairness_analyzer_processor =
            Arc::new(tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()));
        let metrics_processor = None::<crate::metrics::MetricsHandle>;
        let node_registry_processor =
            None::<Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>>;
        let node_identity_processor = None::<crate::governance::NodeIdentity>;
        let tx_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let tx_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_pool_processor = tx_pool.clone();
        let tx_pool_size_processor = tx_pool_size.clone();

        // Initialize per-stream transaction pools (Task 50)
        // QUA-004: Changed from SegQueue to RwLock<Vec> for non-destructive iteration
        let stream_a_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_b_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_c_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_a_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_b_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_c_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_distribution_counter = Arc::new(AtomicU64::new(0));

        // SEC-015: Initialize in-flight transaction set for inter-stream deduplication
        let in_flight_txs = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-003: Initialize pool membership set for pool-level deduplication
        let pool_tx_hashes = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-009: Initialize future-nonce transaction queue
        let future_txs = Arc::new(RwLock::new(HashMap::<
            Address,
            BTreeMap<u64, PooledTransaction>,
        >::new()));

        // Initialize block allocator at 0 — the mining loop's is_ready() barrier ensures
        // blockchain is fully loaded before mining starts, and the existing allocator sync in
        // start_mining() will set the correct height from the loaded blockchain state.
        let block_allocator = Arc::new(BlockNumberAllocator::new(0));
        let parent_coordinator = Arc::new(ParentHashCoordinator::new());
        let block_allocator_processor = block_allocator.clone();
        let parent_coordinator_processor = parent_coordinator.clone();

        // Start periodic cleanup of stale block number reservations
        block_allocator.start_periodic_cleanup();

        // Create network Arc (initially None, set later via set_network())
        let network = Arc::new(RwLock::new(None));
        let network_processor = network.clone();

        // SEC-015: Clone in_flight_txs for process_blocks
        let in_flight_txs_processor = in_flight_txs.clone();

        // QUA-003: Clone pool_tx_hashes for process_blocks
        let pool_tx_hashes_processor = pool_tx_hashes.clone();

        // QUA-009: Clone future_txs for process_blocks
        let future_txs_processor = future_txs.clone();

        tokio::spawn(async move {
            process_blocks(
                block_receiver,
                blockchain_processor,
                miner_address_processor,
                fairness_analyzer_processor,
                metrics_processor,
                node_registry_processor,
                node_identity_processor,
                tx_pool_processor,
                tx_pool_size_processor,
                network_processor,
                block_allocator_processor,
                parent_coordinator_processor,
                None, // No shard manager
                in_flight_txs_processor,
                pool_tx_hashes_processor,
                future_txs_processor,
            )
            .await;
        });

        Self {
            blockchain,
            tx_pool,
            tx_pool_size,
            stream_a_pool,
            stream_b_pool,
            stream_c_pool,
            stream_a_pool_size,
            stream_b_pool_size,
            stream_c_pool_size,
            tx_distribution_counter,
            single_stream_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            block_allocator,
            parent_coordinator,
            miner_address,
            is_mining: Arc::new(RwLock::new(false)),
            shard_manager: None,
            local_shard_id: None, // No sharding
            fairness_analyzer: Arc::new(
                tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()),
            ),
            ordering_policy: Arc::new(RwLock::new(ordering::OrderingPolicy::default())),
            ordering_context: Arc::new(RwLock::new(ordering::OrderingContext::new())),
            last_update_time: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
            metrics: None,
            block_sender,
            node_registry: None,
            node_identity: None,
            network, // Use the SAME Arc that was passed to spawned task
            mining_backend: Arc::new(pow::CpuBackend::new()), // Default to CPU backend
            prune_interval_secs: 60, // Default: prune every 60 seconds
            keep_red_blocks: false, // Default: prune red blocks
            prune_batch_size: 200, // Default: 200 blocks per batch
            ghostdag: None, // GhostDAG set via set_ghostdag() after construction
            #[cfg(feature = "privacy")]
            zk_proving_key: None,
            // BPR-004: Initialize shutdown token
            shutdown_token: Arc::new(AtomicBool::new(false)),
            // SYNC-001: Initialize syncing flag (not syncing by default)
            syncing: Arc::new(AtomicBool::new(false)),
            // SEC-015: Initialize in-flight transaction set
            in_flight_txs,
            // QUA-003: Initialize pool membership set
            pool_tx_hashes,
            // QUA-009: Initialize future-nonce transaction queue
            future_txs,
            // Dilithium3 keypair for PQ block signing (None by default, set via set_dilithium_keypair)
            // Auto-generate when kyber feature is enabled
            #[cfg(feature = "kyber")]
            dilithium_keypair: {
                info!("Kyber feature enabled: auto-generating Dilithium3 keypair for PQ block signing");
                Some(crate::pqc::PqAccount::new_dilithium3())
            },
            #[cfg(not(feature = "kyber"))]
            dilithium_keypair: None,
        }
    }

    // TODO: QUA-003 — Extract shared initialization logic from new(), with_node_registry(), with_sharding()

    /// Set the Dilithium3 keypair for post-quantum block header signing
    /// When set, all blocks mined by this node will include a Dilithium3 signature
    pub fn set_dilithium_keypair(&mut self, keypair: crate::pqc::PqAccount) {
        self.dilithium_keypair = Some(keypair);
    }

    /// Generate and set a new Dilithium3 keypair for block signing
    pub fn generate_dilithium_keypair(&mut self) {
        self.dilithium_keypair = Some(crate::pqc::PqAccount::new_dilithium3());
    }

    /// Create mining manager with node registry for participation tracking
    pub fn with_node_registry(
        blockchain: Arc<RwLock<Blockchain>>,
        miner_address: Address,
        node_registry: Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>,
        node_identity: crate::governance::NodeIdentity,
    ) -> Self {
        // Create channel for block submissions
        let (block_sender, block_receiver) = mpsc::unbounded_channel();

        // Start block processor task
        let blockchain_processor = blockchain.clone();
        let miner_address_processor = miner_address;
        let fairness_analyzer_processor =
            Arc::new(tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()));
        let metrics_processor = None::<crate::metrics::MetricsHandle>;
        let node_registry_processor = Some(node_registry.clone());
        let node_identity_processor = Some(node_identity.clone());
        let tx_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let tx_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_pool_processor = tx_pool.clone();
        let tx_pool_size_processor = tx_pool_size.clone();

        // Initialize per-stream transaction pools (Task 50)
        // QUA-004: Changed from SegQueue to RwLock<Vec> for non-destructive iteration
        let stream_a_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_b_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_c_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_a_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_b_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_c_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_distribution_counter = Arc::new(AtomicU64::new(0));

        // SEC-015: Initialize in-flight transaction set for inter-stream deduplication
        let in_flight_txs = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-003: Initialize pool membership set for pool-level deduplication
        let pool_tx_hashes = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-009: Initialize future-nonce transaction queue
        let future_txs = Arc::new(RwLock::new(HashMap::<
            Address,
            BTreeMap<u64, PooledTransaction>,
        >::new()));

        // Initialize block allocator at 0 — the mining loop's is_ready() barrier ensures
        // blockchain is fully loaded before mining starts, and the existing allocator sync in
        // start_mining() will set the correct height from the loaded blockchain state.
        let block_allocator = Arc::new(BlockNumberAllocator::new(0));
        let parent_coordinator = Arc::new(ParentHashCoordinator::new());
        let block_allocator_processor = block_allocator.clone();
        let parent_coordinator_processor = parent_coordinator.clone();

        // Start periodic cleanup of stale block number reservations
        block_allocator.start_periodic_cleanup();

        // Create network Arc (initially None, set later via set_network())
        let network = Arc::new(RwLock::new(None));
        let network_processor = network.clone();

        // SEC-015: Clone in_flight_txs for process_blocks
        let in_flight_txs_processor = in_flight_txs.clone();

        // QUA-003: Clone pool_tx_hashes for process_blocks
        let pool_tx_hashes_processor = pool_tx_hashes.clone();

        // QUA-009: Clone future_txs for process_blocks
        let future_txs_processor = future_txs.clone();

        tokio::spawn(async move {
            process_blocks(
                block_receiver,
                blockchain_processor,
                miner_address_processor,
                fairness_analyzer_processor,
                metrics_processor,
                node_registry_processor,
                node_identity_processor,
                tx_pool_processor,
                tx_pool_size_processor,
                network_processor,
                block_allocator_processor,
                parent_coordinator_processor,
                None, // No shard manager
                in_flight_txs_processor,
                pool_tx_hashes_processor,
                future_txs_processor,
            )
            .await;
        });

        Self {
            blockchain,
            tx_pool,
            tx_pool_size,
            stream_a_pool,
            stream_b_pool,
            stream_c_pool,
            stream_a_pool_size,
            stream_b_pool_size,
            stream_c_pool_size,
            tx_distribution_counter,
            single_stream_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            block_allocator,
            parent_coordinator,
            miner_address,
            is_mining: Arc::new(RwLock::new(false)),
            shard_manager: None,
            local_shard_id: None, // No sharding
            fairness_analyzer: Arc::new(
                tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()),
            ),
            ordering_policy: Arc::new(RwLock::new(ordering::OrderingPolicy::default())),
            ordering_context: Arc::new(RwLock::new(ordering::OrderingContext::new())),
            last_update_time: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
            metrics: None,
            block_sender,
            node_registry: Some(node_registry),
            node_identity: Some(node_identity),
            network,
            mining_backend: Arc::new(pow::CpuBackend::new()), // Default to CPU backend
            prune_interval_secs: 60,                          // Default: prune every 60 seconds
            keep_red_blocks: false,                           // Default: prune red blocks
            prune_batch_size: 200,                            // Default: 200 blocks per batch
            ghostdag: None, // GhostDAG set via set_ghostdag() after construction
            #[cfg(feature = "privacy")]
            zk_proving_key: None,
            // BPR-004: Initialize shutdown token
            shutdown_token: Arc::new(AtomicBool::new(false)),
            // SYNC-001: Initialize syncing flag (not syncing by default)
            syncing: Arc::new(AtomicBool::new(false)),
            // SEC-015: Initialize in-flight transaction set
            in_flight_txs,
            // QUA-003: Initialize pool membership set
            pool_tx_hashes,
            // QUA-009: Initialize future-nonce transaction queue
            future_txs,
            // Dilithium3 keypair for PQ block signing
            // Auto-generate when kyber feature is enabled
            #[cfg(feature = "kyber")]
            dilithium_keypair: {
                info!("Kyber feature enabled: auto-generating Dilithium3 keypair for PQ block signing");
                Some(crate::pqc::PqAccount::new_dilithium3())
            },
            #[cfg(not(feature = "kyber"))]
            dilithium_keypair: None,
        }
    }

    /// Create mining manager with sharding
    pub fn with_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        miner_address: Address,
        shard_manager: Arc<ShardManager>,
        local_shard_id: usize,
    ) -> Self {
        // Create channel for block submissions
        let (block_sender, block_receiver) = mpsc::unbounded_channel();

        // Start block processor task
        let blockchain_processor = blockchain.clone();
        let miner_address_processor = miner_address;
        let fairness_analyzer_processor =
            Arc::new(tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()));
        let metrics_processor = None::<crate::metrics::MetricsHandle>;
        let node_registry_processor =
            None::<Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>>;
        let node_identity_processor = None::<crate::governance::NodeIdentity>;
        let tx_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let tx_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_pool_processor = tx_pool.clone();
        let tx_pool_size_processor = tx_pool_size.clone();

        // Initialize per-stream transaction pools (Task 50)
        // QUA-004: Changed from SegQueue to RwLock<Vec> for non-destructive iteration
        let stream_a_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_b_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_c_pool = Arc::new(RwLock::new(Vec::<PooledTransaction>::new()));
        let stream_a_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_b_pool_size = Arc::new(AtomicUsize::new(0));
        let stream_c_pool_size = Arc::new(AtomicUsize::new(0));
        let tx_distribution_counter = Arc::new(AtomicU64::new(0));

        // SEC-015: Initialize in-flight transaction set for inter-stream deduplication
        let in_flight_txs = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-003: Initialize pool membership set for pool-level deduplication
        let pool_tx_hashes = Arc::new(RwLock::new(HashSet::<Hash>::new()));

        // QUA-009: Initialize future-nonce transaction queue
        let future_txs = Arc::new(RwLock::new(HashMap::<
            Address,
            BTreeMap<u64, PooledTransaction>,
        >::new()));

        // Initialize block allocator at 0 — the mining loop's is_ready() barrier ensures
        // blockchain is fully loaded before mining starts, and the existing allocator sync in
        // start_mining() will set the correct height from the loaded blockchain state.
        let block_allocator = Arc::new(BlockNumberAllocator::new(0));
        let parent_coordinator = Arc::new(ParentHashCoordinator::new());
        let block_allocator_processor = block_allocator.clone();
        let parent_coordinator_processor = parent_coordinator.clone();

        // Start periodic cleanup of stale block number reservations
        block_allocator.start_periodic_cleanup();

        // Create network Arc (initially None, set later via set_network())
        let network = Arc::new(RwLock::new(None));
        let network_processor = network.clone();
        let shard_manager_processor = Some(shard_manager.clone());

        // SEC-015: Clone in_flight_txs for process_blocks
        let in_flight_txs_processor = in_flight_txs.clone();

        // QUA-003: Clone pool_tx_hashes for process_blocks
        let pool_tx_hashes_processor = pool_tx_hashes.clone();

        // QUA-009: Clone future_txs for process_blocks
        let future_txs_processor = future_txs.clone();

        tokio::spawn(async move {
            process_blocks(
                block_receiver,
                blockchain_processor,
                miner_address_processor,
                fairness_analyzer_processor,
                metrics_processor,
                node_registry_processor,
                node_identity_processor,
                tx_pool_processor,
                tx_pool_size_processor,
                network_processor,
                block_allocator_processor,
                parent_coordinator_processor,
                shard_manager_processor,
                in_flight_txs_processor,
                pool_tx_hashes_processor,
                future_txs_processor,
            )
            .await;
        });

        Self {
            blockchain,
            tx_pool,
            tx_pool_size,
            stream_a_pool,
            stream_b_pool,
            stream_c_pool,
            stream_a_pool_size,
            stream_b_pool_size,
            stream_c_pool_size,
            tx_distribution_counter,
            single_stream_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            block_allocator,
            parent_coordinator,
            miner_address,
            is_mining: Arc::new(RwLock::new(false)),
            shard_manager: Some(shard_manager),
            local_shard_id: Some(local_shard_id),
            fairness_analyzer: Arc::new(
                tokio::sync::RwLock::new(fairness::FairnessAnalyzer::new()),
            ),
            ordering_policy: Arc::new(RwLock::new(ordering::OrderingPolicy::default())),
            ordering_context: Arc::new(RwLock::new(ordering::OrderingContext::new())),
            last_update_time: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            ),
            metrics: None,
            block_sender,
            node_registry: None,
            node_identity: None,
            network, // Use the SAME Arc that was passed to spawned task
            mining_backend: Arc::new(pow::CpuBackend::new()), // Default to CPU backend
            prune_interval_secs: 60, // Default: prune every 60 seconds
            keep_red_blocks: false, // Default: prune red blocks
            prune_batch_size: 200, // Default: 200 blocks per batch
            ghostdag: None, // GhostDAG set via set_ghostdag() after construction
            #[cfg(feature = "privacy")]
            zk_proving_key: None,
            // BPR-004: Initialize shutdown token
            shutdown_token: Arc::new(AtomicBool::new(false)),
            // SYNC-001: Initialize syncing flag (not syncing by default)
            syncing: Arc::new(AtomicBool::new(false)),
            // SEC-015: Initialize in-flight transaction set
            in_flight_txs,
            // QUA-003: Initialize pool membership set
            pool_tx_hashes,
            // QUA-009: Initialize future-nonce transaction queue
            future_txs,
            // Dilithium3 keypair for PQ block signing
            // Auto-generate when kyber feature is enabled
            #[cfg(feature = "kyber")]
            dilithium_keypair: {
                info!("Kyber feature enabled: auto-generating Dilithium3 keypair for PQ block signing");
                Some(crate::pqc::PqAccount::new_dilithium3())
            },
            #[cfg(not(feature = "kyber"))]
            dilithium_keypair: None,
        }
    }

    /// Set network manager for block broadcasting
    pub async fn set_network(&self, network: Arc<crate::network::NetworkManager>) {
        info!("Setting network manager for block broadcasting");
        let mut network_lock = self.network.write().await;
        *network_lock = Some(network);
        info!("Network manager set successfully");
    }

    /// Set metrics handle
    pub fn set_metrics(&mut self, metrics: crate::metrics::MetricsHandle) {
        self.metrics = Some(metrics);
    }

    /// Set mining backend for Stream B (CPU; GPU via OpenCL planned)
    pub fn set_mining_backend(&mut self, backend: Arc<dyn pow::MiningBackend>) {
        self.mining_backend = backend;
        info!("Mining backend set to: {}", self.mining_backend.name());
    }

    /// Set pruning configuration
    pub fn set_pruning_config(
        &mut self,
        prune_interval_secs: u64,
        keep_red_blocks: bool,
        prune_batch_size: usize,
    ) {
        self.prune_interval_secs = prune_interval_secs;
        self.keep_red_blocks = keep_red_blocks;
        self.prune_batch_size = prune_batch_size;
        info!(
            "Pruning config: interval={}s, keep_red_blocks={}, batch_size={}",
            prune_interval_secs, keep_red_blocks, prune_batch_size
        );
    }

    /// Set GhostDAG consensus engine for parent selection
    pub fn set_ghostdag(&mut self, ghostdag: Arc<RwLock<GhostDAG>>) {
        self.ghostdag = Some(ghostdag);
        info!("GhostDAG consensus engine set for mining parent selection");
    }

    /// Set ZK proving key for Stream C state transition proofs
    #[cfg(feature = "privacy")]
    pub fn set_zk_proving_key(&mut self, pk: Arc<ark_groth16::ProvingKey<ark_bn254::Bn254>>) {
        self.zk_proving_key = Some(pk);
        info!("ZK proving key set for Stream C block production");
    }

    /// Clone for mining (internal use)
    /// Clone mining manager for parallel stream mining
    /// Note: node_registry and node_identity are shared across all streams
    fn clone_for_mining(&self) -> Self {
        Self {
            blockchain: self.blockchain.clone(),
            tx_pool: self.tx_pool.clone(),
            tx_pool_size: self.tx_pool_size.clone(),
            stream_a_pool: self.stream_a_pool.clone(),
            stream_b_pool: self.stream_b_pool.clone(),
            stream_c_pool: self.stream_c_pool.clone(),
            stream_a_pool_size: self.stream_a_pool_size.clone(),
            stream_b_pool_size: self.stream_b_pool_size.clone(),
            stream_c_pool_size: self.stream_c_pool_size.clone(),
            tx_distribution_counter: self.tx_distribution_counter.clone(),
            single_stream_mode: self.single_stream_mode.clone(),
            block_allocator: self.block_allocator.clone(),
            parent_coordinator: self.parent_coordinator.clone(),
            miner_address: self.miner_address,
            is_mining: self.is_mining.clone(),
            shard_manager: self.shard_manager.clone(),
            local_shard_id: self.local_shard_id,
            fairness_analyzer: self.fairness_analyzer.clone(),
            ordering_policy: self.ordering_policy.clone(),
            ordering_context: self.ordering_context.clone(),
            last_update_time: AtomicU64::new(self.last_update_time.load(Ordering::Relaxed)),
            metrics: self.metrics.clone(),
            block_sender: self.block_sender.clone(), // Clone sender (receiver is shared)
            node_registry: self.node_registry.clone(),
            node_identity: self.node_identity.clone(),
            network: self.network.clone(), // Clone network reference
            mining_backend: self.mining_backend.clone(), // Clone the mining backend
            prune_interval_secs: self.prune_interval_secs,
            keep_red_blocks: self.keep_red_blocks,
            prune_batch_size: self.prune_batch_size,
            ghostdag: self.ghostdag.clone(), // Clone GhostDAG reference
            #[cfg(feature = "privacy")]
            zk_proving_key: self.zk_proving_key.clone(), // Clone ZK proving key for Stream C
            // BPR-004: Clone shutdown token (shared across all mining streams)
            shutdown_token: self.shutdown_token.clone(),
            // SYNC-001: Clone syncing flag (shared across all mining streams)
            syncing: self.syncing.clone(),
            // SEC-015: Clone in-flight transaction set (shared across all mining streams)
            in_flight_txs: self.in_flight_txs.clone(),
            // QUA-003: Clone pool membership set (shared across all mining streams)
            pool_tx_hashes: self.pool_tx_hashes.clone(),
            // QUA-009: Clone future-nonce transaction queue (shared across all mining streams)
            future_txs: self.future_txs.clone(),
            // Dilithium3 keypair for PQ block signing (shared across all mining streams)
            dilithium_keypair: self.dilithium_keypair.clone(),
        }
    }

    /// Set transaction ordering policy
    pub async fn set_ordering_policy(&self, policy: ordering::OrderingPolicy) {
        *self.ordering_policy.write().await = policy;
    }

    /// Get current ordering policy
    pub async fn get_ordering_policy(&self) -> ordering::OrderingPolicy {
        *self.ordering_policy.read().await
    }

    /// QUA-009: Get the current nonce for a sender from blockchain state
    async fn get_sender_nonce(&self, sender: Address) -> u64 {
        let blockchain = self.blockchain.read().await;
        blockchain.get_nonce(sender)
    }

    /// EIP-1559: Get the current base fee for a given stream.
    ///
    /// This calculates the base fee based on the parent block's gas usage.
    /// For genesis or when no parent exists, returns BASE_FEE_INITIAL.
    ///
    /// # Arguments
    /// * `stream_type` - The stream type (A, B, or C)
    ///
    /// # Returns
    /// The current base fee per gas for the stream
    async fn get_current_base_fee(&self, stream_type: StreamType) -> u128 {
        let blockchain = self.blockchain.read().await;
        let gas_limit = match stream_type {
            StreamType::StreamA => STREAM_A_GAS_LIMIT,
            StreamType::StreamB => STREAM_B_GAS_LIMIT,
            StreamType::StreamC => STREAM_C_GAS_LIMIT,
        };

        match blockchain.get_latest_block_for_stream(stream_type) {
            Some(block) => {
                let parent_base_fee = block.header.base_fee_per_gas;
                // Calculate gas used as sum of gas_limit for all transactions
                // In a real implementation, we'd track actual gas used
                // For now, estimate based on transaction count
                let parent_gas_used: u64 = block.transactions.iter().map(|tx| tx.gas_limit).sum();

                // Calculate new base fee based on parent block
                calculate_base_fee(parent_base_fee, parent_gas_used, gas_limit)
            }
            None => {
                // No parent block (genesis), use initial base fee
                BASE_FEE_INITIAL
            }
        }
    }

    /// ARC-008: Find and evict the lowest-fee transaction from a pool if new_tx_fee is higher
    /// Returns true if a transaction was evicted (new_tx_fee > min_fee), false otherwise
    async fn evict_lowest_fee_tx_if_higher(
        &self,
        pool: &Arc<RwLock<Vec<PooledTransaction>>>,
        pool_size: &AtomicUsize,
        new_tx_fee: u128,
    ) -> bool {
        let pool_read = pool.read().await;
        if pool_read.is_empty() {
            return false;
        }

        // Find the lowest-fee transaction
        if let Some(min_fee_tx) = pool_read.iter().min_by_key(|pt| pt.tx.fee) {
            let min_fee = min_fee_tx.tx.fee;

            // Only evict if new transaction has higher fee
            if new_tx_fee > min_fee {
                drop(pool_read);

                // Remove the lowest-fee transaction from pool
                let mut pool_write = pool.write().await;
                // Re-find the min in case pool changed
                if let Some((idx, evicted)) = pool_write
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, pt)| pt.tx.fee)
                    .map(|(i, pt)| (i, pt.clone()))
                {
                    pool_write.swap_remove(idx);
                    pool_size.fetch_sub(1, Ordering::Release);

                    // Also remove from pool_tx_hashes to allow re-submission
                    let mut hashes = self.pool_tx_hashes.write().await;
                    hashes.remove(&evicted.tx.hash);
                    drop(hashes);

                    debug!(
                        "ARC-008: Evicted low-fee tx {} (fee {}) for higher-fee tx (fee {})",
                        hex::encode(&evicted.tx.hash.0[..8]),
                        evicted.tx.fee,
                        new_tx_fee
                    );
                    return true;
                }
            }
        }
        false
    }

    /// ARC-008: Evict expired transactions from all pools based on TTL
    /// Called periodically at the start of block assembly
    async fn evict_expired_transactions(&self) {
        let now = Instant::now();

        // Evict from Stream A pool
        {
            let mut expired_hashes = Vec::new();
            {
                let pool = self.stream_a_pool.read().await;
                for pt in pool.iter() {
                    if now.duration_since(pt.submitted_at).as_secs() >= TX_TTL_SECS {
                        expired_hashes.push(pt.tx.hash);
                    }
                }
            }
            if !expired_hashes.is_empty() {
                let mut pool = self.stream_a_pool.write().await;
                let before = pool.len();
                pool.retain(|pt| now.duration_since(pt.submitted_at).as_secs() < TX_TTL_SECS);
                let evicted = before - pool.len();
                if evicted > 0 {
                    self.stream_a_pool_size
                        .fetch_sub(evicted, Ordering::Release);
                    // Remove expired hashes from pool_tx_hashes
                    let mut hashes = self.pool_tx_hashes.write().await;
                    for hash in &expired_hashes {
                        hashes.remove(hash);
                    }
                    debug!(
                        "ARC-008: Evicted {} expired transactions from Stream A pool",
                        evicted
                    );
                }
            }
        }

        // Evict from Stream B pool
        {
            let mut expired_hashes = Vec::new();
            {
                let pool = self.stream_b_pool.read().await;
                for pt in pool.iter() {
                    if now.duration_since(pt.submitted_at).as_secs() >= TX_TTL_SECS {
                        expired_hashes.push(pt.tx.hash);
                    }
                }
            }
            if !expired_hashes.is_empty() {
                let mut pool = self.stream_b_pool.write().await;
                let before = pool.len();
                pool.retain(|pt| now.duration_since(pt.submitted_at).as_secs() < TX_TTL_SECS);
                let evicted = before - pool.len();
                if evicted > 0 {
                    self.stream_b_pool_size
                        .fetch_sub(evicted, Ordering::Release);
                    let mut hashes = self.pool_tx_hashes.write().await;
                    for hash in &expired_hashes {
                        hashes.remove(hash);
                    }
                    debug!(
                        "ARC-008: Evicted {} expired transactions from Stream B pool",
                        evicted
                    );
                }
            }
        }

        // Evict from Stream C pool
        {
            let mut expired_hashes = Vec::new();
            {
                let pool = self.stream_c_pool.read().await;
                for pt in pool.iter() {
                    if now.duration_since(pt.submitted_at).as_secs() >= TX_TTL_SECS {
                        expired_hashes.push(pt.tx.hash);
                    }
                }
            }
            if !expired_hashes.is_empty() {
                let mut pool = self.stream_c_pool.write().await;
                let before = pool.len();
                pool.retain(|pt| now.duration_since(pt.submitted_at).as_secs() < TX_TTL_SECS);
                let evicted = before - pool.len();
                if evicted > 0 {
                    self.stream_c_pool_size
                        .fetch_sub(evicted, Ordering::Release);
                    let mut hashes = self.pool_tx_hashes.write().await;
                    for hash in &expired_hashes {
                        hashes.remove(hash);
                    }
                    debug!(
                        "ARC-008: Evicted {} expired transactions from Stream C pool",
                        evicted
                    );
                }
            }
        }

        // Evict from legacy pool
        {
            let mut expired_hashes = Vec::new();
            {
                let pool = self.tx_pool.read().await;
                for pt in pool.iter() {
                    if now.duration_since(pt.submitted_at).as_secs() >= TX_TTL_SECS {
                        expired_hashes.push(pt.tx.hash);
                    }
                }
            }
            if !expired_hashes.is_empty() {
                let mut pool = self.tx_pool.write().await;
                let before = pool.len();
                pool.retain(|pt| now.duration_since(pt.submitted_at).as_secs() < TX_TTL_SECS);
                let evicted = before - pool.len();
                if evicted > 0 {
                    self.tx_pool_size.fetch_sub(evicted, Ordering::Release);
                    let mut hashes = self.pool_tx_hashes.write().await;
                    for hash in &expired_hashes {
                        hashes.remove(hash);
                    }
                    debug!(
                        "ARC-008: Evicted {} expired transactions from legacy pool",
                        evicted
                    );
                }
            }
        }

        // ARC-008: Also clean up stale future-nonce transactions
        self.cleanup_stale_future_txs().await;
    }

    /// QUA-009: Add transaction directly to pool (internal helper)
    /// This bypasses nonce checking and is used for promoting future txs
    /// ARC-008: Implements fee-based eviction (lowest-fee-first when pool is full)
    async fn add_to_pool_inner(&self, pooled_tx: PooledTransaction) {
        let tx = pooled_tx.tx.clone();

        // Determine which stream this transaction will be routed to
        let target_stream = if self.single_stream_mode.load(Ordering::Acquire) {
            0
        } else {
            let counter = self.tx_distribution_counter.load(Ordering::Acquire);
            counter % 3
        };

        // ARC-008: Check per-stream hard cap and evict lowest-fee transaction if new tx has higher fee
        let new_tx_fee = tx.fee;
        match target_stream {
            0 => {
                if self.stream_a_pool_size.load(Ordering::Acquire) >= MAX_STREAM_A_POOL_SIZE {
                    self.evict_lowest_fee_tx_if_higher(
                        &self.stream_a_pool,
                        &self.stream_a_pool_size,
                        new_tx_fee,
                    )
                    .await;
                }
            }
            1 => {
                if self.stream_b_pool_size.load(Ordering::Acquire) >= MAX_STREAM_B_POOL_SIZE {
                    self.evict_lowest_fee_tx_if_higher(
                        &self.stream_b_pool,
                        &self.stream_b_pool_size,
                        new_tx_fee,
                    )
                    .await;
                }
            }
            _ => {
                if self.stream_c_pool_size.load(Ordering::Acquire) >= MAX_STREAM_C_POOL_SIZE {
                    self.evict_lowest_fee_tx_if_higher(
                        &self.stream_c_pool,
                        &self.stream_c_pool_size,
                        new_tx_fee,
                    )
                    .await;
                }
            }
        }

        // Distribute to appropriate stream pool
        if self.single_stream_mode.load(Ordering::Acquire) {
            self.stream_a_pool.write().await.push(pooled_tx.clone());
            self.stream_a_pool_size.fetch_add(1, Ordering::Release);
        } else {
            let counter = self.tx_distribution_counter.fetch_add(1, Ordering::AcqRel);
            let stream_index = counter % 3;

            match stream_index {
                0 => {
                    self.stream_a_pool.write().await.push(pooled_tx.clone());
                    self.stream_a_pool_size.fetch_add(1, Ordering::Release);
                }
                1 => {
                    self.stream_b_pool.write().await.push(pooled_tx.clone());
                    self.stream_b_pool_size.fetch_add(1, Ordering::Release);
                }
                _ => {
                    self.stream_c_pool.write().await.push(pooled_tx.clone());
                    self.stream_c_pool_size.fetch_add(1, Ordering::Release);
                }
            }
        }

        debug!(
            "Transaction {} added to pool (nonce {})",
            hex::encode(&tx.hash.0[..8]),
            tx.nonce
        );
    }

    /// QUA-009: Promote future-nonce transactions that are now executable
    /// Called after a transaction is added to pool or after block commit
    async fn promote_future_txs(&self, sender: Address) {
        let mut futures = self.future_txs.write().await;
        if let Some(sender_queue) = futures.get_mut(&sender) {
            let mut next_nonce = self.get_sender_nonce(sender).await;
            let mut promoted = 0;

            while let Some(pooled_tx) = sender_queue.remove(&next_nonce) {
                // Remove from pool_tx_hashes since we\'re about to add it to the main pool
                // (it will be re-added by add_to_pool_inner via dedup check)
                self.pool_tx_hashes.write().await.remove(&pooled_tx.tx.hash);

                // Phase 6 Gap 5: Route promoted transactions to shard_manager when sharding is enabled
                if let Some(shard_manager) = &self.shard_manager {
                    // Ignore error on promotion - just log and continue
                    if let Err(e) = shard_manager.add_transaction(pooled_tx.tx.clone()).await {
                        debug!("Failed to add promoted tx to shard: {:?}", e);
                    }
                } else {
                    self.add_to_pool_inner(pooled_tx).await;
                }
                promoted += 1;
                next_nonce += 1;
            }

            if promoted > 0 {
                debug!(
                    "Promoted {} future-nonce tx(s) for sender {} (starting at nonce {})",
                    promoted,
                    hex::encode(&sender.0[..8]),
                    next_nonce - promoted as u64
                );
            }

            if sender_queue.is_empty() {
                futures.remove(&sender);
            }
        }
    }

    /// QUA-009: Clean up stale future-nonce transactions (older than TTL)
    pub async fn cleanup_stale_future_txs(&self) {
        let now = tokio::time::Instant::now();
        let mut futures = self.future_txs.write().await;
        let mut evicted_hashes = Vec::new();

        // MED-01: Fix retain logic - collect evicted hashes and remove stale entries in one pass
        futures.retain(|_sender, queue| {
            queue.retain(|_nonce, tx| {
                let age = now.duration_since(tx.submitted_at).as_secs();
                let is_stale = age >= FUTURE_TX_TTL_SECS;
                if is_stale {
                    evicted_hashes.push(tx.tx.hash);
                }
                !is_stale
            });
            !queue.is_empty()
        });

        // Clean up pool_tx_hashes for evicted transactions
        if !evicted_hashes.is_empty() {
            let mut pool_hashes = self.pool_tx_hashes.write().await;
            for hash in &evicted_hashes {
                pool_hashes.remove(hash);
            }
        }

        if !evicted_hashes.is_empty() {
            debug!(
                "Cleaned up {} stale future-nonce transactions",
                evicted_hashes.len()
            );
        }
    }

    /// Add transaction to pool
    pub async fn add_transaction(&self, tx: Transaction) -> crate::error::BlockchainResult<()> {
        // Mempool logging disabled to reduce console noise

        // QUA-003: Check pool-level deduplication before insertion
        let tx_hash = tx.hash;
        {
            let mut pool_hashes = self.pool_tx_hashes.write().await;
            if !pool_hashes.insert(tx_hash) {
                debug!(
                    "Duplicate transaction {} rejected at pool insertion",
                    hex::encode(&tx_hash.0[..8])
                );
                return Err(crate::error::BlockchainError::Validation(
                    "duplicate transaction".to_string(),
                ));
            }
        }

        // Record transaction arrival time for fairness analysis
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        {
            let mut analyzer = self.fairness_analyzer.write().await;
            analyzer.record_transaction_arrival(tx.hash, timestamp);
        }

        // Record arrival in ordering context
        {
            let mut context = self.ordering_context.write().await;
            context.record_arrival(tx.hash, timestamp);
        }

        // EIP-1559: Validate transaction can afford base fee
        // Use Stream A's base fee as the default for mempool validation
        // (transactions are distributed across streams after acceptance)
        let base_fee = self.get_current_base_fee(StreamType::StreamA).await;
        if !can_afford_base_fee(&tx, base_fee) {
            self.pool_tx_hashes.write().await.remove(&tx_hash);
            return Err(crate::error::BlockchainError::Validation(format!(
                "transaction max fee per gas ({}) below base fee ({})",
                tx.max_fee_per_gas
                    .unwrap_or_else(|| tx.fee / tx.gas_limit.max(1) as u128),
                base_fee
            )));
        }

        // QUA-009: Check nonce to determine if tx is ready or should be queued
        let expected_nonce = self.get_sender_nonce(tx.from).await;
        let sender = tx.from;

        if tx.nonce == expected_nonce {
            // Ready to execute - add to pool and promote any unblocked future txs
            // Phase 6 Gap 5: Route transactions to shard_manager when sharding is enabled
            if let Some(shard_manager) = &self.shard_manager {
                // Route to the appropriate shard based on sender address
                shard_manager.add_transaction(tx.clone()).await?;
            } else {
                self.add_to_pool_inner(PooledTransaction::new(tx.clone()))
                    .await;
            }
            // Check if any future txs are now unblocked
            self.promote_future_txs(sender).await;
            return Ok(());
        } else if tx.nonce > expected_nonce
            && tx.nonce <= expected_nonce.saturating_add(MAX_FUTURE_NONCE_GAP)
        {
            // Future nonce within reasonable gap - queue it
            let mut futures = self.future_txs.write().await;
            let sender_queue = futures.entry(sender).or_insert_with(BTreeMap::new);

            // Limit per-sender future queue to prevent memory abuse
            if sender_queue.len() >= MAX_FUTURE_TXS_PER_SENDER {
                // Remove from pool_tx_hashes since we're rejecting
                self.pool_tx_hashes.write().await.remove(&tx_hash);
                return Err(crate::error::BlockchainError::Validation(
                    "too many future-nonce transactions queued for sender".to_string(),
                ));
            }

            // Check if this nonce is already queued
            if sender_queue.contains_key(&tx.nonce) {
                self.pool_tx_hashes.write().await.remove(&tx_hash);
                return Err(crate::error::BlockchainError::Validation(
                    "transaction with this nonce already queued".to_string(),
                ));
            }

            sender_queue.insert(tx.nonce, PooledTransaction::new(tx.clone()));
            debug!(
                "Queued future-nonce tx from {} nonce {} (expected {})",
                hex::encode(&sender.0[..8]),
                tx.nonce,
                expected_nonce
            );
            return Ok(());
        } else if tx.nonce < expected_nonce {
            // Nonce too low - reject
            self.pool_tx_hashes.write().await.remove(&tx_hash);
            return Err(crate::error::BlockchainError::Validation(
                "nonce too low".to_string(),
            ));
        } else {
            // Nonce too far ahead - reject
            self.pool_tx_hashes.write().await.remove(&tx_hash);
            return Err(crate::error::BlockchainError::Validation(
                "nonce too far ahead".to_string(),
            ));
        }
    }

    /// Get fairness metrics for a block
    pub async fn get_fairness_metrics(&self, block: &Block) -> fairness::FairnessMetrics {
        let analyzer = self.fairness_analyzer.read().await;
        analyzer.analyze_block(block)
    }

    /// Get pending transactions count
    pub async fn pending_count(&self) -> usize {
        self.stream_a_pool_size.load(Ordering::Acquire)
            + self.stream_b_pool_size.load(Ordering::Acquire)
            + self.stream_c_pool_size.load(Ordering::Acquire)
    }

    /// Get mempool sizes for all streams (for Prometheus metrics)
    /// Returns (total, stream_a, stream_b, stream_c)
    pub fn get_mempool_sizes(&self) -> (usize, usize, usize, usize) {
        let stream_a = self.stream_a_pool_size.load(Ordering::Acquire);
        let stream_b = self.stream_b_pool_size.load(Ordering::Acquire);
        let stream_c = self.stream_c_pool_size.load(Ordering::Acquire);
        (stream_a + stream_b + stream_c, stream_a, stream_b, stream_c)
    }

    /// Get individual stream pool sizes (for direct access)
    pub fn get_stream_a_pool_size(&self) -> usize {
        self.stream_a_pool_size.load(Ordering::Acquire)
    }

    pub fn get_stream_b_pool_size(&self) -> usize {
        self.stream_b_pool_size.load(Ordering::Acquire)
    }

    pub fn get_stream_c_pool_size(&self) -> usize {
        self.stream_c_pool_size.load(Ordering::Acquire)
    }

    /// Get all transactions from pool as HashMap (for mempool lookup)
    ///
    /// This creates a snapshot of the current transaction pool for compact block reconstruction.
    /// Note: This is a snapshot - transactions may be removed from pool after this call.
    /// QUA-004: Uses read locks for non-destructive iteration - no longer drains pools.
    pub async fn get_mempool_snapshot(
        &self,
    ) -> std::collections::HashMap<crate::types::Hash, Transaction> {
        let mut mempool = std::collections::HashMap::new();

        // Collect from legacy shared pool using read lock (non-destructive)
        let pool = self.tx_pool.read().await;
        for pooled in pool.iter() {
            mempool.insert(pooled.tx.hash, pooled.tx.clone());
        }
        drop(pool);

        // Collect from Stream A pool using read lock (non-destructive)
        let pool = self.stream_a_pool.read().await;
        for pooled in pool.iter() {
            mempool.insert(pooled.tx.hash, pooled.tx.clone());
        }
        drop(pool);

        // Collect from Stream B pool using read lock (non-destructive)
        let pool = self.stream_b_pool.read().await;
        for pooled in pool.iter() {
            mempool.insert(pooled.tx.hash, pooled.tx.clone());
        }
        drop(pool);

        // Collect from Stream C pool using read lock (non-destructive)
        let pool = self.stream_c_pool.read().await;
        for pooled in pool.iter() {
            mempool.insert(pooled.tx.hash, pooled.tx.clone());
        }
        drop(pool);

        mempool
    }

    /// Get mempool transaction hashes as a HashSet (for compact block creation)
    ///
    /// Returns hashes from all pools (legacy + per-stream) for CompactBlock::from_block()
    /// QUA-004: Uses read locks for non-destructive iteration - no longer drains pools.
    pub async fn get_mempool_hashes(&self) -> std::collections::HashSet<crate::types::Hash> {
        let mut hashes = std::collections::HashSet::new();

        // Collect from legacy shared pool using read lock (non-destructive)
        let pool = self.tx_pool.read().await;
        for pooled in pool.iter() {
            hashes.insert(pooled.tx.hash);
        }
        drop(pool);

        // Collect from Stream A pool using read lock (non-destructive)
        let pool = self.stream_a_pool.read().await;
        for pooled in pool.iter() {
            hashes.insert(pooled.tx.hash);
        }
        drop(pool);

        // Collect from Stream B pool using read lock (non-destructive)
        let pool = self.stream_b_pool.read().await;
        for pooled in pool.iter() {
            hashes.insert(pooled.tx.hash);
        }
        drop(pool);

        // Collect from Stream C pool using read lock (non-destructive)
        let pool = self.stream_c_pool.read().await;
        for pooled in pool.iter() {
            hashes.insert(pooled.tx.hash);
        }
        drop(pool);

        hashes
    }

    /// Start mining all streams
    pub async fn start_mining(&self) {
        self.start_mining_streams(true, true, true).await;
    }

    /// Start mining with only Stream A (single-stream mode)
    /// This reduces CPU usage significantly - useful for resource-constrained VPS
    pub async fn start_mining_single_stream(&self) {
        info!("Starting single-stream mining (Stream A only)");
        info!("This mode reduces CPU usage by ~66%");
        // Task 50: Enable single-stream mode for TX distribution
        self.single_stream_mode.store(true, Ordering::Release);
        self.start_mining_streams(true, false, false).await;
    }

    /// Start mining specific streams
    /// stream_a: ASIC mining (Blake3, 10s blocks, 50 IDAG reward)
    /// stream_b: CPU mining (B3MemHash, 5s blocks, 25 IDAG reward) — GPU planned
    /// stream_c: ZK proofs (100ms blocks, fee-based only)
    pub async fn start_mining_streams(&self, stream_a: bool, stream_b: bool, stream_c: bool) {
        *self.is_mining.write().await = true;

        // Sync block allocator from current blockchain height (process_blocks also syncs at start)
        // FIX: Use get_max_block_number() for accurate chain height (not cached atomic)
        {
            let blockchain = self.blockchain.read().await;
            let block_num = blockchain.get_max_block_number();
            let current_height = block_num + 1;
            let (next_available, _, _) = self.block_allocator.get_stats().await;
            if next_available < current_height {
                self.block_allocator.set_next_available(current_height);
            }
        }

        // Clone the MiningManager for each enabled stream
        if stream_a {
            let self_a = self.clone_for_mining();
            tokio::spawn(async move {
                self_a.mine_stream_a().await;
            });
        } else {
            warn!("Stream A disabled");
        }

        if stream_b {
            let self_b = self.clone_for_mining();
            tokio::spawn(async move {
                self_b.mine_stream_b().await;
            });
        } else {
            warn!("Stream B disabled");
        }

        if stream_c {
            warn!("Stream C enabled (higher CPU usage)");
            let self_c = self.clone_for_mining();
            tokio::spawn(async move {
                self_c.mine_stream_c().await;
            });
        } else {
            info!("Stream C disabled (better CPU performance)");
        }

        // Spawn background pruning task
        let blockchain_prune = self.blockchain.clone();
        let is_mining_prune = self.is_mining.clone();
        let prune_interval = self.prune_interval_secs;
        let keep_red = self.keep_red_blocks;
        let batch_size = self.prune_batch_size;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(prune_interval));
            let mut last_pruned_height: u64 = 0;

            loop {
                interval.tick().await;

                // Check if mining is still active
                if !*is_mining_prune.read().await {
                    break;
                }

                let start_time = std::time::Instant::now();
                let mut blocks_pruned: usize = 0;
                let mut bytes_freed: usize = 0;

                // Step 1: Get finalized height and current height
                let (finalized_height, current_height) = {
                    let blockchain = blockchain_prune.read().await;
                    let finalized = blockchain
                        .get_finalized_block_number()
                        .unwrap_or(None)
                        .unwrap_or(0);
                    let current = blockchain.latest_block_number();
                    (finalized, current)
                };

                // Skip if no finalized blocks yet or no progress since last prune
                if finalized_height == 0 || finalized_height <= last_pruned_height {
                    // Still do in-memory pruning of spent outputs
                    {
                        let blockchain = blockchain_prune.read().await;
                        // Prune spent outputs (uses block_in_place internally)
                        drop(blockchain);
                        let blockchain = blockchain_prune.write().await;
                        blockchain.prune_spent_outputs(current_height);
                    }
                    debug!(
                        "Pruning cycle: no new finalized blocks (finalized: {}, last_pruned: {})",
                        finalized_height, last_pruned_height
                    );
                    continue;
                }

                // Step 2: In-memory GhostDAG pruning (existing logic)
                if !keep_red {
                    let blockchain = blockchain_prune.read().await;
                    if let Some(dag) = blockchain.ghostdag() {
                        if let Ok(mut dag) = dag.try_write() {
                            dag.prune_old_blocks(current_height);
                        }
                    }
                }

                // Step 3: Sled disk pruning - delete red blocks below finality
                // Only if keep_red is false
                if !keep_red {
                    // Collect block hashes to check (from storage, in batches)
                    // We scan blocks between last_pruned_height and finalized_height
                    let blocks_to_check: Vec<(crate::types::Hash, u64, usize)> = {
                        let blockchain = blockchain_prune.read().await;
                        blockchain.with_blocks(|all_blocks| {
                            all_blocks
                                .iter()
                                .filter(|b| {
                                    b.header.block_number > last_pruned_height
                                        && b.header.block_number <= finalized_height
                                })
                                .map(|b| {
                                    let estimated_size = b.transactions.len() * 300 + 300;
                                    (b.hash, b.header.block_number, estimated_size)
                                })
                                .collect()
                        })
                    };

                    // Check each block: if NOT blue, delete from sled
                    for chunk in blocks_to_check.chunks(batch_size) {
                        let mut hashes_to_delete: Vec<(crate::types::Hash, usize)> = Vec::new();

                        {
                            let blockchain = blockchain_prune.read().await;
                            for (hash, _block_num, size) in chunk {
                                if !blockchain.is_blue_block(hash) {
                                    hashes_to_delete.push((*hash, *size));
                                }
                            }
                        }

                        // Delete in spawn_blocking to avoid starving tokio
                        if !hashes_to_delete.is_empty() {
                            let blockchain_clone = blockchain_prune.clone();
                            let deleted = tokio::task::spawn_blocking(move || {
                                let mut count = 0usize;
                                let mut freed = 0usize;

                                // Use block_in_place pattern for accessing blockchain from blocking context
                                let rt = tokio::runtime::Handle::current();
                                let blockchain = rt.block_on(blockchain_clone.read());

                                for (hash, size) in &hashes_to_delete {
                                    if blockchain.delete_block_from_storage(hash).unwrap_or(false) {
                                        count += 1;
                                        freed += size;
                                    }
                                }
                                (count, freed)
                            })
                            .await
                            .unwrap_or((0, 0));

                            blocks_pruned += deleted.0;
                            bytes_freed += deleted.1;
                        }

                        // Yield between batches
                        tokio::task::yield_now().await;
                    }
                }

                // Step 4: Prune spent outputs (existing)
                {
                    let blockchain = blockchain_prune.write().await;
                    blockchain.prune_spent_outputs(current_height);
                }

                last_pruned_height = finalized_height;

                let duration = start_time.elapsed();
                if blocks_pruned > 0 {
                    info!(
                        "Pruning cycle: {} red blocks deleted, ~{} bytes freed, took {:?} (finalized height: {})",
                        blocks_pruned, bytes_freed, duration, finalized_height
                    );
                } else {
                    debug!(
                        "Pruning cycle: no red blocks to prune (finalized height: {}, took {:?})",
                        finalized_height, duration
                    );
                }
            }
        });

        info!(
            "Background pruning task started (interval: {}s, keep_red_blocks: {}, batch_size: {})",
            prune_interval, keep_red, batch_size
        );
    }

    /// Stop mining
    pub async fn stop_mining(&self) {
        *self.is_mining.write().await = false;
    }

    /// SYNC-001: Pause mining during Initial Block Download (IBD)
    /// This prevents locally-mined blocks from creating DAG tips that
    /// diverge from the peer's chain, which would cause all synced blocks
    /// to become orphans.
    pub fn pause_for_sync(&self) {
        self.syncing.store(true, Ordering::Release);
        info!("Mining paused during initial block download");
    }

    /// SYNC-001: Resume mining after sync completes
    /// Called when IBD finishes to allow mining to continue.
    pub fn resume_after_sync(&self) {
        self.syncing.store(false, Ordering::Release);
        info!("Mining resumed after sync complete");
    }

    /// SYNC-001: Check if node is currently syncing
    pub fn is_syncing(&self) -> bool {
        self.syncing.load(Ordering::Acquire)
    }

    /// Sync block allocator to current blockchain height
    /// Called after IBD completes to ensure mining starts at correct height
    pub async fn sync_block_allocator_to_chain_height(&self) {
        let blockchain = self.blockchain.read().await;
        let block_num = blockchain.latest_block_number();
        let current_height = block_num + 1;
        drop(blockchain);

        let (next_available, _, _) = self.block_allocator.get_stats().await;
        if next_available < current_height {
            self.block_allocator.set_next_available(current_height);
            info!(
                "[SYNC] Updated block allocator from {} to height {}",
                next_available, current_height
            );
        }
    }

    /// Check if mining is active
    pub fn is_mining(&self) -> &Arc<RwLock<bool>> {
        &self.is_mining
    }

    /// BPR-004: Request graceful shutdown of mining operations
    pub fn shutdown(&self) {
        info!("Mining shutdown requested");
        self.shutdown_token.store(true, Ordering::Relaxed);
    }

    /// Mine Stream A blocks (ASIC, 10s blocks, 10,000 txs, 50 IDAG reward)
    /// ACTUAL PROOF-OF-WORK IMPLEMENTATION (not timer-based)
    async fn mine_stream_a(&self) {
        debug!("Stream A: Starting mining loop");
        // Stagger startup to avoid lock contention
        sleep(Duration::from_millis(100)).await;

        // Track difficulty for adjustment
        let mut current_difficulty = pow::INITIAL_DIFFICULTY_A;

        while *self.is_mining.read().await {
            // BPR-004: Check for shutdown signal
            if self.shutdown_token.load(Ordering::Relaxed) {
                info!("Mining stream A shutting down gracefully");
                break;
            }

            // SYNC-001: Check if we're syncing - skip mining to prevent DAG tip contamination
            if self.syncing.load(Ordering::Acquire) {
                debug!("Stream A: Skipping mining round - sync in progress");
                sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Wait for blockchain to be loaded before mining
            {
                let bc = self.blockchain.read().await;
                if !bc.is_ready() {
                    drop(bc);
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
            }

            // ARC-008: Evict expired transactions before assembling block
            self.evict_expired_transactions().await;

            // eprintln!("🔄 Stream Loop iteration");
            let mining_start = Instant::now();

            // Extract transactions from shard manager (when sharding) or tx_pool (fallback)
            let mut txs = if let (Some(shard_manager), Some(local_shard_id)) =
                (&self.shard_manager, self.local_shard_id)
            {
                // Task #215: Pull from shard-assigned pools when sharding is enabled
                let shard_txs = shard_manager
                    .get_shard_transactions(local_shard_id, STREAM_A_MAX_TXS)
                    .await;

                if shard_txs.is_empty() {
                    // No transactions available, wait and retry
                    sleep(Duration::from_millis(100)).await;
                    continue;
                }

                // Remove transactions from the local shard
                let _ = shard_manager
                    .remove_shard_transactions(local_shard_id, shard_txs.len())
                    .await;

                let mut txs: Vec<Transaction> =
                    shard_txs.into_iter().take(STREAM_A_MAX_TXS).collect();

                // Apply ordering policy
                let policy = *self.ordering_policy.read().await;
                // PER-003: Update atomic timestamp without lock
                self.last_update_time.store(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                    Ordering::Relaxed,
                );
                let mut context = self.ordering_context.write().await;
                txs = ordering::order_transactions(txs, policy, &mut context);
                drop(context);

                txs
            } else {
                // Task 50: Pop transactions from Stream A's dedicated pool (no sharding)
                // ERR-005: Check TTL at dequeue time
                // QUA-004: Use write lock to drain transactions from front of Vec
                // BPR-005: Sort by fee descending (highest fee first) for priority ordering
                let now = Instant::now();
                let count = self
                    .stream_a_pool_size
                    .load(Ordering::Acquire)
                    .min(STREAM_A_MAX_TXS);
                if count > 0 {
                    debug!("Stream A: Found {} txs in pool", count);
                }
                let mut txs = Vec::with_capacity(count);

                // Drain up to count transactions from front of pool
                // ERR-005: Skip expired transactions at dequeue time
                // SEC-015: Skip transactions already in-flight in another stream
                // Task #24: Collect cross-stream cleanup hashes before dropping lock
                let mut to_cleanup: Vec<Hash> = Vec::new();
                {
                    let mut pool = self.stream_a_pool.write().await;

                    // BPR-005: Sort by fee descending (highest fee first) before selection
                    pool.sort_unstable_by(|a, b| b.tx.fee.cmp(&a.tx.fee));

                    let mut drained = 0;
                    while drained < count && !pool.is_empty() {
                        // HIGH-14: Use swap_remove(0) instead of remove(0) for O(1) removal
                        // Pool is sorted by fee descending before this loop, so ordering doesn't matter
                        let pooled = pool.swap_remove(0);
                        let tx_hash = pooled.tx.hash;
                        if pooled.is_expired(now) {
                            warn!(
                                "Stream A: Transaction {} expired after {}s, dropping",
                                hex::encode(tx_hash),
                                now.duration_since(pooled.submitted_at).as_secs()
                            );
                            self.stream_a_pool_size.fetch_sub(1, Ordering::Release);
                            // QUA-003: Remove from pool membership set on expiry
                            self.pool_tx_hashes.write().await.remove(&tx_hash);
                            continue; // Skip expired transaction
                        }

                        // SEC-015: Check if transaction is already being assembled by another stream
                        {
                            let mut in_flight = self.in_flight_txs.write().await;
                            // Periodic cleanup: if set grows too large, clear it to prevent memory leak
                            if in_flight.len() > 10_000 {
                                warn!("In-flight transaction set exceeded 10000, clearing to prevent memory leak");
                                in_flight.clear();
                            }
                            if !in_flight.insert(tx_hash) {
                                // Transaction already being assembled by another stream — skip it
                                debug!("Stream A: Skipping transaction {} — already in-flight in another stream", hex::encode(&tx_hash.0[..8]));
                                self.stream_a_pool_size.fetch_sub(1, Ordering::Release);
                                // QUA-003: Remove from pool membership set since it's being handled by another stream
                                self.pool_tx_hashes.write().await.remove(&tx_hash);
                                continue;
                            }
                        }

                        // Collect hash for cross-stream cleanup after dropping pool lock
                        to_cleanup.push(tx_hash);

                        debug!("Stream A: pulling tx {:?}", tx_hash);
                        txs.push(pooled.tx);
                        self.stream_a_pool_size.fetch_sub(1, Ordering::Release);
                        // QUA-003: Remove from pool membership set on successful dequeue
                        self.pool_tx_hashes.write().await.remove(&tx_hash);
                        drained += 1;
                    }
                } // pool lock dropped here

                // Task #24: Cross-stream cleanup after dropping stream A pool lock
                // SEC-015: Remove from other stream pools to prevent cross-stream double-inclusion
                if !to_cleanup.is_empty() {
                    let cleanup_set: HashSet<Hash> = to_cleanup.iter().copied().collect();
                    {
                        let mut b_pool = self.stream_b_pool.write().await;
                        let before = b_pool.len();
                        b_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - b_pool.len();
                        if removed > 0 {
                            self.stream_b_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                    {
                        let mut c_pool = self.stream_c_pool.write().await;
                        let before = c_pool.len();
                        c_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - c_pool.len();
                        if removed > 0 {
                            self.stream_c_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                }

                // Apply ordering policy if we have transactions
                if !txs.is_empty() {
                    let policy = *self.ordering_policy.read().await;
                    // PER-003: Update atomic timestamp without lock
                    self.last_update_time.store(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        Ordering::Relaxed,
                    );
                    match tokio::time::timeout(
                        Duration::from_millis(10),
                        self.ordering_context.write(),
                    )
                    .await
                    {
                        Ok(mut context) => {
                            txs = ordering::order_transactions(txs, policy, &mut context);
                        }
                        Err(_) => {
                            let mut temp_context = ordering::OrderingContext::new();
                            txs = ordering::order_transactions(txs, policy, &mut temp_context);
                        }
                    }
                }

                txs
            };

            // Use GhostDAG tips for parent selection if available, otherwise fall back to parent coordinator
            let parent_hashes = if let Some(ref ghostdag) = self.ghostdag {
                // Get tips from GhostDAG consensus
                let dag = ghostdag.read().await;
                let tips = dag.get_tips();
                drop(dag);

                if tips.is_empty() {
                    // GhostDAG has no tips yet, fall back to parent coordinator
                    self.parent_coordinator
                        .select_parents(&self.blockchain, StreamType::StreamA)
                        .await
                } else {
                    // Use GhostDAG tips as parents
                    tips
                }
            } else {
                // GhostDAG not set, use parent coordinator (backward compatible)
                self.parent_coordinator
                    .select_parents(&self.blockchain, StreamType::StreamA)
                    .await
            };
            if parent_hashes.is_empty() {
                // No parents available yet (GhostDAG not ready or chain empty)
                sleep(Duration::from_millis(200)).await;
                continue;
            }
            if !parent_hashes.is_empty() {
                let valid = self
                    .parent_coordinator
                    .are_parents_valid(&self.blockchain, &parent_hashes)
                    .await;
                if !valid {
                    // Task #25: Return transactions to Stream A's pool before retrying
                    {
                        let mut pool = self.stream_a_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_a_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // Task #25: Clear in-flight transactions on parent validation failure
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    sleep(Duration::from_millis(100)).await;
                    continue;
                }
            }
            let difficulty = current_difficulty;

            // Reserve block number BEFORE creating the block to ensure hash stability
            let block_number = self.block_allocator.reserve(StreamType::StreamA).await;

            // EIP-1559: Get current base fee for this stream
            let base_fee = self.get_current_base_fee(StreamType::StreamA).await;

            let header_template = BlockHeader::new(
                parent_hashes.clone(),
                block_number,
                StreamType::StreamA,
                difficulty,
                base_fee,
            );
            let header_template_clone = header_template.clone(); // preserve timestamp for final header

            debug!(
                "Stream A: assembling candidate block #{}, txs: {}",
                block_number,
                txs.len()
            );

            debug!(
                "Stream A: assembling candidate block #{}, txs: {}",
                block_number,
                txs.len()
            );

            // Pre-PoW block size validation - prevent wasted mining on oversized blocks
            // QUA-006 + ARC-008: Use shared trim helper with O(log n) binary search
            txs = trim_block_transactions(&header_template_clone, &txs, "Stream A");
            // Calculate hashes and root for trimmed transactions
            let tx_hashes: Vec<Hash> = txs.iter().map(|tx| tx.hash).collect();
            let transactions_root = pow::calculate_transactions_root(&tx_hashes);

            // MINE THE BLOCK - ACTUAL PROOF-OF-WORK
            // This is the critical part: iterate nonce until we find a hash that meets difficulty

            // Run mining in a blocking task (PoW is CPU-intensive)
            let mining_result = tokio::task::spawn_blocking(move || {
                pow::mine_block(
                    &header_template,
                    &transactions_root,
                    StreamType::StreamA,
                    None,
                )
            })
            .await;

            match mining_result {
                Ok(Some((nonce, block_hash))) => {
                    // Mining successful! Create block with found nonce
                    let mut header = header_template_clone.clone();
                    header.nonce = nonce; // keep original timestamp used during PoW
                    let mut block = Block::new(header, txs.clone());

                    // Use the mined hash (from PoW) instead of calculated hash
                    // This ensures the hash matches what was actually mined
                    block.hash = block_hash;

                    // PQ Signature: Sign block header with Dilithium3 if keypair is available
                    if let Some(ref dilithium_keypair) = self.dilithium_keypair {
                        let signing_hash = block.header.calculate_signing_hash();
                        match dilithium_keypair.sign(&signing_hash) {
                            Ok(pq_sig) => {
                                block.header.pq_signature = Some(pq_sig);
                                block.header.miner_pq_pubkey =
                                    Some(dilithium_keypair.public_key().to_vec());
                                debug!(
                                    "Stream A: Block {} signed with Dilithium3 PQ signature",
                                    block_number
                                );
                            }
                            Err(e) => {
                                error!("SEC: Failed to sign block with Dilithium3: {}", e);
                            }
                        }
                    }

                    // Calculate actual block time
                    let actual_time = mining_start.elapsed().as_secs().max(1);

                    // Adjust difficulty for next block
                    current_difficulty = pow::adjust_difficulty(
                        current_difficulty,
                        pow::STREAM_A_TARGET_TIME,
                        actual_time,
                    );

                    // Send block to processor
                    let _ = self.block_sender.send(BlockSubmission {
                        block,
                        stream_type: StreamType::StreamA,
                        block_number,
                        reward: STREAM_A_REWARD,
                        fees: 0,
                    });

                    // Mining stream logging disabled

                    // Block time tracking disabled

                    // Prevent lock starvation in dev mode
                    sleep(Duration::from_millis(100)).await;
                }
                Ok(None) => {
                    error!(
                        "Stream A: Mining failed for block #{} (difficulty too high?)",
                        block_number
                    );
                    // Release the reserved block number since mining failed
                    self.block_allocator
                        .release(block_number, StreamType::StreamA)
                        .await;
                    // Task 50: Return transactions to Stream A's pool (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    {
                        let mut pool = self.stream_a_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_a_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on mining failure
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    // Wait a bit before retrying
                    sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    error!("Stream A: Mining error: {:?}", e);
                    // Release the reserved block number since mining failed
                    self.block_allocator
                        .release(block_number, StreamType::StreamA)
                        .await;
                    // Task 50: Return transactions to Stream A's pool (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    {
                        let mut pool = self.stream_a_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_a_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on mining error
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Mine Stream B blocks (CPU, 1s blocks, 5,000 txs, 25 IDAG reward) — GPU via OpenCL planned
    /// ACTUAL PROOF-OF-WORK IMPLEMENTATION (not timer-based)
    async fn mine_stream_b(&self) {
        debug!("Stream B: Starting mining loop");
        // Stagger startup to avoid lock contention
        sleep(Duration::from_millis(200)).await;

        // Track difficulty for adjustment
        let mut current_difficulty = pow::INITIAL_DIFFICULTY_B;
        // Use wall-clock time between successful B blocks for DAA — not per-batch time.
        // Per-batch time is floored at 1s by .max(1), causing difficulty to always
        // increase on fast solves regardless of the true inter-block interval.
        let mut last_b_block_time = Instant::now();

        while *self.is_mining.read().await {
            // BPR-004: Check for shutdown signal
            if self.shutdown_token.load(Ordering::Relaxed) {
                info!("Mining stream B shutting down gracefully");
                break;
            }

            // SYNC-001: Check if we're syncing - skip mining to prevent DAG tip contamination
            if self.syncing.load(Ordering::Acquire) {
                debug!("Stream B: Skipping mining round - sync in progress");
                sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Wait for blockchain to be loaded before mining
            {
                let bc = self.blockchain.read().await;
                if !bc.is_ready() {
                    drop(bc);
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
            }

            // ARC-008: Evict expired transactions before assembling block
            self.evict_expired_transactions().await;

            debug!("Stream B: Loop iteration (checking for txs)");
            let mining_start = Instant::now();
            let mut txs = if let Some(shard_manager) = &self.shard_manager {
                // Get transactions from all shards
                let mut all_txs = Vec::new();
                let shard_count = shard_manager.shard_count();
                let txs_per_shard = STREAM_B_MAX_TXS / shard_count.max(1);

                for shard_id in 0..shard_count {
                    let shard_txs = shard_manager
                        .get_shard_transactions(shard_id, txs_per_shard)
                        .await;
                    all_txs.extend(shard_txs);
                    if all_txs.len() >= STREAM_B_MAX_TXS {
                        break;
                    }
                }

                if all_txs.is_empty() {
                    sleep(Duration::from_secs(pow::STREAM_B_TARGET_TIME)).await;
                    continue;
                }

                // Remove transactions from shards
                for shard_id in 0..shard_count {
                    let _ = shard_manager
                        .remove_shard_transactions(shard_id, txs_per_shard)
                        .await;
                }

                let mut txs: Vec<Transaction> =
                    all_txs.into_iter().take(STREAM_B_MAX_TXS).collect();

                // Apply ordering policy
                let policy = *self.ordering_policy.read().await;
                // PER-003: Update atomic timestamp without lock
                self.last_update_time.store(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64,
                    Ordering::Relaxed,
                );
                let mut context = self.ordering_context.write().await;
                txs = ordering::order_transactions(txs, policy, &mut context);
                drop(context);

                txs
            } else {
                // Task 50: Pop transactions from Stream B's dedicated pool (no sharding)
                // ERR-005: Check TTL at dequeue time
                // QUA-004: Use write lock to drain transactions from front of Vec
                // BPR-005: Sort by fee descending (highest fee first) for priority ordering
                let now = Instant::now();
                let count = self
                    .stream_b_pool_size
                    .load(Ordering::Acquire)
                    .min(STREAM_B_MAX_TXS);
                if count > 0 {
                    debug!("Stream B: Found {} txs in pool", count);
                }
                let mut txs = Vec::with_capacity(count);

                // Drain up to count transactions from front of pool
                // ERR-005: Skip expired transactions at dequeue time
                // SEC-015: Skip transactions already in-flight in another stream
                // Task #24: Collect cross-stream cleanup hashes before dropping lock
                let mut to_cleanup: Vec<Hash> = Vec::new();
                {
                    let mut pool = self.stream_b_pool.write().await;

                    // BPR-005: Sort by fee descending (highest fee first) before selection
                    pool.sort_unstable_by(|a, b| b.tx.fee.cmp(&a.tx.fee));

                    let mut drained = 0;
                    while drained < count && !pool.is_empty() {
                        // HIGH-14: Use swap_remove(0) instead of remove(0) for O(1) removal
                        // Pool is sorted by fee descending before this loop, so ordering doesn't matter
                        let pooled = pool.swap_remove(0);
                        let tx_hash = pooled.tx.hash;
                        if pooled.is_expired(now) {
                            warn!(
                                "Stream B: Transaction {} expired after {}s, dropping",
                                hex::encode(tx_hash),
                                now.duration_since(pooled.submitted_at).as_secs()
                            );
                            self.stream_b_pool_size.fetch_sub(1, Ordering::Release);
                            // QUA-003: Remove from pool membership set on expiry
                            self.pool_tx_hashes.write().await.remove(&tx_hash);
                            continue; // Skip expired transaction
                        }

                        // SEC-015: Check if transaction is already being assembled by another stream
                        {
                            let mut in_flight = self.in_flight_txs.write().await;
                            // Periodic cleanup: if set grows too large, clear it to prevent memory leak
                            if in_flight.len() > 10_000 {
                                warn!("In-flight transaction set exceeded 10000, clearing to prevent memory leak");
                                in_flight.clear();
                            }
                            if !in_flight.insert(tx_hash) {
                                // Transaction already being assembled by another stream — skip it
                                debug!("Stream B: Skipping transaction {} — already in-flight in another stream", hex::encode(&tx_hash.0[..8]));
                                self.stream_b_pool_size.fetch_sub(1, Ordering::Release);
                                // QUA-003: Remove from pool membership set since it's being handled by another stream
                                self.pool_tx_hashes.write().await.remove(&tx_hash);
                                continue;
                            }
                        }

                        // Collect hash for cross-stream cleanup after dropping pool lock
                        to_cleanup.push(tx_hash);

                        debug!("Stream B: pulling tx {:?}", tx_hash);
                        txs.push(pooled.tx);
                        self.stream_b_pool_size.fetch_sub(1, Ordering::Release);
                        // QUA-003: Remove from pool membership set on successful dequeue
                        self.pool_tx_hashes.write().await.remove(&tx_hash);
                        drained += 1;
                    }
                } // pool lock dropped here

                // Task #24: Cross-stream cleanup after dropping stream B pool lock
                // SEC-015: Remove from other stream pools to prevent cross-stream double-inclusion
                if !to_cleanup.is_empty() {
                    let cleanup_set: HashSet<Hash> = to_cleanup.iter().copied().collect();
                    {
                        let mut a_pool = self.stream_a_pool.write().await;
                        let before = a_pool.len();
                        a_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - a_pool.len();
                        if removed > 0 {
                            self.stream_a_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                    {
                        let mut c_pool = self.stream_c_pool.write().await;
                        let before = c_pool.len();
                        c_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - c_pool.len();
                        if removed > 0 {
                            self.stream_c_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                }

                // Apply ordering policy if we have transactions
                // Use try_write to avoid deadlock - if context is busy, skip ordering update
                if !txs.is_empty() {
                    let policy = *self.ordering_policy.read().await;
                    // PER-003: Update atomic timestamp without lock
                    self.last_update_time.store(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        Ordering::Relaxed,
                    );
                    // Use timeout to avoid deadlock - if context is busy, skip update
                    match tokio::time::timeout(
                        Duration::from_millis(10),
                        self.ordering_context.write(),
                    )
                    .await
                    {
                        Ok(mut context) => {
                            txs = ordering::order_transactions(txs, policy, &mut context);
                        }
                        Err(_) => {
                            // Context is busy, order without updating context (non-critical)
                            let mut temp_context = ordering::OrderingContext::new();
                            txs = ordering::order_transactions(txs, policy, &mut temp_context);
                        }
                    }
                }

                txs
            };

            let parent_hashes = if let Some(ref ghostdag) = self.ghostdag {
                let dag = ghostdag.read().await;
                let tips = dag.get_tips();
                drop(dag);
                if tips.is_empty() {
                    self.parent_coordinator
                        .select_parents(&self.blockchain, StreamType::StreamB)
                        .await
                } else {
                    tips
                }
            } else {
                self.parent_coordinator
                    .select_parents(&self.blockchain, StreamType::StreamB)
                    .await
            };
            if parent_hashes.is_empty() {
                // No parents available yet (GhostDAG not ready or chain empty)
                sleep(Duration::from_millis(200)).await;
                continue;
            }
            if !parent_hashes.is_empty() {
                let valid = self
                    .parent_coordinator
                    .are_parents_valid(&self.blockchain, &parent_hashes)
                    .await;
                if !valid {
                    // Task #25: Return transactions to Stream B's pool before retrying
                    {
                        let mut pool = self.stream_b_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_b_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // Task #25: Clear in-flight transactions on parent validation failure
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    sleep(Duration::from_millis(100)).await;
                    continue;
                }
            }
            let difficulty = current_difficulty;

            // Reserve block number BEFORE creating the block to ensure hash stability
            let block_number = self.block_allocator.reserve(StreamType::StreamB).await;

            // EIP-1559: Get current base fee for this stream
            let base_fee = self.get_current_base_fee(StreamType::StreamB).await;

            let header_template = BlockHeader::new(
                parent_hashes.clone(),
                block_number,
                StreamType::StreamB,
                difficulty,
                base_fee,
            );
            let header_template_clone = header_template.clone();

            // Pre-PoW block size validation - prevent wasted mining on oversized blocks
            // QUA-006 + ARC-008: Use shared trim helper with O(log n) binary search
            txs = trim_block_transactions(&header_template_clone, &txs, "Stream B");
            // Calculate hashes and root for trimmed transactions
            let tx_hashes: Vec<Hash> = txs.iter().map(|tx| tx.hash).collect();
            let transactions_root = pow::calculate_transactions_root(&tx_hashes);

            // MINE THE BLOCK - ACTUAL PROOF-OF-WORK using configured mining backend
            // Mining stream logging disabled

            // Clone the backend Arc for use in spawn_blocking
            let backend = self.mining_backend.clone();
            let mining_result = tokio::task::spawn_blocking(move || {
                backend.mine(&header_template, &transactions_root, None)
            })
            .await;

            match mining_result {
                Ok(Some((nonce, block_hash))) => {
                    // Mining successful!
                    let mut header = header_template_clone.clone();
                    header.nonce = nonce;
                    let mut block = Block::new(header, txs.clone());

                    // Use the mined hash (from PoW) instead of calculated hash
                    block.hash = block_hash;

                    // PQ Signature: Sign block header with Dilithium3 if keypair is available
                    if let Some(ref dilithium_keypair) = self.dilithium_keypair {
                        let signing_hash = block.header.calculate_signing_hash();
                        match dilithium_keypair.sign(&signing_hash) {
                            Ok(pq_sig) => {
                                block.header.pq_signature = Some(pq_sig);
                                block.header.miner_pq_pubkey =
                                    Some(dilithium_keypair.public_key().to_vec());
                                debug!(
                                    "Stream B: Block {} signed with Dilithium3 PQ signature",
                                    block_number
                                );
                            }
                            Err(e) => {
                                error!("SEC: Failed to sign block with Dilithium3: {}", e);
                            }
                        }
                    }

                    // Use inter-block wall time, not per-batch time.
                    // Per-batch time is floored at 1s by as_secs(), causing the DAA
                    // to see every fast solve as "1s < 5s target" and increase
                    // difficulty toward MAX. Inter-block time includes all Ok(None)
                    // retry cycles, giving the DAA the true signal it needs.
                    let actual_time = last_b_block_time.elapsed().as_secs().max(1);
                    last_b_block_time = Instant::now();
                    current_difficulty = pow::adjust_difficulty(
                        current_difficulty,
                        pow::STREAM_B_TARGET_TIME,
                        actual_time,
                    );

                    let _ = self.block_sender.send(BlockSubmission {
                        block,
                        stream_type: StreamType::StreamB,
                        block_number,
                        reward: STREAM_B_REWARD,
                        fees: 0,
                    });

                    // Mining stream logging disabled

                    // Block time tracking disabled

                    // Prevent lock starvation in dev mode
                    sleep(Duration::from_millis(100)).await;
                }
                Ok(None) => {
                    // 8 000-nonce batch found no solution — difficulty is too high.
                    // Reduce difficulty now so the next batch is likely to succeed,
                    // then refresh the header timestamp.
                    current_difficulty = pow::adjust_difficulty(
                        current_difficulty,
                        pow::STREAM_B_TARGET_TIME,
                        pow::STREAM_B_TARGET_TIME * 6, // 30s virtual → 0.667× per miss
                    );
                    debug!(
                        "Stream B: No solution, reducing difficulty to {} for block #{}",
                        current_difficulty, block_number
                    );
                    self.block_allocator
                        .release(block_number, StreamType::StreamB)
                        .await;
                    // Task 50: Return transactions to Stream B's pool (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    {
                        let mut pool = self.stream_b_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_b_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on mining failure
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    // Minimal pause; outer loop will get a fresh reservation + fresh timestamp
                    sleep(Duration::from_millis(10)).await;
                }
                Err(e) => {
                    error!("Stream B: Mining error: {:?}", e);
                    // Release the reserved block number since mining failed
                    self.block_allocator
                        .release(block_number, StreamType::StreamB)
                        .await;
                    // Task 50: Return transactions to Stream B's pool (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    {
                        let mut pool = self.stream_b_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_b_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on mining error
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Mine Stream C blocks (ZK, 100ms blocks, 1,000 txs, fee-based only)
    async fn mine_stream_c(&self) {
        debug!("Stream C: Starting mining loop");
        // Stagger startup to avoid lock contention
        sleep(Duration::from_millis(300)).await;

        while *self.is_mining.read().await {
            // BPR-004: Check for shutdown signal
            if self.shutdown_token.load(Ordering::Relaxed) {
                info!("Mining stream C shutting down gracefully");
                break;
            }

            // SYNC-001: Check if we're syncing - skip mining to prevent DAG tip contamination
            if self.syncing.load(Ordering::Acquire) {
                debug!("Stream C: Skipping mining round - sync in progress");
                sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Wait for blockchain to be loaded before mining
            {
                let bc = self.blockchain.read().await;
                if !bc.is_ready() {
                    drop(bc);
                    sleep(Duration::from_millis(500)).await;
                    continue;
                }
            }

            // ARC-008: Evict expired transactions before assembling block
            self.evict_expired_transactions().await;

            debug!("Stream C: Loop iteration (checking for txs)");
            // Extract transactions from shard manager (when sharding) or tx_pool (fallback)
            let txs = if let (Some(shard_manager), Some(local_shard_id)) =
                (&self.shard_manager, self.local_shard_id)
            {
                // Task #215: Pull from shard-assigned pools when sharding is enabled
                let shard_txs = shard_manager
                    .get_shard_transactions(local_shard_id, STREAM_C_MAX_TXS)
                    .await;

                if shard_txs.is_empty() {
                    // No transactions available, continue with empty block
                    Vec::new()
                } else {
                    // Remove transactions from the local shard
                    let _ = shard_manager
                        .remove_shard_transactions(local_shard_id, shard_txs.len())
                        .await;

                    let mut txs: Vec<Transaction> =
                        shard_txs.into_iter().take(STREAM_C_MAX_TXS).collect();

                    // Apply ordering policy
                    let policy = *self.ordering_policy.read().await;
                    // PER-003: Update atomic timestamp without lock
                    self.last_update_time.store(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        Ordering::Relaxed,
                    );
                    let mut context = self.ordering_context.write().await;
                    txs = ordering::order_transactions(txs, policy, &mut context);
                    drop(context);

                    txs
                }
            } else {
                // Task 50: Pop transactions from Stream C's dedicated pool (no sharding)
                // ERR-005: Check TTL at dequeue time
                // QUA-004: Use write lock to drain transactions from front of Vec
                // BPR-005: Sort by fee descending (highest fee first) for priority ordering
                let now = Instant::now();
                let count = self
                    .stream_c_pool_size
                    .load(Ordering::Acquire)
                    .min(STREAM_C_MAX_TXS);
                if count > 0 {
                    debug!("Stream C: Found {} txs in pool", count);
                }
                let mut txs = Vec::with_capacity(count);

                // Drain up to count transactions from front of pool
                // ERR-005: Skip expired transactions at dequeue time
                // SEC-015: Skip transactions already in-flight in another stream
                // Task #24: Collect cross-stream cleanup hashes before dropping lock
                let mut to_cleanup: Vec<Hash> = Vec::new();
                {
                    let mut pool = self.stream_c_pool.write().await;

                    // BPR-005: Sort by fee descending (highest fee first) before selection
                    pool.sort_unstable_by(|a, b| b.tx.fee.cmp(&a.tx.fee));

                    let mut drained = 0;
                    while drained < count && !pool.is_empty() {
                        // HIGH-14: Use swap_remove(0) instead of remove(0) for O(1) removal
                        // Pool is sorted by fee descending before this loop, so ordering doesn't matter
                        let pooled = pool.swap_remove(0);
                        let tx_hash = pooled.tx.hash;
                        if pooled.is_expired(now) {
                            warn!(
                                "Stream C: Transaction {} expired after {}s, dropping",
                                hex::encode(tx_hash),
                                now.duration_since(pooled.submitted_at).as_secs()
                            );
                            self.stream_c_pool_size.fetch_sub(1, Ordering::Release);
                            // QUA-003: Remove from pool membership set on expiry
                            self.pool_tx_hashes.write().await.remove(&tx_hash);
                            continue; // Skip expired transaction
                        }

                        // SEC-015: Check if transaction is already being assembled by another stream
                        {
                            let mut in_flight = self.in_flight_txs.write().await;
                            // Periodic cleanup: if set grows too large, clear it to prevent memory leak
                            if in_flight.len() > 10_000 {
                                warn!("In-flight transaction set exceeded 10000, clearing to prevent memory leak");
                                in_flight.clear();
                            }
                            if !in_flight.insert(tx_hash) {
                                // Transaction already being assembled by another stream — skip it
                                debug!("Stream C: Skipping transaction {} — already in-flight in another stream", hex::encode(&tx_hash.0[..8]));
                                self.stream_c_pool_size.fetch_sub(1, Ordering::Release);
                                // QUA-003: Remove from pool membership set since it's being handled by another stream
                                self.pool_tx_hashes.write().await.remove(&tx_hash);
                                continue;
                            }
                        }

                        // Collect hash for cross-stream cleanup after dropping pool lock
                        to_cleanup.push(tx_hash);

                        debug!("Stream C: pulling tx {:?}", tx_hash);
                        txs.push(pooled.tx);
                        self.stream_c_pool_size.fetch_sub(1, Ordering::Release);
                        // QUA-003: Remove from pool membership set on successful dequeue
                        self.pool_tx_hashes.write().await.remove(&tx_hash);
                        drained += 1;
                    }
                } // pool lock dropped here

                // Task #24: Cross-stream cleanup after dropping stream C pool lock
                // SEC-015: Remove from other stream pools to prevent cross-stream double-inclusion
                if !to_cleanup.is_empty() {
                    let cleanup_set: HashSet<Hash> = to_cleanup.iter().copied().collect();
                    {
                        let mut a_pool = self.stream_a_pool.write().await;
                        let before = a_pool.len();
                        a_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - a_pool.len();
                        if removed > 0 {
                            self.stream_a_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                    {
                        let mut b_pool = self.stream_b_pool.write().await;
                        let before = b_pool.len();
                        b_pool.retain(|p| !cleanup_set.contains(&p.tx.hash));
                        let removed = before - b_pool.len();
                        if removed > 0 {
                            self.stream_b_pool_size.fetch_sub(removed, Ordering::Release);
                        }
                    }
                }

                // Apply ordering policy if we have transactions
                // Use try_write to avoid deadlock - if context is busy, skip ordering update
                if !txs.is_empty() {
                    let policy = *self.ordering_policy.read().await;
                    // PER-003: Update atomic timestamp without lock
                    self.last_update_time.store(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        Ordering::Relaxed,
                    );
                    // Use timeout to avoid deadlock - if context is busy, skip update
                    match tokio::time::timeout(
                        Duration::from_millis(10),
                        self.ordering_context.write(),
                    )
                    .await
                    {
                        Ok(mut context) => {
                            txs = ordering::order_transactions(txs, policy, &mut context);
                        }
                        Err(_) => {
                            // Context is busy, order without updating context (non-critical)
                            let mut temp_context = ordering::OrderingContext::new();
                            txs = ordering::order_transactions(txs, policy, &mut temp_context);
                        }
                    }
                }

                txs
            };

            let total_fees: u128 = txs.iter().map(|tx| tx.fee).sum();

            // Use GhostDAG tips for parent selection if available, otherwise fall back to parent coordinator
            let parent_hashes = if let Some(ref ghostdag) = self.ghostdag {
                let dag = ghostdag.read().await;
                let tips = dag.get_tips();
                drop(dag);
                if tips.is_empty() {
                    self.parent_coordinator
                        .select_parents(&self.blockchain, StreamType::StreamC)
                        .await
                } else {
                    tips
                }
            } else {
                self.parent_coordinator
                    .select_parents(&self.blockchain, StreamType::StreamC)
                    .await
            };
            if parent_hashes.is_empty() {
                // No parents available yet (GhostDAG not ready or chain empty)
                sleep(Duration::from_millis(200)).await;
                continue;
            }
            if !parent_hashes.is_empty() {
                let valid = self
                    .parent_coordinator
                    .are_parents_valid(&self.blockchain, &parent_hashes)
                    .await;
                if !valid {
                    // Task 50: Return transactions to Stream C's pool before retrying (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    {
                        let mut pool = self.stream_c_pool.write().await;
                        for tx in &txs {
                            pool.push(PooledTransaction::new(tx.clone()));
                            self.stream_c_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on parent validation failure
                    {
                        let mut in_flight = self.in_flight_txs.write().await;
                        for tx in &txs {
                            in_flight.remove(&tx.hash);
                        }
                    }
                    sleep(Duration::from_millis(50)).await;
                    continue;
                }
            }

            // Reserve block number BEFORE creating the block to ensure hash stability
            let block_number = self.block_allocator.reserve(StreamType::StreamC).await;

            // EIP-1559: Get current base fee for this stream
            let base_fee = self.get_current_base_fee(StreamType::StreamC).await;

            let header = BlockHeader::new(
                parent_hashes.clone(),
                block_number,
                StreamType::StreamC,
                4,
                base_fee,
            );

            // Pre-submission block size validation (Stream C has no PoW, but still enforce size limit)
            // QUA-006 + ARC-008: Use shared trim helper with O(log n) binary search
            let trimmed_txs = trim_block_transactions(&header, &txs, "Stream C");
            let mut block = Block::new(header.clone(), trimmed_txs);

            // Generate ZK proof for state transition if proving key is available
            #[cfg(feature = "privacy")]
            {
                if let Some(ref pk) = self.zk_proving_key {
                    // Clone data for spawn_blocking
                    let pk_clone = pk.clone();
                    let txs_for_proof = txs.clone();

                    // Clone blockchain reference for the blocking task
                    let blockchain_for_proof = self.blockchain.clone();

                    // Generate ZK proof in blocking task
                    let proof_result = tokio::task::spawn_blocking(move || {
                        // Calculate transactions root for public input
                        let tx_hashes: Vec<Hash> = txs_for_proof.iter().map(|tx| tx.hash).collect();
                        let transactions_root = crate::pow::calculate_transactions_root(&tx_hashes);

                        // Get current state root from Verkle tree before executing transactions
                        // Use blocking_read since we're in a spawn_blocking context
                        // Collect balances and Verkle proofs for circuit witnesses
                        let (pre_state_hash, balances, proofs) = {
                            let blockchain = blockchain_for_proof.blocking_read();
                            let state_root = blockchain.state_root();

                            // Collect all balances and Verkle proofs needed for the circuit
                            let mut balances = Vec::with_capacity(txs_for_proof.len());
                            let mut proofs = Vec::with_capacity(txs_for_proof.len());
                            for tx in &txs_for_proof {
                                let sender_balance = blockchain.get_balance(tx.from);
                                let receiver_balance = blockchain.get_balance(tx.to);
                                balances.push((sender_balance, receiver_balance));

                                // Get Verkle proofs for sender and receiver balance authentication
                                let sender_proof = blockchain.get_balance_with_proof(tx.from);
                                let receiver_proof = blockchain.get_balance_with_proof(tx.to);
                                proofs.push((sender_proof, receiver_proof));
                            }

                            (state_root, balances, proofs)
                        };

                        // Use real state root if available, otherwise fallback to placeholder
                        let pre_state_hash: [u8; 32] = match pre_state_hash {
                            Some(root) => root.0,
                            None => {
                                tracing::warn!("Verkle state not available, using fallback hash");
                                let mut hasher = Keccak256::new();
                                hasher.update(b"pre");
                                let hash = hasher.finalize();
                                let mut result = [0u8; 32];
                                result.copy_from_slice(&hash);
                                result
                            }
                        };

                        // For post_state_hash: create a deterministic commitment from pre_state + transactions_root
                        let post_state_hash: [u8; 32] = {
                            let mut hasher = Keccak256::new();
                            hasher.update(&pre_state_hash);
                            hasher.update(&transactions_root.0);
                            let hash = hasher.finalize();
                            let mut result = [0u8; 32];
                            result.copy_from_slice(&hash);
                            result
                        };

                        // Create simplified circuit for batch balance conservation
                        // Circuit now uses single constraint regardless of transaction count
                        // Milestone 7.3: Enable Verkle path verification for balance authentication
                        let num_txs = txs_for_proof.len();
                        let mut circuit = StateTransitionCircuit::new_batch_with_verkle(num_txs.max(1));

                        // Set transaction amounts and real balances for batch total computation
                        for (i, tx) in txs_for_proof.iter().enumerate().take(num_txs) {
                            let amount = Fr::from(tx.value);
                            let (sender_balance, receiver_balance) = balances.get(i).copied().unwrap_or((0, 0));
                            let sender_fr = Fr::from(sender_balance);
                            let receiver_fr = Fr::from(receiver_balance);
                            circuit.set_transaction(i, amount, sender_fr, receiver_fr);

                            // Milestone 7.3: Set Verkle paths for balance authentication
                            // Extract real Verkle proof paths from the state tree
                            let (sender_proof_opt, receiver_proof_opt) = proofs.get(i).cloned().unwrap_or((None, None));

                            // Convert proof to sibling hashes and indices for sender
                            let (sender_siblings, sender_indices) = match sender_proof_opt {
                                Some((_, state_proof)) => {
                                    // Convert proof hashes to field elements
                                    let siblings: Vec<Fr> = state_proof.proof.iter()
                                        .map(|h| crate::verkle::hash_to_field_element(&h.0))
                                        .collect();
                                    // Derive indices from address bytes (first 4 bytes of 20-byte address)
                                    let mut sibs = [Fr::from(0u64); 4];
                                    let mut idxs = [0u8; 4];
                                    for (j, s) in siblings.iter().enumerate().take(4) {
                                        sibs[j] = *s;
                                    }
                                    for (j, idx) in tx.from.iter().enumerate().take(4) {
                                        idxs[j] = *idx;
                                    }
                                    (sibs, idxs)
                                }
                                None => {
                                    // Fallback: use placeholder if proof not available
                                    ([Fr::from(0u64); 4], [0u8; 4])
                                }
                            };

                            // Convert proof to sibling hashes and indices for receiver
                            let (receiver_siblings, receiver_indices) = match receiver_proof_opt {
                                Some((_, state_proof)) => {
                                    let siblings: Vec<Fr> = state_proof.proof.iter()
                                        .map(|h| crate::verkle::hash_to_field_element(&h.0))
                                        .collect();
                                    let mut sibs = [Fr::from(0u64); 4];
                                    let mut idxs = [0u8; 4];
                                    for (j, s) in siblings.iter().enumerate().take(4) {
                                        sibs[j] = *s;
                                    }
                                    for (j, idx) in tx.to.iter().enumerate().take(4) {
                                        idxs[j] = *idx;
                                    }
                                    (sibs, idxs)
                                }
                                None => ([Fr::from(0u64); 4], [0u8; 4])
                            };

                            let sender_path = VerklePathWitness::from_proof(
                                sender_fr,
                                &sender_siblings,
                                &sender_indices,
                            );
                            let receiver_path = VerklePathWitness::from_proof(
                                receiver_fr,
                                &receiver_siblings,
                                &receiver_indices,
                            );
                            circuit.set_sender_verkle_path(i, sender_path);
                            circuit.set_receiver_verkle_path(i, receiver_path);
                        }

                        // Set public inputs (convert hashes to field elements)
                        let pre_state_fr = Fr::from_be_bytes_mod_order(&pre_state_hash);
                        let post_state_fr = Fr::from_be_bytes_mod_order(&post_state_hash);
                        let tx_root_fr = Fr::from_be_bytes_mod_order(&transactions_root);
                        circuit.set_public_inputs(pre_state_fr, post_state_fr, tx_root_fr);

                        tracing::debug!(
                            "Stream C ZK: using real Verkle proof paths for {} sender and {} receiver balances",
                            txs_for_proof.len(), txs_for_proof.len()
                        );

                        // Generate proof
                        prove_state_transition(&pk_clone, circuit)
                    }).await;

                    match proof_result {
                        Ok(Ok(proof_bytes)) => {
                            block.zk_proof = Some(proof_bytes);
                            tracing::debug!(
                                "ZK proof generated for Stream C block {}",
                                block_number
                            );
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("ZK proof generation failed: {}", e);
                        }
                        Err(e) => {
                            tracing::warn!("ZK proof task panicked: {}", e);
                        }
                    }
                }
            }

            // PQ Signature: Sign block header with Dilithium3 if keypair is available
            if let Some(ref dilithium_keypair) = self.dilithium_keypair {
                let signing_hash = block.header.calculate_signing_hash();
                match dilithium_keypair.sign(&signing_hash) {
                    Ok(pq_sig) => {
                        block.header.pq_signature = Some(pq_sig);
                        block.header.miner_pq_pubkey =
                            Some(dilithium_keypair.public_key().to_vec());
                        debug!(
                            "Stream C: Block {} signed with Dilithium3 PQ signature",
                            block_number
                        );
                    }
                    Err(e) => {
                        error!("SEC: Failed to sign block with Dilithium3: {}", e);
                    }
                }
            }

            // Send block to processor via channel (non-blocking, eliminates deadlock)
            let _ = self.block_sender.send(BlockSubmission {
                block,
                stream_type: StreamType::StreamC,
                block_number,
                reward: 0, // Stream C is fee-based only
                fees: total_fees,
            });

            // Mining stream logging disabled

            sleep(STREAM_C_BLOCK_TIME).await;
        }
    }
}

/// Maximum allowed gap between new block number and current blockchain height
const MAX_BLOCK_NUMBER_GAP: u64 = 50;

/// Process blocks from channel - serializes block additions to prevent deadlock
async fn process_blocks(
    mut receiver: mpsc::UnboundedReceiver<BlockSubmission>,
    blockchain: Arc<RwLock<Blockchain>>,
    miner_address: Address,
    fairness_analyzer: Arc<tokio::sync::RwLock<fairness::FairnessAnalyzer>>,
    metrics: Option<crate::metrics::MetricsHandle>,
    node_registry: Option<Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>>,
    node_identity: Option<crate::governance::NodeIdentity>,
    tx_pool: Arc<RwLock<Vec<PooledTransaction>>>,
    tx_pool_size: Arc<AtomicUsize>,
    network: Arc<RwLock<Option<Arc<crate::network::NetworkManager>>>>,
    block_allocator: Arc<BlockNumberAllocator>,
    _parent_coordinator: Arc<ParentHashCoordinator>,
    shard_manager: Option<Arc<ShardManager>>,
    in_flight_txs: Arc<RwLock<HashSet<Hash>>>, // SEC-015: In-flight transactions to clear on commit
    pool_tx_hashes: Arc<RwLock<HashSet<Hash>>>, // QUA-003: Pool membership set to clear on commit
    future_txs: Arc<RwLock<HashMap<Address, BTreeMap<u64, PooledTransaction>>>>, // QUA-009: Future-nonce queue
) {
    // Note: For Task 50, we keep using the legacy tx_pool for recovery
    // since the process_blocks function doesn't have access to per-stream pools.
    // Transactions are recovered to the legacy pool and will be redistributed
    // when add_transaction is called again.
    // Sync allocator from latest block number in blockchain
    // FIX: Use get_max_block_number() for accurate chain height (not cached atomic)
    {
        let blockchain = blockchain.read().await;
        let max_block_num = blockchain.get_max_block_number();
        block_allocator.set_next_available(max_block_num + 1);
    }

    while let Some(submission) = receiver.recv().await {
        let BlockSubmission {
            block,
            stream_type,
            block_number,
            reward: _,
            fees,
        } = submission;

        // Block number was already reserved by the mining stream before hashing
        // Verify the block number matches what was reserved
        let actual_block_number = block.header.block_number;
        if actual_block_number != block_number {
            warn!(
                "MiningManager: Block number mismatch (expected {}, got {})",
                block_number, actual_block_number
            );
            // Release the reservation since we're rejecting this block
            block_allocator.release(block_number, stream_type).await;
            // Return transactions to pool (ERR-005: wrap with timestamp)
            // QUA-004: Use write lock to return transactions
            if !block.transactions.is_empty() {
                let mut pool = tx_pool.write().await;
                for tx in &block.transactions {
                    pool.push(PooledTransaction::new(tx.clone()));
                    tx_pool_size.fetch_add(1, Ordering::Release);
                }
            }
            // SEC-015: Clear in-flight transactions on block number mismatch
            let mut in_flight = in_flight_txs.write().await;
            for tx in &block.transactions {
                in_flight.remove(&tx.hash);
            }
            continue;
        }

        // Calculate actual reward based on block height and halving schedule
        let actual_reward = get_block_reward(block_number, stream_type);

        // Validate block number gap to prevent excessive jumps
        // FIX: Use get_max_block_number() for accurate chain height (not cached atomic)
        let current_height = {
            let blockchain = blockchain.read().await;
            blockchain.get_max_block_number()
        };
        if block_number > current_height + MAX_BLOCK_NUMBER_GAP {
            error!("MiningManager: Block number {} too far ahead of current height {} — resetting allocator to {}",
                   block_number, current_height, current_height + 1);
            // Reset the allocator to current_height + 1 so future blocks get valid numbers.
            // Without this, the free-list recycles too-high numbers and mining can never recover.
            block_allocator.reset_to(current_height + 1).await;
            // Return transactions to pool (ERR-005: wrap with timestamp)
            // QUA-004: Use write lock to return transactions
            if !block.transactions.is_empty() {
                let mut pool = tx_pool.write().await;
                for tx in &block.transactions {
                    pool.push(PooledTransaction::new(tx.clone()));
                    tx_pool_size.fetch_add(1, Ordering::Release);
                }
            }
            // SEC-015: Clear in-flight transactions on block number gap validation failure
            let mut in_flight = in_flight_txs.write().await;
            for tx in &block.transactions {
                in_flight.remove(&tx.hash);
            }
            // Backoff to prevent tight error loop
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            continue;
        }

        // MED-17: Calculate fees outside the lock to minimize lock hold time
        // EIP-1559: Enhanced fee distribution with base fee burn + priority fee split
        let base_fee = block.header.base_fee_per_gas;

        // Calculate fees using EIP-1559 model (outside lock)
        let mut total_base_fee: u128 = 0;
        let mut total_priority_fee: u128 = 0;

        for tx in &block.transactions {
            // Calculate base fee portion (100% burned)
            let tx_base_fee = base_fee * tx.gas_limit as u128;
            total_base_fee += tx_base_fee;

            // Calculate priority fee (tip) portion
            let effective_tip = calculate_effective_tip(tx, base_fee);
            let tx_priority_fee = effective_tip * tx.gas_limit as u128;
            total_priority_fee += tx_priority_fee;
        }

        // Priority fee split: 50% to miner, 50% burned (IronDAG policy)
        let miner_priority_share = total_priority_fee / 2;
        let burned_priority_share = total_priority_fee - miner_priority_share;

        // Total burned = base fee (100%) + 50% of priority fee
        let total_burned = total_base_fee + burned_priority_share;

        // Total miner reward = block reward + 50% of priority fee
        let total_miner_reward = actual_reward.saturating_add(miner_priority_share);

        // Log halving era if we're past era 0
        let era = block_number / HALVING_INTERVAL;
        if era > 0 {
            info!(
                "Block {} reward: {} IDAG (halving era {})",
                block_number,
                actual_reward as f64 / 1e18,
                era
            );
        }

        // Log fee distribution
        if total_burned > 0 || total_priority_fee > 0 {
            info!("EIP-1559 fees: base_fee={} Gwei, total_burned={} IDAG, miner_priority_share={} IDAG, txs={}",
                  base_fee / 1_000_000_000,
                  total_burned as f64 / 1e18,
                  miner_priority_share as f64 / 1e18,
                  block.transactions.len());
        }

        // Finding 5: Verify PoW hash BEFORE acquiring the blockchain write lock.
        // B3MemHash (~8.8ms) and Blake3 run in a blocking thread with no locks held,
        // so Stream A/sync can proceed concurrently. The write lock is then held only
        // for state-mutation (~1-2ms), not the full ~10ms it held before.
        let pow_hash_valid = if block.header.block_number > 0 {
            let verify_block = block.clone();
            tokio::task::spawn_blocking(move || {
                let tx_hashes: Vec<Hash> =
                    verify_block.transactions.iter().map(|tx| tx.hash).collect();
                let transactions_root = pow::calculate_transactions_root(&tx_hashes);
                let calculated_hash = match verify_block.header.stream_type {
                    StreamType::StreamA => pow::hash_blake3(&verify_block.header, &transactions_root),
                    StreamType::StreamB => pow::hash_b3memhash(&verify_block.header, &transactions_root),
                    _ => verify_block.calculate_hash(),
                };
                verify_block.hash == calculated_hash
            })
            .await
            .unwrap_or(false)
        } else {
            true // Genesis block — skip hash check
        };

        if !pow_hash_valid {
            error!(
                "MiningManager: Block #{} stream {:?} failed PoW hash verification — discarding",
                block_number, stream_type
            );
            block_allocator.release(block_number, stream_type).await;
            {
                let mut pool = tx_pool.write().await;
                for tx in &block.transactions {
                    pool.push(PooledTransaction::new(tx.clone()));
                    tx_pool_size.fetch_add(1, Ordering::Release);
                }
            }
            {
                let mut in_flight = in_flight_txs.write().await;
                for tx in &block.transactions {
                    in_flight.remove(&tx.hash);
                }
            }
            continue;
        }

        // Add block to blockchain - CRITICAL: minimize lock hold time
        let block_added = {
            let mut blockchain = blockchain.write().await;
            let add_result = blockchain.add_block_pre_verified(block.clone()).await;

            if add_result.is_ok() {
                // Track cumulative burned fees
                blockchain.add_burned_fees(total_burned).await;

                let current_balance = blockchain.get_balance(miner_address);
                if let Err(e) = blockchain.set_balance(
                    miner_address,
                    current_balance.saturating_add(total_miner_reward),
                ) {
                    warn!("Failed to persist reward: {}", e);
                }
                true
            } else {
                if let Err(e) = add_result {
                    error!(
                        "MiningManager: Block validation failed for Stream {:?} block #{}: {}",
                        stream_type, block_number, e
                    );
                    block_allocator.release(block_number, stream_type).await;
                    // Return transactions to pool (ERR-005: wrap with timestamp)
                    // QUA-004: Use write lock to return transactions
                    if !block.transactions.is_empty() {
                        let mut pool = tx_pool.write().await;
                        for tx in &block.transactions {
                            pool.push(PooledTransaction::new(tx.clone()));
                            tx_pool_size.fetch_add(1, Ordering::Release);
                        }
                    }
                    // SEC-015: Clear in-flight transactions on block validation failure
                    let mut in_flight = in_flight_txs.write().await;
                    for tx in &block.transactions {
                        in_flight.remove(&tx.hash);
                    }
                }
                false
            }
        };

        if !block_added {
            continue;
        }

        // SEC-015: Clear in-flight transactions after successful block commit
        if !block.transactions.is_empty() {
            let mut in_flight = in_flight_txs.write().await;
            for tx in &block.transactions {
                in_flight.remove(&tx.hash);
            }
        }

        // QUA-003: Clear pool membership set after successful block commit
        if !block.transactions.is_empty() {
            let mut pool_hashes = pool_tx_hashes.write().await;
            for tx in &block.transactions {
                pool_hashes.remove(&tx.hash);
            }
        }

        // QUA-009: Promote future-nonce transactions that are now executable
        // Collect unique senders from the block
        if !block.transactions.is_empty() {
            let senders: std::collections::HashSet<Address> =
                block.transactions.iter().map(|tx| tx.from).collect();

            for sender in senders {
                // Get the current nonce for this sender
                let current_nonce = {
                    let bc = blockchain.read().await;
                    bc.get_nonce(sender)
                };

                // Try to promote consecutive transactions from future queue
                let mut futures = future_txs.write().await;
                if let Some(sender_queue) = futures.get_mut(&sender) {
                    let mut next_nonce = current_nonce;
                    let mut promoted = 0;

                    while let Some(pooled_tx) = sender_queue.remove(&next_nonce) {
                        // Add to legacy pool for recovery (will be redistributed)
                        tx_pool.write().await.push(pooled_tx.clone());
                        tx_pool_size.fetch_add(1, Ordering::Release);

                        // Add back to pool_tx_hashes
                        pool_tx_hashes.write().await.insert(pooled_tx.tx.hash);

                        promoted += 1;
                        next_nonce += 1;
                    }

                    if promoted > 0 {
                        debug!(
                            "Promoted {} future-nonce tx(s) for sender {} after block commit",
                            promoted,
                            hex::encode(&sender.0[..8])
                        );
                    }

                    if sender_queue.is_empty() {
                        futures.remove(&sender);
                    }
                }
            }
        }

        if let Err(e) = block_allocator.confirm(block_number).await {
            warn!(
                "MiningManager: Failed to confirm block {}: {}",
                block_number, e
            );
        }

        // Phase 6: Notify shards of new block height (StateSync) for cross-shard consistency
        // Broadcast ALL shard block heights, not just shard 0
        if let Some(ref sm) = shard_manager {
            sm.broadcast_all_shard_block_heights().await;
        }

        // Cross-Shard Execution Bridge: Process cross-shard transactions in the mined block
        // For each transaction, check if it's cross-shard and process accordingly
        if let Some(ref sm) = shard_manager {
            for tx in &block.transactions {
                // Determine if this is a cross-shard transaction
                let from_shard = sm.get_shard_for_address(&tx.from);
                let to_shard = if !tx.to.is_zero() {
                    sm.get_shard_for_address(&tx.to)
                } else {
                    from_shard // Contract deployment stays on sender's shard
                };

                // Process cross-shard transaction
                if from_shard != to_shard {
                    let tx_hash = tx.hash;

                    // Ensure transaction is registered in cross_shard_txs
                    // (may already be registered via add_transaction, but register if not)
                    if !sm.has_cross_shard_transaction(tx_hash).await {
                        sm.register_cross_shard_transaction(tx.clone(), from_shard, to_shard)
                            .await;
                    }

                    // Process the cross-shard transaction (create receipt, send to target shard)
                    match sm.process_cross_shard_transaction(tx_hash).await {
                        Ok(()) => {
                            debug!(
                                "Cross-shard receipt created: shard {} -> shard {}",
                                from_shard, to_shard
                            );
                        }
                        Err(e) => {
                            // Log error but continue - don't halt mining
                            warn!(
                                "Failed to process cross-shard tx {}: {}",
                                hex::encode(tx_hash),
                                e
                            );
                        }
                    }
                }
            }
        }

        // CRITICAL: Broadcast block to network peers OUTSIDE the blockchain lock
        // This prevents network latency from blocking mining
        {
            let network_lock = network.read().await;
            if let Some(ref network_manager) = *network_lock {
                debug!("Broadcasting block #{} to network peers", block_number);
                if let Err(e) = network_manager.broadcast_block(&block, true).await {
                    warn!("Failed to broadcast block: {}", e);
                }
            }
        }

        // Record participation in node registry (CRITICAL for longevity tracking)
        if let (Some(ref registry), Some(ref identity)) = (&node_registry, &node_identity) {
            let participation = crate::governance::ParticipationType::BlockMined {
                stream: stream_type,
                block_hash: block.hash,
            };
            let mut registry = registry.write().await;
            registry.record_participation(identity, participation);
        }

        // Analyze fairness (outside blockchain lock)
        let _fairness_metrics = {
            let analyzer = fairness_analyzer.read().await;
            analyzer.analyze_block(&block)
        };

        // Record metrics
        if let Some(ref metrics) = metrics {
            let block_size = std::mem::size_of_val(&block)
                + block
                    .transactions
                    .iter()
                    .map(|tx| std::mem::size_of_val(tx))
                    .sum::<usize>();
            if let Ok(m) = (*metrics).lock() {
                let stream_name = match stream_type {
                    StreamType::StreamA => "A",
                    StreamType::StreamB => "B",
                    StreamType::StreamC => "C",
                };
                m.record_block_mined(stream_name, block_size, actual_reward);
            }
        }

        // Print success message
        let stream_name = match stream_type {
            StreamType::StreamA => "A",
            StreamType::StreamB => "B",
            StreamType::StreamC => "C",
        };
        let reward_str = if fees > 0 {
            format!("fees: {} IDAG", fees / 1_000_000_000_000_000_000)
        } else {
            format!("reward: {} IDAG", actual_reward / 1_000_000_000_000_000_000)
        };

        info!(
            "Stream {}: Mined block #{} ({} txs) - {}",
            stream_name,
            block_number,
            block.transactions.len(),
            reward_str
        );
    }

    // ERR-006: Log warning when process_blocks task exits due to channel closure
    warn!("process_blocks task exiting: channel closed");
}

#[cfg(test)]
mod halving_tests {
    use super::*;

    #[test]
    fn test_halving_era_0() {
        assert_eq!(get_block_reward(0, StreamType::StreamA), STREAM_A_REWARD);
        assert_eq!(get_block_reward(0, StreamType::StreamB), STREAM_B_REWARD);
        assert_eq!(get_block_reward(0, StreamType::StreamC), 0);
    }

    #[test]
    fn test_halving_era_1() {
        assert_eq!(
            get_block_reward(HALVING_INTERVAL, StreamType::StreamA),
            STREAM_A_REWARD / 2
        );
        assert_eq!(
            get_block_reward(HALVING_INTERVAL, StreamType::StreamB),
            STREAM_B_REWARD / 2
        );
    }

    #[test]
    fn test_halving_era_2() {
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 2, StreamType::StreamA),
            STREAM_A_REWARD / 4
        );
    }

    #[test]
    fn test_halving_era_3() {
        // Third halving: reward is 1/8 of original
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 3, StreamType::StreamA),
            STREAM_A_REWARD / 8
        );
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 3, StreamType::StreamB),
            STREAM_B_REWARD / 8
        );
    }

    #[test]
    fn test_halving_exhaustion() {
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 64, StreamType::StreamA),
            0
        );
    }

    #[test]
    fn test_halving_boundary() {
        // Last block before halving gets full reward
        assert_eq!(
            get_block_reward(HALVING_INTERVAL - 1, StreamType::StreamA),
            STREAM_A_REWARD
        );
        // First block after halving gets half
        assert_eq!(
            get_block_reward(HALVING_INTERVAL, StreamType::StreamA),
            STREAM_A_REWARD / 2
        );
    }

    // =========================================================================
    // Additional Halving Edge Case Tests (TEST-07)
    // =========================================================================

    #[test]
    fn test_halving_stream_b() {
        // Stream B has different base reward but same halving schedule
        assert_eq!(get_block_reward(0, StreamType::StreamB), STREAM_B_REWARD);
        assert_eq!(
            get_block_reward(HALVING_INTERVAL, StreamType::StreamB),
            STREAM_B_REWARD / 2
        );
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 2, StreamType::StreamB),
            STREAM_B_REWARD / 4
        );
    }

    #[test]
    fn test_halving_stream_c_always_zero() {
        // Stream C is fee-only, should always return 0
        assert_eq!(get_block_reward(0, StreamType::StreamC), 0);
        assert_eq!(get_block_reward(HALVING_INTERVAL, StreamType::StreamC), 0);
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 64, StreamType::StreamC),
            0
        );
        assert_eq!(get_block_reward(u64::MAX, StreamType::StreamC), 0);
    }

    #[test]
    fn test_halving_era_4() {
        // Fourth halving: reward is 1/16 of original
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 4, StreamType::StreamA),
            STREAM_A_REWARD / 16
        );
    }

    #[test]
    fn test_halving_era_10() {
        // 10th halving: reward is 1/1024 of original
        let expected = STREAM_A_REWARD >> 10;
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 10, StreamType::StreamA),
            expected
        );
    }

    #[test]
    fn test_halving_just_before_exhaustion() {
        // Block just before 64th halving should have tiny reward
        let reward = get_block_reward(HALVING_INTERVAL * 63, StreamType::StreamA);
        assert!(reward > 0, "Should still have some reward at era 63");
        assert_eq!(reward, STREAM_A_REWARD >> 63);
    }

    #[test]
    fn test_halving_at_exhaustion() {
        // At 64th halving, reward should be 0
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 64, StreamType::StreamA),
            0
        );
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 64, StreamType::StreamB),
            0
        );
    }

    #[test]
    fn test_halving_past_exhaustion() {
        // Past 64th halving, reward should remain 0
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 65, StreamType::StreamA),
            0
        );
        assert_eq!(
            get_block_reward(HALVING_INTERVAL * 100, StreamType::StreamA),
            0
        );
        assert_eq!(get_block_reward(u64::MAX, StreamType::StreamA), 0);
    }

    #[test]
    fn test_halving_all_boundary_blocks() {
        // Test all halving boundary transitions
        for era in 1..=5 {
            let before = get_block_reward(HALVING_INTERVAL * era - 1, StreamType::StreamA);
            let after = get_block_reward(HALVING_INTERVAL * era, StreamType::StreamA);

            // Before should be double the after (for first few eras)
            if era < 64 {
                assert_eq!(
                    before,
                    STREAM_A_REWARD >> (era - 1),
                    "Era {} before boundary",
                    era
                );
                assert_eq!(after, STREAM_A_REWARD >> era, "Era {} after boundary", era);
            }
        }
    }

    #[test]
    fn test_halving_reward_never_increases() {
        // Verify reward never increases across halvings
        let mut prev_reward = STREAM_A_REWARD;
        for era in 1..=70 {
            let height = HALVING_INTERVAL * era;
            let reward = get_block_reward(height, StreamType::StreamA);
            assert!(
                reward <= prev_reward,
                "Reward should never increase: era {} reward {} > previous {}",
                era,
                reward,
                prev_reward
            );
            prev_reward = reward;
        }
    }

    #[test]
    fn test_halving_within_era() {
        // Blocks within the same era should have the same reward
        let era = 2;
        let start_height = HALVING_INTERVAL * era;
        let end_height = HALVING_INTERVAL * (era + 1) - 1;
        let expected_reward = STREAM_A_REWARD >> era;

        // Check first, middle, and last block of era
        assert_eq!(
            get_block_reward(start_height, StreamType::StreamA),
            expected_reward
        );
        assert_eq!(
            get_block_reward(start_height + HALVING_INTERVAL / 2, StreamType::StreamA),
            expected_reward
        );
        assert_eq!(
            get_block_reward(end_height, StreamType::StreamA),
            expected_reward
        );
    }
}
