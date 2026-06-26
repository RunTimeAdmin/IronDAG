//! GhostDAG Consensus Implementation
//!
//! Full GhostDAG (BlockDAG) consensus algorithm based on Kaspa's protocol.
//! Orders blocks in a DAG structure using blue score calculation.

pub mod storage;
// #[cfg(test)]
// mod tests_storage; // TODO: Fix imports

use crate::blockchain::Block;
use crate::storage::Database;
use crate::types::Hash;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// GhostDAG consensus engine
///
/// Now uses hybrid storage:
/// - Hot DAG (recent blocks) in RAM for fast access
/// - Finalized blocks on disk (sled database)
/// - Prevents memory exhaustion at scale
///
/// CRITICAL: This struct is stateless regarding block data.
/// All block data comes from `storage` to prevent double-booking memory.
pub struct GhostDAG {
    // DAG security parameter K (max parents per block, tips selection)
    // Controls tradeoff between parallelism and convergence speed
    // Higher K = more parallelism but slower block ordering finality
    k: usize,

    // Hybrid storage (hot cache + disk) - ONLY source of block data
    storage: storage::HybridDagStorage,

    // Consensus state (lightweight, doesn't store block data)
    genesis_hash: Option<Hash>,           // Cached genesis block hash
    blue_set: HashSet<Hash>,              // Blue blocks (selected for consensus)
    red_set: HashSet<Hash>,               // Red blocks (not selected)
    blue_score: HashMap<Hash, u64>,       // Blue score for each block
    block_timestamps: HashMap<Hash, u64>, // Timestamp cache for sort comparator (avoids disk reads)
    ordering: Vec<Hash>,                  // Final block ordering
    checkpoint_stale_rebuilt: bool,       // Guard: stale-checkpoint BFS rebuild fires at most once per session
}

impl GhostDAG {
    /// Create new GhostDAG (in-memory only, for backward compatibility)
    /// Uses default K=4
    pub fn new() -> Self {
        Self::with_k(4)
    }

    /// Create GhostDAG with specific K parameter
    ///
    /// # Arguments
    /// * `k` - DAG security parameter (1-64). Controls max parents and tips selection.
    ///   - K=4 is standard (Kaspa default)
    ///   - Higher K = more parallelism, slower convergence
    ///   - Lower K = faster convergence, less parallelism
    ///
    /// # Panics
    /// Panics if K is not in range [1, 64]
    pub fn with_k(k: usize) -> Self {
        assert!(
            k >= 1 && k <= 64,
            "GhostDAG K must be between 1 and 64, got {}",
            k
        );
        Self {
            k,
            storage: storage::HybridDagStorage::new(Default::default()),
            genesis_hash: None,
            blue_set: HashSet::new(),
            red_set: HashSet::new(),
            blue_score: HashMap::new(),
            block_timestamps: HashMap::new(),
            checkpoint_stale_rebuilt: false,
            ordering: Vec::new(),
        }
    }

    /// Create GhostDAG with database (hybrid storage)
    /// Uses default K=4
    /// Attempts to load consensus state from checkpoint; falls back to empty state
    pub fn with_database(database: Arc<Database>) -> Self {
        Self::with_database_and_k(database, 4)
    }

    /// Create GhostDAG with database and specific K parameter
    ///
    /// # Arguments
    /// * `database` - Database for persistent storage
    /// * `k` - DAG security parameter (1-64)
    ///
    /// # Panics
    /// Panics if K is not in range [1, 64]
    pub fn with_database_and_k(database: Arc<Database>, k: usize) -> Self {
        assert!(
            k >= 1 && k <= 64,
            "GhostDAG K must be between 1 and 64, got {}",
            k
        );
        let config = storage::DagStorageConfig {
            hot_cache_size: 1000,
            finalized_depth: 500,
            confirmations_for_checkpoint: 100, // Prune blocks with 100+ confirmations to bound hot DAG memory
        };

        // Extract finality_depth before config is moved
        let finality_depth = config.confirmations_for_checkpoint;

        let storage = storage::HybridDagStorage::with_database(database, config);

        // Try to load from checkpoint
        let (genesis_hash, blue_set, blue_score, ordering, _loaded_from_checkpoint) = match storage
            .load_checkpoint()
        {
            Ok(Some((genesis, blue_set, blue_scores, ordering))) => {
                info!("[GhostDAG] Loaded consensus state from checkpoint: {} blue blocks, {} scores, {} ordering",
                             blue_set.len(), blue_scores.len(), ordering.len());
                (genesis, blue_set, blue_scores, ordering, true)
            }
            Ok(None) => {
                info!("[GhostDAG] No checkpoint found, starting with empty state");
                (None, HashSet::new(), HashMap::new(), Vec::new(), false)
            }
            Err(e) => {
                warn!(
                    "[GhostDAG] Failed to load checkpoint: {}, starting fresh",
                    e
                );
                (None, HashSet::new(), HashMap::new(), Vec::new(), false)
            }
        };

        let mut ghostdag = Self {
            k,
            storage,
            genesis_hash,
            blue_set,
            red_set: HashSet::new(), // Red set is recomputed on demand
            blue_score,
            block_timestamps: HashMap::new(), // Timestamps will be populated as blocks are added
            ordering,
            checkpoint_stale_rebuilt: false,
        };

        // RECOVERY: If ordering is too short for finalization, rebuild from stored blocks.
        // This handles cases where checkpoint loading fails or returns empty/short ordering.
        if ghostdag.ordering.len() <= finality_depth {
            match ghostdag.rebuild_ordering_from_storage() {
                Ok(count) if count > 0 => {
                    info!("[GhostDAG] Rebuilt ordering from {} stored blocks", count);
                }
                Ok(_) => {
                    // No blocks in storage, nothing to rebuild
                }
                Err(e) => {
                    warn!("[GhostDAG] Failed to rebuild ordering from storage: {}", e);
                }
            }
        }

        ghostdag
    }

    /// Check if consensus state was loaded from a checkpoint (useful for diagnostics)
    pub fn loaded_from_checkpoint(&self) -> bool {
        !self.blue_set.is_empty() || !self.ordering.is_empty()
    }

    /// Get the GhostDAG K parameter
    /// Returns the maximum number of tips/parents for block selection
    pub fn get_k(&self) -> usize {
        self.k
    }

    /// Add a block to the DAG and recalculate consensus
    ///
    /// CRITICAL: Block data is ONLY stored in `storage`, not in this struct.
    /// This prevents double-booking memory.
    ///
    /// PERF: Takes &Block reference to avoid expensive clone of transaction data.
    pub fn add_block(&mut self, block: &Block) -> crate::error::BlockchainResult<()> {
        let hash = block.hash;

        // Cache genesis hash if this is the first block
        if self.genesis_hash.is_none() && block.header.parent_hashes.is_empty() {
            self.genesis_hash = Some(hash);
        }

        // Add to hybrid storage (ONLY place block data is stored)
        // PERF: Storage receives &Block to avoid redundant clone
        self.storage.add_block(block)?;

        // Cache block timestamp for sort comparator (avoids disk reads in hot path)
        self.block_timestamps.insert(hash, block.header.timestamp);

        // Build parent-child relationships (stored in storage, not here)
        for parent_hash in &block.header.parent_hashes {
            let mut existing_children = self.storage.get_children(parent_hash)?;
            if !existing_children.contains(&hash) {
                existing_children.push(hash);
                self.storage.set_children(*parent_hash, existing_children);
            }
        }
        // New block has no children yet; ensure get_children(hash) returns [] until someone uses us as parent
        self.storage.set_children(hash, vec![]);

        // Incremental blue set update (only affected subtree); full recalc when empty (e.g. genesis)
        if self.blue_set.is_empty() {
            self.update_blue_set()?;
        } else {
            self.update_blue_set_incremental(hash)?;
        }

        // Optional checkpoint pruning: drop blocks with enough confirmations to bound memory
        let n = self.storage.confirmations_for_checkpoint();
        if n > 0 {
            self.prune_below_checkpoint(n)?;
        }

        // Periodic checkpoint: save consensus state every N blocks
        let checkpoint_interval = self.storage.confirmations_for_checkpoint();
        if checkpoint_interval > 0 {
            let blocks_since = self.storage.increment_checkpoint_counter();
            if blocks_since >= checkpoint_interval {
                // Save checkpoint (non-blocking: log warning on failure but continue)
                if let Err(e) = self.storage.save_checkpoint(
                    self.genesis_hash,
                    &self.blue_set,
                    &self.blue_score,
                    &self.ordering,
                ) {
                    warn!("[GhostDAG] Failed to save checkpoint: {}", e);
                } else {
                    self.storage.reset_checkpoint_counter();
                }
            }
        }

        Ok(())
    }

    /// Prune blocks that have at least `confirmations` (blocks before them in ordering).
    /// Ordering is blue-score desc so index 0 = tip; block at index i has i confirmations.
    fn prune_below_checkpoint(
        &mut self,
        confirmations: usize,
    ) -> crate::error::BlockchainResult<()> {
        // Keep a buffer above the finality depth so get_finalized_block_hash() can return a value.
        // Prune only when ordering exceeds the high water mark to avoid truncating on every block.
        let buffer = confirmations; // Keep 2x confirmations (200 entries) before pruning
        let high_water_mark = confirmations + buffer;

        if self.ordering.len() <= high_water_mark {
            return Ok(()); // Don't prune until we have enough buffer
        }

        // Keep the most recent `high_water_mark` entries, prune the rest
        let to_prune: Vec<Hash> = self.ordering[high_water_mark..].to_vec();
        for h in &to_prune {
            self.blue_set.remove(h);
            self.blue_score.remove(h);
            self.red_set.remove(h);
            self.block_timestamps.remove(h);
        }
        self.ordering.truncate(high_water_mark);
        self.storage.prune_blocks_by_hash(&to_prune);
        Ok(())
    }

    /// Update blue set using GhostDAG algorithm
    fn update_blue_set(&mut self) -> crate::error::BlockchainResult<()> {
        // Get genesis hash (cached or from storage)
        let genesis_hash = if let Some(genesis) = self.genesis_hash {
            genesis
        } else {
            // No genesis cached yet - this shouldn't happen, but handle gracefully
            return Ok(());
        };

        // Verify genesis block exists in storage
        let genesis_block = match self.storage.get_block(&genesis_hash)? {
            Some(block) => block,
            None => return Ok(()), // Genesis not found, skip
        };

        // Verify it's actually a genesis block
        if !genesis_block.header.parent_hashes.is_empty() {
            return Ok(()); // Not a genesis block
        }

        let genesis_blocks = vec![genesis_hash];

        // Reset blue set and scores
        self.blue_score.clear();
        self.blue_set.clear();
        self.red_set.clear();

        // Initialize genesis blocks
        for genesis_hash in &genesis_blocks {
            self.blue_score.insert(*genesis_hash, 1);
            self.blue_set.insert(*genesis_hash);
        }

        // BFS traversal to calculate blue scores
        let mut queue = VecDeque::from(genesis_blocks);
        let mut visited = HashSet::new();

        for genesis in &queue {
            visited.insert(*genesis);
        }

        while let Some(current) = queue.pop_front() {
            // Process children of current block
            // Get children from storage (hot cache or disk)
            let children = self.storage.get_children(&current)?;

            for child_hash in children {
                if visited.contains(&child_hash) {
                    continue;
                }
                visited.insert(child_hash);
                queue.push_back(child_hash);

                // Calculate blue score for child
                // Blue score = max(blue scores of blue parents) + 1
                // Get block from storage (hot cache or disk)
                let block = match self.storage.get_block(&child_hash)? {
                    Some(block) => block,
                    None => continue, // Block not found, skip
                };

                let parent_scores: Vec<u64> = block
                    .header
                    .parent_hashes
                    .iter()
                    .filter(|parent_hash| {
                        // Check if parent is in blue set
                        self.blue_set.contains(*parent_hash)
                    })
                    .filter_map(|parent_hash| self.blue_score.get(parent_hash).copied())
                    .collect();

                if !parent_scores.is_empty() {
                    let max_parent_score = parent_scores.iter().max().copied().unwrap_or(0);
                    let child_blue_score = max_parent_score + 1;
                    self.blue_score.insert(child_hash, child_blue_score);
                    self.blue_set.insert(child_hash);
                } else {
                    // No blue parents - mark as red
                    self.red_set.insert(child_hash);
                }
            }
        }

        // Order blocks by blue score (descending) and timestamp (ascending)
        // Get blocks from storage (hot cache or disk)
        let mut ordered: Vec<(Hash, u64, u64)> = Vec::new();
        for hash in &self.blue_set {
            if let Ok(Some(block)) = self.storage.get_block(hash) {
                let score = self.blue_score.get(hash).copied().unwrap_or(0);
                ordered.push((*hash, score, block.header.timestamp));
            }
        }

        // Sort: first by blue score (descending), then by timestamp (ascending)
        ordered.sort_by(|a, b| match b.1.cmp(&a.1) {
            std::cmp::Ordering::Equal => a.2.cmp(&b.2),
            other => other,
        });

        self.ordering = ordered.into_iter().map(|(hash, _, _)| hash).collect();

        Ok(())
    }

    /// Rebuild the entire blue set from scratch.
    ///
    /// This is a public method for startup/recovery scenarios when the DAG state
    /// needs to be fully recalculated. This performs a full BFS traversal from
    /// genesis and rebuilds the blue_set, red_set, blue_score, and ordering.
    ///
    /// Note: This is O(n²) and should only be used when necessary (startup, recovery,
    /// or explicit user request). During normal operation, use the incremental update.
    pub fn rebuild_from_scratch(&mut self) -> crate::error::BlockchainResult<()> {
        self.update_blue_set()
    }

    /// Rebuild ordering from blocks already in storage.
    ///
    /// This is a recovery method for when checkpoint loading fails or returns
    /// an ordering that's too short for finalization (len <= finality_depth).
    /// It loads all blocks from storage, sorts them by block number, and
    /// rebuilds the ordering, blue_set, blue_score, and timestamp cache.
    ///
    /// For a healthy linear chain, all blocks are considered "blue".
    /// Blue score = position in the chain (genesis = 1, block N = N+1).
    fn rebuild_ordering_from_storage(&mut self) -> crate::error::BlockchainResult<usize> {
        // Get all blocks from storage (hot cache + disk)
        let blocks = self.storage.get_all_blocks()?;

        if blocks.is_empty() {
            return Ok(0);
        }

        // Sort blocks by block number (topological order for linear chain)
        let mut sorted_blocks = blocks;
        sorted_blocks.sort_by_key(|b| b.header.block_number);

        // Clear existing state
        self.blue_set.clear();
        self.red_set.clear();
        self.blue_score.clear();
        self.block_timestamps.clear();
        self.ordering.clear();

        // Rebuild ordering, blue_set, blue_score from sorted blocks
        // For a linear chain, all blocks are blue. Blue score = position + 1.
        for (position, block) in sorted_blocks.iter().enumerate() {
            let hash = block.hash;
            let blue_score = (position + 1) as u64; // Genesis = 1, block 1 = 2, etc.

            self.blue_set.insert(hash);
            self.blue_score.insert(hash, blue_score);
            self.block_timestamps.insert(hash, block.header.timestamp);
            self.ordering.push(hash);

            // Detect genesis (no parents)
            if block.header.parent_hashes.is_empty() && self.genesis_hash.is_none() {
                self.genesis_hash = Some(hash);
            }
        }

        // Ordering is built with oldest block at index 0 (ascending by blue score).
        // But GhostDAG ordering expects: highest blue score first (descending).
        // Reverse to match expected ordering: tip first, genesis last.
        self.ordering.reverse();

        Ok(self.ordering.len())
    }

    /// Incremental blue set update: only recompute for the new block and its descendants.
    /// Avoids full BFS from genesis (O(n²) → O(affected)).
    fn update_blue_set_incremental(
        &mut self,
        new_block_hash: Hash,
    ) -> crate::error::BlockchainResult<()> {
        let genesis_hash = match self.genesis_hash {
            Some(h) => h,
            None => return Ok(()),
        };
        if self.storage.get_block(&genesis_hash)?.is_none() {
            return Ok(());
        }

        // STALE CHECKPOINT DETECTION:
        // If the new block's parents are in hot_blocks but NOT in blue_set, the checkpoint
        // blue_set is stale (from a previous chain state). Clear and rebuild from scratch.
        // This happens when blocks are loaded from disk but checkpoint blue_set doesn't match.
        if let Some(block) = self.storage.get_block(&new_block_hash)? {
            let parents = &block.header.parent_hashes;
            if !parents.is_empty() {
                let hot_blocks = self.storage.get_hot_blocks();
                let parents_in_hot = parents.iter().any(|p| hot_blocks.contains_key(p));
                let parents_in_blue = parents.iter().any(|p| self.blue_set.contains(p));

                if parents_in_hot && !parents_in_blue {
                    if !self.checkpoint_stale_rebuilt {
                        // First occurrence: genuine stale checkpoint from disk. Rebuild once.
                        warn!(
                            "[GhostDAG] Stale checkpoint detected: parents in hot_blocks but not in blue_set. Rebuilding blue_set from {} hot blocks",
                            hot_blocks.len()
                        );
                        self.blue_set.clear();
                        self.red_set.clear();
                        self.blue_score.clear();
                        self.ordering.clear();
                        self.checkpoint_stale_rebuilt = true;
                        return self.update_blue_set();
                    } else {
                        // Already rebuilt this session. Parent arrived out-of-order during burst
                        // sync — insert it directly rather than triggering another O(n²) BFS.
                        let next_score = self.blue_score.values().max().copied().unwrap_or(0) + 1;
                        for p in parents.iter() {
                            if hot_blocks.contains_key(p) && !self.blue_set.contains(p) {
                                self.blue_set.insert(*p);
                                self.blue_score.entry(*p).or_insert(next_score);
                            }
                        }
                    }
                }
            }
        }

        // OPTIMIZATION 3: Early exit for tip blocks (90%+ of blocks during normal mining)
        // If the new block has no children, the affected set is just the block itself.
        let children = self.storage.get_children(&new_block_hash)?;
        let affected = if children.is_empty() {
            // Tip block: skip BFS entirely
            HashSet::from([new_block_hash])
        } else {
            // Block has descendants (out-of-order arrival or sync): run full BFS
            let mut affected = HashSet::new();
            let mut queue = VecDeque::new();
            queue.push_back(new_block_hash);
            affected.insert(new_block_hash);
            while let Some(h) = queue.pop_front() {
                let children = self.storage.get_children(&h)?;
                for child in children {
                    if affected.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
            affected
        };

        // 2) Remove affected entries so we recompute them (keep rest of DAG unchanged)
        // Track changes for incremental ordering update
        let mut blocks_added_to_blue: HashSet<Hash> = HashSet::new();
        let mut blocks_removed_from_blue: HashSet<Hash> = HashSet::new();
        for h in &affected {
            // Track blocks being removed from blue_set (for ordering update)
            if self.blue_set.remove(h) {
                blocks_removed_from_blue.insert(*h);
            }
            self.blue_score.remove(h);
            self.red_set.remove(h);
        }

        // 3) Topological order of affected set (parents before children)
        // In-degree = number of parents that are in affected set
        let mut in_degree: HashMap<Hash, u32> = HashMap::new();
        for &h in &affected {
            let block = match self.storage.get_block(&h)? {
                Some(b) => b,
                None => continue,
            };
            let parents_in_affected = block
                .header
                .parent_hashes
                .iter()
                .filter(|p| {
                    let parent: Hash = **p;
                    affected.contains(&parent)
                })
                .count();
            in_degree.insert(h, parents_in_affected as u32);
        }
        let mut topo = Vec::with_capacity(affected.len());
        let mut topo_queue = VecDeque::new();
        for &h in &affected {
            if in_degree.get(&h).copied().unwrap_or(1) == 0 {
                topo_queue.push_back(h);
            }
        }
        while let Some(h) = topo_queue.pop_front() {
            topo.push(h);
            let children = self.storage.get_children(&h)?;
            for child in children {
                if affected.contains(&child) {
                    if let Some(d) = in_degree.get_mut(&child) {
                        *d = d.saturating_sub(1);
                        if *d == 0 {
                            topo_queue.push_back(child);
                        }
                    }
                }
            }
        }

        // 4) Process in topological order: blue score = max(blue parent scores) + 1
        for hash in topo {
            let block = match self.storage.get_block(&hash)? {
                Some(b) => b,
                None => continue,
            };
            let parent_scores: Vec<u64> = block
                .header
                .parent_hashes
                .iter()
                .filter(|p| self.blue_set.contains(*p))
                .filter_map(|p| self.blue_score.get(p).copied())
                .collect();
            if parent_scores.is_empty() {
                // Special case: genesis block has no parents, always blue with score 1
                if block.header.parent_hashes.is_empty() {
                    self.blue_set.insert(hash);
                    self.blue_score.insert(hash, 1);
                } else {
                    self.red_set.insert(hash);
                    // Track if this block was previously blue (now moved to red)
                    if blocks_removed_from_blue.contains(&hash) {
                        // Already tracked in blocks_removed_from_blue
                    }
                }
            } else {
                let max_parent = parent_scores.into_iter().max().unwrap_or(0);
                let score = max_parent + 1;
                self.blue_score.insert(hash, score);
                self.blue_set.insert(hash);
                // Track blocks newly added to blue_set
                if blocks_removed_from_blue.contains(&hash) {
                    // Block was blue, still blue - no net change to ordering
                    blocks_removed_from_blue.remove(&hash);
                } else {
                    // Block is newly added to blue_set
                    blocks_added_to_blue.insert(hash);
                }
            }
        }

        // FALLBACK: Ensure new blocks are correctly classified as blue on linear chains
        // This handles cases where the incremental update might miss blocks due to
        // checkpoint loading inconsistencies or hash comparison issues.
        // On a linear chain (single miner), if all parents are blue, the block should be blue.

        // Collect information about blocks that might need reclassification
        // We do this in separate steps to avoid borrow checker issues with closures
        let mut new_block_parents: Option<Vec<Hash>> = None;
        let mut affected_red_blocks: Vec<(Hash, Vec<Hash>)> = Vec::new();

        // Check the new block if it wasn't added to blue_set
        let new_block_in_blue = self.blue_set.contains(&new_block_hash);
        if !new_block_in_blue {
            if let Ok(Some(block)) = self.storage.get_block(&new_block_hash) {
                let parents = block.header.parent_hashes.clone();
                let all_parents_blue = parents.iter().all(|p| self.blue_set.contains(p));
                if all_parents_blue || parents.is_empty() {
                    new_block_parents = Some(parents);
                }
            }
        }

        // Check affected blocks that are in red_set
        for hash in &affected {
            if self.red_set.contains(hash) && !self.blue_set.contains(hash) {
                if let Ok(Some(block)) = self.storage.get_block(hash) {
                    let parents = block.header.parent_hashes.clone();
                    let all_parents_blue = parents.iter().all(|p| self.blue_set.contains(p));
                    if all_parents_blue || parents.is_empty() {
                        affected_red_blocks.push((*hash, parents));
                    }
                }
            }
        }

        // Process the new block if it needs to be classified as blue
        if let Some(parents) = new_block_parents {
            let parent_scores: Vec<u64> = parents
                .iter()
                .filter_map(|p| self.blue_score.get(p).copied())
                .collect();

            let score = if parent_scores.is_empty() {
                1
            } else {
                parent_scores.into_iter().max().unwrap_or(0) + 1
            };

            self.blue_score.insert(new_block_hash, score);
            self.blue_set.insert(new_block_hash);
            self.red_set.remove(&new_block_hash);
            blocks_added_to_blue.insert(new_block_hash);
        }

        // Process affected blocks that should be blue
        for (hash, parents) in affected_red_blocks {
            let parent_scores: Vec<u64> = parents
                .iter()
                .filter_map(|p| self.blue_score.get(p).copied())
                .collect();

            let score = if parent_scores.is_empty() {
                1
            } else {
                parent_scores.into_iter().max().unwrap_or(0) + 1
            };

            self.blue_score.insert(hash, score);
            self.blue_set.insert(hash);
            self.red_set.remove(&hash);
            blocks_removed_from_blue.remove(&hash);
            blocks_added_to_blue.insert(hash);
        }

        // OPTIMIZATION 2: Incremental ordering update instead of full re-sort
        // Only update ordering for blocks that actually changed (O(log n) vs O(n log n))

        // Remove blocks that transitioned from blue to red
        for h in &blocks_removed_from_blue {
            if let Some(pos) = self.ordering.iter().position(|x| *x == *h) {
                self.ordering.remove(pos);
            }
        }

        // Insert newly blue blocks using binary search
        for h in &blocks_added_to_blue {
            let score = self.blue_score.get(h).copied().unwrap_or(0);
            // Use timestamp cache instead of fetching full block from storage
            let timestamp = self.block_timestamps.get(h).copied().unwrap_or(0);

            // Binary search for correct position: sorted by score desc, then timestamp asc
            let pos = self.ordering.binary_search_by(|probe| {
                let probe_score = self.blue_score.get(probe).copied().unwrap_or(0);
                match score.cmp(&probe_score) {
                    std::cmp::Ordering::Equal => {
                        // Use timestamp cache instead of disk read (PER-005 optimization)
                        let probe_timestamp =
                            self.block_timestamps.get(probe).copied().unwrap_or(0);
                        timestamp.cmp(&probe_timestamp)
                    }
                    other => other,
                }
            });

            // Insert at the found position (or where it should go)
            match pos {
                Ok(idx) => self.ordering.insert(idx, *h),
                Err(idx) => self.ordering.insert(idx, *h),
            }
        }

        // Note: If no blue_set changes occurred, ordering is already correct (no-op)

        Ok(())
    }

    /// Get blocks in final consensus order
    ///
    /// Note: Returns owned blocks since storage may return from disk.
    /// For performance, consider caching frequently accessed blocks.
    pub fn get_ordered_blocks(&self) -> crate::error::BlockchainResult<Vec<Block>> {
        let mut blocks = Vec::new();
        for hash in &self.ordering {
            if let Some(block) = self.storage.get_block(hash)? {
                blocks.push(block);
            }
        }
        Ok(blocks)
    }

    /// Get blue set (selected blocks for consensus)
    pub fn get_blue_set(&self) -> &HashSet<Hash> {
        &self.blue_set
    }

    /// Get red set (blocks not selected)
    pub fn get_red_set(&self) -> &HashSet<Hash> {
        &self.red_set
    }

    /// Get blue score for a block
    pub fn get_blue_score(&self, hash: &Hash) -> Option<u64> {
        self.blue_score.get(hash).copied()
    }

    /// Finality depth for "finalized" block (blocks with this many confirmations). Uses config or 1.
    fn finality_depth(&self) -> usize {
        let n = self.storage.confirmations_for_checkpoint();
        if n > 0 {
            n
        } else {
            1
        }
    }

    /// Hash of the finalized tip (block with at least finality_depth confirmations). None if not enough blocks.
    pub fn get_finalized_block_hash(&self) -> crate::error::BlockchainResult<Option<Hash>> {
        let depth = self.finality_depth();
        if self.ordering.len() <= depth {
            return Ok(None);
        }
        Ok(Some(self.ordering[depth]))
    }

    /// Block number of the finalized tip (for RPC "finalized" / "safe" tags). None if not enough blocks.
    pub fn get_finalized_block_number(&self) -> crate::error::BlockchainResult<Option<u64>> {
        let hash = self.get_finalized_block_hash()?;
        let Some(hash) = hash else {
            return Ok(None);
        };
        let block = self.storage.get_block(&hash)?;
        Ok(block.map(|b| b.header.block_number))
    }

    /// Prune old red blocks from the DAG to bound memory usage.
    /// Should be called periodically (e.g., every 100 blocks) from the blockchain layer.
    pub fn prune_old_blocks(&mut self, current_height: u64) {
        let finality_depth = 100; // Match FINALITY_DEPTH constant
        let pruned = self
            .storage
            .prune_red_blocks(current_height, finality_depth);
        if pruned > 0 {
            debug!(
                "GhostDAG: pruned {} old red blocks at height {}",
                pruned, current_height
            );
        }
    }

    /// Clear all GhostDAG state (used during full resync)
    pub fn clear(&mut self) {
        self.genesis_hash = None;
        self.blue_set.clear();
        self.red_set.clear();
        self.blue_score.clear();
        self.block_timestamps.clear();
        self.ordering.clear();
        self.storage.clear();
    }

    /// Get total number of blocks in DAG
    ///
    /// Note: This returns the count from hot cache only.
    /// For total count including disk, we'd need to query storage.
    pub fn get_block_count(&self) -> usize {
        self.storage.get_hot_blocks().len()
    }

    /// Get number of blue blocks
    pub fn get_blue_block_count(&self) -> usize {
        self.blue_set.len()
    }

    /// Get number of red blocks
    pub fn get_red_block_count(&self) -> usize {
        self.red_set.len()
    }

    /// Calculate transactions per second from ordered blocks
    pub fn get_tps(&self, duration_seconds: u64) -> f64 {
        if self.ordering.is_empty() {
            return 0.0;
        }

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Get blocks from recent duration (from storage)
        let mut recent_blocks: Vec<Block> = Vec::new();
        for hash in &self.ordering {
            if let Ok(Some(block)) = self.storage.get_block(hash) {
                let age = current_time.saturating_sub(block.header.timestamp);
                if age <= duration_seconds {
                    recent_blocks.push(block);
                }
            }
        }

        if recent_blocks.is_empty() {
            return 0.0;
        }

        let total_txs: usize = recent_blocks.iter().map(|b| b.transactions.len()).sum();

        let timestamps: Vec<u64> = recent_blocks.iter().map(|b| b.header.timestamp).collect();

        let time_span = timestamps
            .iter()
            .max()
            .and_then(|max| timestamps.iter().min().map(|min| max - min))
            .unwrap_or(1);

        if time_span == 0 {
            return 0.0;
        }

        total_txs as f64 / time_span as f64
    }

    /// Get DAG statistics
    ///
    /// Note: Statistics are calculated from hot cache only for performance.
    /// For complete statistics including disk blocks, this would need to scan storage.
    pub fn get_stats(&self) -> DAGStats {
        let hot_blocks = self.storage.get_hot_blocks();
        let total_blocks = hot_blocks.len();

        let total_txs: usize = hot_blocks.values().map(|b| b.transactions.len()).sum();

        let total_size: usize = hot_blocks
            .values()
            .map(|b| {
                // Approximate block size
                std::mem::size_of::<Block>()
                    + b.transactions.len() * std::mem::size_of::<crate::blockchain::Transaction>()
            })
            .sum();

        // Derive red_blocks from total - blue to ensure consistency
        // (red_set may contain stale entries from incremental updates)
        let blue_blocks = self.blue_set.len();
        let red_blocks = if total_blocks > blue_blocks {
            total_blocks - blue_blocks
        } else {
            0
        };

        DAGStats {
            total_blocks,
            blue_blocks,
            red_blocks,
            total_transactions: total_txs,
            total_size_bytes: total_size,
            avg_block_size: total_size.checked_div(total_blocks).unwrap_or(0),
            avg_txs_per_block: if total_blocks == 0 {
                0.0
            } else {
                total_txs as f64 / total_blocks as f64
            },
        }
    }

    /// Get block by hash
    ///
    /// Returns block from storage (hot cache or disk).
    pub fn get_block(&self, hash: &Hash) -> crate::error::BlockchainResult<Option<Block>> {
        self.storage.get_block(hash)
    }

    /// Check if block is in blue set
    pub fn is_blue(&self, hash: &Hash) -> bool {
        self.blue_set.contains(hash)
    }

    /// Check if block is in red set
    pub fn is_red(&self, hash: &Hash) -> bool {
        self.red_set.contains(hash)
    }

    /// Get current DAG tips (blocks with no children / highest blue-score leaves)
    ///
    /// Tips are blocks in the DAG that have no children pointing to them.
    /// Returns up to 3 tips with the highest blue scores.
    /// If DAG is empty or only has genesis, returns genesis hash.
    pub fn get_tips(&self) -> Vec<Hash> {
        // If no blocks in DAG, return empty
        if self.blue_set.is_empty() && self.red_set.is_empty() {
            return Vec::new();
        }

        // Get genesis hash if available
        let genesis = match self.genesis_hash {
            Some(h) => h,
            None => return Vec::new(),
        };

        // Find all blocks that have no children (tips)
        // A block is a tip if get_children(hash) returns empty
        let mut tips: Vec<(Hash, u64)> = Vec::new();

        // Check all blocks in blue set
        for hash in &self.blue_set {
            match self.storage.get_children(hash) {
                Ok(children) => {
                    if children.is_empty() {
                        // This is a tip - get its blue score
                        let score = self.blue_score.get(hash).copied().unwrap_or(0);
                        tips.push((*hash, score));
                    }
                }
                Err(_) => {
                    // If we can't get children, treat as tip (defensive)
                    let score = self.blue_score.get(hash).copied().unwrap_or(0);
                    tips.push((*hash, score));
                }
            }
        }

        // Also check red set for tips (they might not have children yet)
        for hash in &self.red_set {
            match self.storage.get_children(hash) {
                Ok(children) => {
                    if children.is_empty() {
                        // Red block with no children - it's also a tip
                        // Use blue score of 0 for red blocks
                        tips.push((*hash, 0));
                    }
                }
                Err(_) => {
                    tips.push((*hash, 0));
                }
            }
        }

        // If no tips found but we have genesis, return genesis
        if tips.is_empty() {
            return vec![genesis];
        }

        // Sort tips by blue score (descending) - highest score first
        tips.sort_by_key(|b| std::cmp::Reverse(b.1));

        // Return top K tips (or fewer if less available)
        // K is the GhostDAG security parameter
        tips.into_iter()
            .take(self.k)
            .map(|(hash, _)| hash)
            .collect()
    }
}

/// DAG statistics
#[derive(Debug, Clone)]
pub struct DAGStats {
    pub total_blocks: usize,
    pub blue_blocks: usize,
    pub red_blocks: usize,
    pub total_transactions: usize,
    pub total_size_bytes: usize,
    pub avg_block_size: usize,
    pub avg_txs_per_block: f64,
}
