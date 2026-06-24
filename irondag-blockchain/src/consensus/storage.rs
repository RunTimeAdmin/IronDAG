//! GhostDAG storage integration
//!
//! Implements hybrid storage: hot DAG in RAM, finalized blocks on disk.
//! This solves the "in-memory consensus will crash at scale" problem.

use crate::blockchain::Block;
use crate::storage::Database;
use crate::types::Hash;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{debug, info};

// ============================================================================
// KEY PREFIX CONSTANTS for consensus state persistence
// ============================================================================

/// Prefix for blue score entries: [PREFIX_BLUE_SCORE][32-byte hash] -> u64 LE
const PREFIX_BLUE_SCORE: u8 = 0x10;
/// Prefix for blue set membership: [PREFIX_BLUE_SET][32-byte hash] -> empty
const PREFIX_BLUE_SET: u8 = 0x11;
/// Prefix for ordering: [PREFIX_ORDERING][8-byte index BE] -> 32-byte hash
const PREFIX_ORDERING: u8 = 0x12;
/// Key for checkpoint metadata (genesis hash + block count)
const KEY_CHECKPOINT_META: &[u8] = b"checkpoint_meta";

/// Configuration for DAG storage
pub struct DagStorageConfig {
    /// Number of recent blocks to keep in RAM (hot cache)
    pub hot_cache_size: usize,
    /// Blocks older than this are considered "finalized" and can be flushed to disk
    pub finalized_depth: usize,
    /// Blocks with at least this many confirmations are checkpointed (pruned from hot state). 0 = disable checkpoint pruning.
    pub confirmations_for_checkpoint: usize,
}

/// Statistics for DAG pruning operations
#[derive(Debug, Clone, Default)]
pub struct PruningStats {
    pub hot_block_count: usize,
    pub finalized_count: usize,
    pub red_blocks_pruned: usize,
    pub last_prune_height: u64,
}

impl Default for DagStorageConfig {
    fn default() -> Self {
        Self {
            hot_cache_size: 1000,
            finalized_depth: 500,
            confirmations_for_checkpoint: 100, // Prune blocks with 100+ confirmations to bound hot DAG memory
        }
    }
}

/// Hybrid storage for GhostDAG
///
/// Architecture:
/// - Hot DAG (recent blocks) in RAM for fast access
/// - Finalized blocks on disk (sled database)
/// - LRU-style eviction: insertion order (oldest inserted evicted first)
pub struct HybridDagStorage {
    /// Database for persistent storage
    database: Option<Arc<Database>>,

    /// Hot cache: recent blocks in RAM
    hot_blocks: HashMap<Hash, Block>,

    /// Eviction order: hashes in insertion order (front = oldest). Kept in sync with hot_blocks.
    hot_blocks_order: VecDeque<Hash>,

    /// Hot cache: parent-child relationships (recent blocks)
    hot_children: HashMap<Hash, Vec<Hash>>,

    /// Hot cache: blue set (recent blocks)
    hot_blue_set: HashSet<Hash>,

    /// Hot cache: blue scores (recent blocks)
    hot_blue_scores: HashMap<Hash, u64>,

    /// Configuration
    config: DagStorageConfig,

    /// Track which blocks are finalized (on disk)
    finalized_blocks: HashSet<Hash>,

    /// Blocks added since last checkpoint (for periodic checkpointing)
    blocks_since_checkpoint: usize,
}

impl HybridDagStorage {
    /// Create new hybrid storage
    pub fn new(config: DagStorageConfig) -> Self {
        Self {
            database: None,
            hot_blocks: HashMap::new(),
            hot_blocks_order: VecDeque::new(),
            hot_children: HashMap::new(),
            hot_blue_set: HashSet::new(),
            hot_blue_scores: HashMap::new(),
            config,
            finalized_blocks: HashSet::new(),
            blocks_since_checkpoint: 0,
        }
    }

    /// Create with database
    pub fn with_database(database: Arc<Database>, config: DagStorageConfig) -> Self {
        Self {
            database: Some(database),
            hot_blocks: HashMap::new(),
            hot_blocks_order: VecDeque::new(),
            hot_children: HashMap::new(),
            hot_blue_set: HashSet::new(),
            hot_blue_scores: HashMap::new(),
            config,
            finalized_blocks: HashSet::new(),
            blocks_since_checkpoint: 0,
        }
    }

    /// Get block (checks hot cache first, then disk)
    pub fn get_block(&self, hash: &Hash) -> crate::error::BlockchainResult<Option<Block>> {
        // Check hot cache first (fast)
        if let Some(block) = self.hot_blocks.get(hash) {
            return Ok(Some(block.clone()));
        }

        // Check disk if database available
        if let Some(ref db) = self.database {
            use crate::storage::BlockStore;
            let block_store = BlockStore::new(db);
            return block_store.get(hash);
        }

        Ok(None)
    }

    /// Add block to storage
    ///
    /// PERF: Takes &Block reference to avoid expensive clone of transaction data.
    /// The block is cloned once when inserting into hot_blocks HashMap.
    pub fn add_block(&mut self, block: &Block) -> crate::error::BlockchainResult<()> {
        let hash = block.hash;

        // Add to hot cache; record insertion order only for new blocks (eviction = oldest first)
        if !self.hot_blocks.contains_key(&hash) {
            self.hot_blocks_order.push_back(hash);
        }
        self.hot_blocks.insert(hash, block.clone());

        // PERF (PER-002 TODO): Persist to disk synchronously inside the block processing lock.
        // Future optimization: Use a background task/channel to defer disk writes outside the lock.
        // Current tradeoff: Simplicity vs. throughput. Sled's async writes help mitigate blocking.
        // For high-throughput scenarios, consider:
        //   1. tokio::task::spawn_blocking for disk writes
        //   2. A persistent queue + background writer thread
        //   3. Batch writes with fsync coalescing
        if let Some(ref db) = self.database {
            use crate::storage::{BlockStore, ParentChildStore};
            let block_store = BlockStore::new(db);
            block_store.put(&block)?;

            // CRITICAL: Store parent-child relationships for efficient DAG traversal
            let parent_child_store = ParentChildStore::new(db);

            // Store parents of this block (reverse index)
            parent_child_store.put_parents(&hash, &block.header.parent_hashes)?;

            // Update children for each parent
            for parent_hash in &block.header.parent_hashes {
                // Get existing children (from cache or disk)
                let mut children = self.get_children(parent_hash)?;

                // Add this block as a child if not already present
                if !children.contains(&hash) {
                    children.push(hash);
                    parent_child_store.put_children(parent_hash, &children)?;

                    // Also update hot cache if parent is in cache
                    if self.hot_children.contains_key(parent_hash) {
                        self.hot_children.insert(*parent_hash, children);
                    }
                }
            }
        }

        // Prune hot cache if it exceeds size
        if self.hot_blocks.len() > self.config.hot_cache_size {
            self.prune_hot_cache();
        }

        Ok(())
    }

    /// Prune hot cache by evicting oldest-inserted blocks (LRU-style ordered eviction).
    /// Evicts from the front of the insertion order until at or below target size.
    fn prune_hot_cache(&mut self) {
        let target_size = self.config.hot_cache_size;
        let mut remove_count = self.hot_blocks.len().saturating_sub(target_size);
        if remove_count == 0 {
            return;
        }
        while remove_count > 0 {
            let hash = match self.hot_blocks_order.pop_front() {
                Some(h) => h,
                None => break,
            };
            if self.hot_blocks.remove(&hash).is_some() {
                self.hot_children.remove(&hash);
                self.hot_blue_set.remove(&hash);
                self.hot_blue_scores.remove(&hash);
                self.finalized_blocks.insert(hash);
                remove_count -= 1;
            }
        }
    }

    /// Get children of a block (checks hot cache and disk)
    pub fn get_children(&self, parent_hash: &Hash) -> crate::error::BlockchainResult<Vec<Hash>> {
        // Check hot cache first (fast path)
        if let Some(children) = self.hot_children.get(parent_hash) {
            return Ok(children.clone());
        }

        // Check disk for finalized blocks (slower but necessary for traversal)
        if let Some(ref db) = self.database {
            use crate::storage::ParentChildStore;
            let parent_child_store = ParentChildStore::new(db);
            if let Some(children) = parent_child_store.get_children(parent_hash)? {
                return Ok(children);
            }
        }

        // Not found in cache or disk
        Ok(Vec::new())
    }

    /// Set children for a block
    pub fn set_children(&mut self, parent_hash: Hash, children: Vec<Hash>) {
        self.hot_children.insert(parent_hash, children);
    }

    /// Check if block is in blue set
    pub fn is_blue(&self, hash: &Hash) -> bool {
        self.hot_blue_set.contains(hash)
    }

    /// Add to blue set
    pub fn add_to_blue_set(&mut self, hash: Hash) {
        self.hot_blue_set.insert(hash);
    }

    /// Get blue score
    pub fn get_blue_score(&self, hash: &Hash) -> Option<u64> {
        self.hot_blue_scores.get(hash).copied()
    }

    /// Set blue score
    pub fn set_blue_score(&mut self, hash: Hash, score: u64) {
        self.hot_blue_scores.insert(hash, score);
    }

    /// Get all hot blocks (for fast iteration)
    pub fn get_hot_blocks(&self) -> &HashMap<Hash, Block> {
        &self.hot_blocks
    }

    /// Get all blocks from storage (hot cache + disk)
    /// Used for recovery when checkpoint ordering is too short for finalization.
    pub fn get_all_blocks(&self) -> crate::error::BlockchainResult<Vec<Block>> {
        // If no database, just return hot blocks
        let Some(ref db) = self.database else {
            return Ok(self.hot_blocks.values().cloned().collect());
        };

        // Use BlockStore to get all blocks from disk
        use crate::storage::BlockStore;
        let block_store = BlockStore::new(db);
        block_store.get_all_blocks()
    }

    /// Check if block is finalized (on disk only)
    pub fn is_finalized(&self, hash: &Hash) -> bool {
        self.finalized_blocks.contains(hash)
    }

    /// Confirmations required before a block is checkpointed (0 = disabled).
    pub fn confirmations_for_checkpoint(&self) -> usize {
        self.config.confirmations_for_checkpoint
    }

    /// Prune specific blocks (checkpoint): remove from hot cache and mark finalized.
    /// Used when consensus marks blocks as finalized after N confirmations.
    pub fn prune_blocks_by_hash(&mut self, hashes: &[Hash]) {
        let set: HashSet<Hash> = hashes.iter().copied().collect();
        for h in hashes {
            self.hot_blocks.remove(h);
            self.hot_children.remove(h);
            self.hot_blue_set.remove(h);
            self.hot_blue_scores.remove(h);
            self.finalized_blocks.insert(*h);
        }
        self.hot_blocks_order.retain(|h| !set.contains(h));
    }

    /// Prune red blocks (blocks not in blue set) that are older than the finality threshold.
    /// Returns the count of pruned blocks.
    pub fn prune_red_blocks(&mut self, current_height: u64, finality_depth: u64) -> usize {
        if current_height <= finality_depth {
            return 0;
        }
        let threshold = current_height - finality_depth;

        // Find blocks NOT in hot_blue_set that are older than threshold
        let to_prune: Vec<Hash> = self
            .hot_blocks
            .iter()
            .filter(|(hash, block)| {
                !self.hot_blue_set.contains(*hash) && block.header.block_number < threshold
            })
            .map(|(hash, _)| *hash)
            .collect();

        let pruned_count = to_prune.len();

        // Remove pruned blocks from all hot caches
        for hash in &to_prune {
            self.hot_blocks.remove(hash);
            self.hot_children.remove(hash);
            self.hot_blue_scores.remove(hash);
            self.finalized_blocks.insert(*hash);
        }

        // Update the order queue
        let prune_set: HashSet<Hash> = to_prune.into_iter().collect();
        self.hot_blocks_order.retain(|h| !prune_set.contains(h));

        if pruned_count > 0 {
            debug!(
                "Pruned {} red blocks older than height {}",
                pruned_count, threshold
            );
        }

        pruned_count
    }

    /// Get pruning statistics for monitoring
    pub fn get_pruning_stats(&self) -> PruningStats {
        PruningStats {
            hot_block_count: self.hot_blocks.len(),
            finalized_count: self.finalized_blocks.len(),
            red_blocks_pruned: 0, // caller tracks cumulative
            last_prune_height: 0, // caller tracks
        }
    }

    /// Clear all storage (used during full resync)
    pub fn clear(&mut self) {
        self.hot_blocks.clear();
        self.hot_blocks_order.clear();
        self.hot_children.clear();
        self.hot_blue_set.clear();
        self.hot_blue_scores.clear();
        self.finalized_blocks.clear();
        self.blocks_since_checkpoint = 0;
    }

    // ========================================================================
    // CHECKPOINT PERSISTENCE METHODS
    // ========================================================================

    /// Increment the checkpoint counter and return the new value
    pub fn increment_checkpoint_counter(&mut self) -> usize {
        self.blocks_since_checkpoint += 1;
        self.blocks_since_checkpoint
    }

    /// Reset the checkpoint counter (after successful checkpoint)
    pub fn reset_checkpoint_counter(&mut self) {
        self.blocks_since_checkpoint = 0;
    }

    /// Get the current checkpoint counter
    pub fn blocks_since_checkpoint(&self) -> usize {
        self.blocks_since_checkpoint
    }

    /// Save a single blue score to sled
    pub fn save_blue_score(&self, hash: &Hash, score: u64) -> crate::error::BlockchainResult<()> {
        if let Some(ref db) = self.database {
            let mut key = vec![PREFIX_BLUE_SCORE];
            key.extend_from_slice(hash.as_ref());
            let value = score.to_le_bytes().to_vec();
            db.insert_raw(key, value)?;
        }
        Ok(())
    }

    /// Load all blue scores from sled
    pub fn load_blue_scores(&self) -> crate::error::BlockchainResult<HashMap<Hash, u64>> {
        let mut scores = HashMap::new();
        if let Some(ref db) = self.database {
            for (key, value) in db.scan_prefix_collect(&[PREFIX_BLUE_SCORE]) {
                if key.len() == 33 && value.len() == 8 {
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&key[1..]);
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&value);
                    scores.insert(Hash(hash), u64::from_le_bytes(bytes));
                }
            }
        }
        Ok(scores)
    }

    /// Save the blue set to sled
    pub fn save_blue_set(&self, blue_set: &HashSet<Hash>) -> crate::error::BlockchainResult<()> {
        if let Some(ref db) = self.database {
            // Clear old blue set entries first
            let keys_to_remove: Vec<Vec<u8>> = db
                .scan_prefix_collect(&[PREFIX_BLUE_SET])
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            for key in keys_to_remove {
                db.remove_raw(&key)?;
            }

            // Insert new blue set entries
            for hash in blue_set {
                let mut key = vec![PREFIX_BLUE_SET];
                key.extend_from_slice(hash.as_ref());
                db.insert_raw(key, vec![])?;
            }
        }
        Ok(())
    }

    /// Load the blue set from sled
    pub fn load_blue_set(&self) -> crate::error::BlockchainResult<HashSet<Hash>> {
        let mut blue_set = HashSet::new();
        if let Some(ref db) = self.database {
            for (key, _) in db.scan_prefix_collect(&[PREFIX_BLUE_SET]) {
                if key.len() == 33 {
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&key[1..]);
                    blue_set.insert(Hash(hash));
                }
            }
        }
        Ok(blue_set)
    }

    /// Save the ordering to sled
    pub fn save_ordering(&self, ordering: &[Hash]) -> crate::error::BlockchainResult<()> {
        if let Some(ref db) = self.database {
            // Clear old ordering entries first
            let keys_to_remove: Vec<Vec<u8>> = db
                .scan_prefix_collect(&[PREFIX_ORDERING])
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            for key in keys_to_remove {
                db.remove_raw(&key)?;
            }

            // Insert new ordering entries (index -> hash)
            for (idx, hash) in ordering.iter().enumerate() {
                let mut key = vec![PREFIX_ORDERING];
                key.extend_from_slice(&(idx as u64).to_be_bytes());
                db.insert_raw(key, hash.as_ref().to_vec())?;
            }
        }
        Ok(())
    }

    /// Load the ordering from sled
    pub fn load_ordering(&self) -> crate::error::BlockchainResult<Vec<Hash>> {
        let mut ordering_map: HashMap<u64, Hash> = HashMap::new();
        if let Some(ref db) = self.database {
            for (key, value) in db.scan_prefix_collect(&[PREFIX_ORDERING]) {
                if key.len() == 9 && value.len() == 32 {
                    let mut idx_bytes = [0u8; 8];
                    idx_bytes.copy_from_slice(&key[1..]);
                    let idx = u64::from_be_bytes(idx_bytes);
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&value);
                    ordering_map.insert(idx, Hash(hash));
                }
            }
        }

        // Sort by index and return as Vec
        let max_idx = ordering_map.keys().max().copied().unwrap_or(0);
        let mut ordering = Vec::with_capacity(ordering_map.len());
        for i in 0..=max_idx {
            if let Some(hash) = ordering_map.get(&i) {
                ordering.push(*hash);
            }
        }
        Ok(ordering)
    }

    /// Atomically save a checkpoint of all consensus state
    pub fn save_checkpoint(
        &self,
        genesis_hash: Option<Hash>,
        blue_set: &HashSet<Hash>,
        blue_scores: &HashMap<Hash, u64>,
        ordering: &[Hash],
    ) -> crate::error::BlockchainResult<()> {
        if let Some(ref db) = self.database {
            info!(
                "[Checkpoint] Saving consensus state: {} blue blocks, {} scores, {} ordering",
                blue_set.len(),
                blue_scores.len(),
                ordering.len()
            );

            // Save genesis hash and block count as metadata
            let mut meta_value = Vec::with_capacity(40);
            if let Some(genesis) = genesis_hash {
                meta_value.extend_from_slice(genesis.as_ref());
            } else {
                meta_value.extend_from_slice(&[0u8; 32]);
            }
            meta_value.extend_from_slice(&(ordering.len() as u64).to_be_bytes());
            db.insert_raw(KEY_CHECKPOINT_META.to_vec(), meta_value)?;

            // Save blue set
            self.save_blue_set(blue_set)?;

            // Save blue scores
            for (hash, score) in blue_scores {
                self.save_blue_score(hash, *score)?;
            }

            // Save ordering
            self.save_ordering(ordering)?;

            // Flush to ensure durability
            let _ = db.flush();

            info!("[Checkpoint] Consensus state saved successfully");
        }
        Ok(())
    }

    /// Load the last checkpoint from sled
    /// Returns (genesis_hash, blue_set, blue_scores, ordering) if checkpoint exists
    pub fn load_checkpoint(
        &self,
    ) -> crate::error::BlockchainResult<
        Option<(Option<Hash>, HashSet<Hash>, HashMap<Hash, u64>, Vec<Hash>)>,
    > {
        if let Some(ref db) = self.database {
            // Check if checkpoint metadata exists
            let meta = db.get_raw(KEY_CHECKPOINT_META)?;

            let meta = match meta {
                Some(m) => m,
                None => return Ok(None), // No checkpoint exists
            };

            if meta.len() < 40 {
                return Ok(None); // Invalid metadata
            }

            // Parse genesis hash
            let mut genesis_hash_bytes = [0u8; 32];
            genesis_hash_bytes.copy_from_slice(&meta[0..32]);
            let genesis_hash = if genesis_hash_bytes == [0u8; 32] {
                None
            } else {
                Some(Hash(genesis_hash_bytes))
            };

            // Load blue set
            let blue_set = self.load_blue_set()?;

            // Load blue scores
            let blue_scores = self.load_blue_scores()?;

            // Load ordering
            let ordering = self.load_ordering()?;

            if blue_set.is_empty() && ordering.is_empty() {
                info!("[Checkpoint] Found checkpoint metadata but no data, ignoring");
                return Ok(None);
            }

            info!(
                "[Checkpoint] Loaded consensus state: {} blue blocks, {} scores, {} ordering",
                blue_set.len(),
                blue_scores.len(),
                ordering.len()
            );

            return Ok(Some((genesis_hash, blue_set, blue_scores, ordering)));
        }
        Ok(None)
    }
}
