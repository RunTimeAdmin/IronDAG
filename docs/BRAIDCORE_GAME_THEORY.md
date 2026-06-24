# BraidCore Game Theory Analysis

**Date:** April 2026  
**Scope:** Formal analysis of BraidCore cross-stream failure modes and game-theoretic attack vectors  
**Codebase Version:** IronDAG Blockchain (current main)

---

## Executive Summary

This document provides a formal analysis of the BraidCore multi-stream mining architecture, identifying shared resource contention points, cross-stream attack vectors, liveness denial scenarios, and economic incentive misalignments. All findings are derived from direct code analysis with specific file:line references.

**Key Findings:**
- The `BlockNumberAllocator` creates a potential starvation vector via free-list recycling
- The `BLOCK_PROCESSING_LOCK` serializes all block commits, creating a throughput bottleneck
- Stream C can potentially flood the system with low-cost blocks due to no PoW requirement
- Difficulty is adjusted independently per-stream, creating potential cross-stream manipulation vectors
- GhostDAG ordering is stream-agnostic, which is correct but lacks stream-specific weight adjustments

---

## 1. Architecture Overview

### 1.1 BraidCore Design

The BraidCore architecture implements three parallel mining streams with different characteristics:

| Stream | Algorithm    | Block Time | Max Txs | Block Reward | Hardware Target |
|--------|--------------|------------|---------|--------------|-----------------|
| **A**  | Blake3       | 10s        | 10,000  | 50 IDAG      | ASIC            |
| **B**  | KHeavyHash   | 5s         | 5,000   | 25 IDAG      | CPU/GPU         |
| **C**  | Keccak256    | 1s         | 1,000   | 0 (fee-only) | Any             |

**Code References:**
- `mining.rs:1-11` — Stream definitions and block times
- `mining.rs:43-66` — Reward constants and halving schedule
- `mining.rs:68-78` — Block time and max transaction constants

### 1.2 Shared Infrastructure

All three streams share critical infrastructure components:

#### 1.2.1 BlockNumberAllocator

```
Location: mining.rs:255-368
```

The `BlockNumberAllocator` is a single shared instance (`Arc<BlockNumberAllocator>`) that coordinates block numbering across all streams:

- **`next_available: Arc<AtomicU64>`** — Monotonically increasing counter
- **`pending_reservations: Arc<RwLock<HashMap<u64, (StreamType, Instant)>>>`** — Tracks reserved numbers
- **`free_list: Arc<Mutex<BinaryHeap<Reverse<u64>>>>`** — SEC-015: Recycled block numbers

**Reservation Flow (`mining.rs:288-300`):**
```rust
pub async fn reserve(&self, stream_type: StreamType) -> u64 {
    // SEC-015: Try to reuse a released block number from free-list first
    let recycled = self.free_list.lock().await.pop();
    if let Some(Reverse(num)) = recycled {
        self.pending_reservations.write().await.insert(num, (stream_type, std::time::Instant::now()));
        return num;
    }
    // No recycled numbers available — allocate new
    let mut pending = self.pending_reservations.write().await;
    let num = self.next_available.fetch_add(1, Ordering::SeqCst);
    pending.insert(num, (stream_type, std::time::Instant::now()));
    num
}
```

#### 1.2.2 BLOCK_PROCESSING_LOCK

```
Location: blockchain/mod.rs:51-55
```

```rust
static BLOCK_PROCESSING_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
```

This global mutex serializes the entire block processing pipeline:
- Acquired at the start of `add_block()` (`blockchain/mod.rs:614`)
- Also acquired in `add_block_for_sync()` (`blockchain/mod.rs:787`)
- Released only after full validation, GhostDAG update, and state persistence

**Critical Section (`blockchain/mod.rs:608-777`):**
```rust
pub fn add_block(&mut self, block: Block) -> crate::error::BlockchainResult<()> {
    let _processing_guard = BLOCK_PROCESSING_LOCK.lock();
    // ... entire validation and commit pipeline ...
}
```

#### 1.2.3 GhostDAG Consensus Engine

```
Location: consensus/mod.rs:26-42
```

The GhostDAG engine is shared across all streams:
- Blocks from all streams are added to the same DAG structure
- Blue/red classification is stream-agnostic
- Ordering uses blue score + timestamp, not stream type

**Shared State:**
- `blue_set: HashSet<Hash>` — Blue blocks (selected for consensus)
- `red_set: HashSet<Hash>` — Red blocks (not selected)
- `blue_score: HashMap<Hash, u64>` — Blue score per block
- `ordering: Vec<Hash>` — Final consensus ordering

#### 1.2.4 ParentHashCoordinator

```
Location: mining.rs:370-416
```

Coordinates parent hash selection across streams to reduce race conditions:
- Uses a `coordinator_lock: Arc<Mutex<()>>` for atomic selection
- Selects last 3 blocks as parents regardless of stream type

---

## 2. Shared Resource Contention Analysis

### 2.1 Block Number Allocator

#### 2.1.1 Fairness Analysis

The allocator uses a simple first-come-first-served reservation model. Streams compete for block numbers via:

1. **Direct allocation** (`fetch_add`) — New numbers from `next_available`
2. **Free-list recycling** (`pop`) — Reused numbers from failed validations

**Fairness Properties:**
- No stream priority in allocation — all streams have equal access
- No rate limiting per stream — a fast stream can reserve more numbers
- No starvation detection — no mechanism to ensure fair distribution

**Code Reference (`mining.rs:296-299`):**
```rust
let mut pending = self.pending_reservations.write().await;
let num = self.next_available.fetch_add(1, Ordering::SeqCst);
pending.insert(num, (stream_type, std::time::Instant::now()));
```

#### 2.1.2 Starvation Scenarios

**Scenario 1: Stream C Block Number Monopolization**

Stream C produces blocks every 1 second vs. Stream A's 10 seconds. If Stream C miners submit many blocks:
1. Stream C reserves more block numbers per unit time
2. Stream A miners find their reserved numbers increasingly ahead of committed height
3. The gap triggers allocator reset (`mining.rs:2587-2592`)

**Code Reference (`mining.rs:2587-2592`):**
```rust
if block_number > current_height + MAX_BLOCK_NUMBER_GAP {
    error!("MiningManager: Block number {} too far ahead of current height {} — resetting allocator to {}", 
           block_number, current_height, current_height + 1);
    block_allocator.reset_to(current_height + 1).await;
```

**Scenario 2: Free-List Attack**

An attacker could intentionally fail validations to populate the free-list:
1. Reserve block numbers from Stream C (no PoW cost)
2. Submit invalid blocks to trigger `release()`
3. Free-list grows with low numbers
4. Legitimate Stream A/B blocks get recycled numbers

**Code Reference (`mining.rs:311-318`):**
```rust
pub async fn release(&self, block_number: u64, _stream_type: StreamType) {
    if self.pending_reservations.write().await.remove(&block_number).is_some() {
        self.failed_reservations.fetch_add(1, Ordering::SeqCst);
        // SEC-015: Push released number onto free-list for reuse
        self.free_list.lock().await.push(Reverse(block_number));
    }
}
```

**Mitigation (Partially Implemented):**
- `MAX_BLOCK_NUMBER_GAP = 50` (`mining.rs:2516`) prevents excessive gaps
- Stale reservation cleanup (`mining.rs:343-367`) with 60-second timeout

### 2.2 BLOCK_PROCESSING_LOCK

#### 2.2.1 Serialization Bottleneck

The global lock creates a single serialization point for all block commits:

**Impact Analysis:**
| Stream | Block Rate | Lock Acquisitions/sec (theoretical) |
|--------|------------|-------------------------------------|
| A      | 0.1 Hz     | 0.1                                 |
| B      | 0.2 Hz     | 0.2                                 |
| C      | 1 Hz       | 1.0                                 |
| **Total** | —      | **~1.3 Hz**                         |

At 1.3 Hz lock acquisition rate, the system can theoretically process ~1.3 blocks/second. However:

1. **Lock hold time** includes:
   - Block structure validation
   - Hash verification (different algorithms per stream)
   - ZK proof verification (Stream C, if `privacy` feature enabled)
   - GhostDAG update (O(affected) for incremental)
   - State persistence to disk

2. **Contention point:**
   - Multiple streams compete for the same lock
   - Stream C's 1-second block time means more frequent lock requests
   - Mining threads block while waiting for commit

**Code Reference (`blockchain/mod.rs:614`):**
```rust
let _processing_guard = BLOCK_PROCESSING_LOCK.lock();
```

#### 2.2.2 Priority Inversion Risk

The lock treats all streams equally. However:
- Stream A blocks (50 IDAG reward) have equal priority to Stream C blocks (fee-only)
- A Stream C block being processed blocks a Stream A block waiting for the lock
- No priority mechanism exists to prefer higher-value blocks

### 2.3 GhostDAG Write Lock Contention

GhostDAG is accessed via `Arc<RwLock<GhostDAG>>`:
- **Writers:** Block commits (via `add_block`), pruning, sync
- **Readers:** Parent selection (`get_tips`), RPC queries

**Code Reference (`mining.rs:1819-1833`):**
```rust
let parent_hashes = if let Some(ref ghostdag) = self.ghostdag {
    let dag = ghostdag.read().await;
    let tips = dag.get_tips();
    drop(dag);
    // ...
}
```

**Contention Points:**
1. Mining threads need read lock for parent selection
2. Block commits need write lock via `add_block` to GhostDAG
3. RPC endpoints may hold read locks for queries

**Mitigation:**
- `try_write()` with fallback to `blocking_write()` (`blockchain/mod.rs:427-441`)
- Incremental blue set update (`consensus/mod.rs:346-500`) reduces O(n²) to O(affected)

---

## 3. Cross-Stream Attack Vectors

### 3.1 Stream C Flooding

**Attack Vector:** Can Stream C's low-cost submissions starve Streams A/B?

**Analysis:**

Stream C has significant advantages for potential flooding:
1. **No Proof-of-Work:** Stream C uses Keccak256 hashing, not PoW (`mining.rs:2430-2436`)
2. **No ZK Proof Required:** ZK proving is optional with `privacy` feature flag
3. **Fast Block Time:** 1-second vs. 10-second (A) and 5-second (B)
4. **Fee-Only Incentive:** Miners only earn transaction fees

**Code Reference (`mining.rs:2430-2436`):**
```rust
// Stream C has no PoW, but still enforce size limit
let trimmed_txs = trim_block_transactions(&header, &txs, "Stream C");
block = Block::new(header.clone(), trimmed_txs);
```

**Flooding Scenario:**
1. Attacker creates many empty/near-empty Stream C blocks
2. Each block reserves a block number via `BlockNumberAllocator`
3. Blocks enter the `process_blocks` queue via `mpsc::unbounded_channel`
4. Stream C's 1-second rate allows ~10x more submissions than Stream A

**Existing Mitigations:**
- `MAX_BLOCK_NUMBER_GAP = 50` — Allocator resets if gap too large (`mining.rs:2516`)
- `BLOCK_PROCESSING_LOCK` — Serializes commits, but doesn't prevent queue buildup
- Per-stream transaction pool limits (`mining.rs:84-88`)

**Gap:** No rate limiting per stream for block submissions.

### 3.2 Block Number Exhaustion

**Attack Vector:** Can one stream monopolize block numbers?

**Analysis:**

The allocator uses `AtomicU64::fetch_add` for allocation:
- No per-stream quotas
- No rate limiting
- Stream C's faster block rate means more reservations

**Code Reference (`mining.rs:296-299`):**
```rust
let num = self.next_available.fetch_add(1, Ordering::SeqCst);
```

**Attack Scenario:**
1. Malicious Stream C miner produces many blocks
2. Each reserves a block number before validation
3. Legitimate Stream A blocks find higher numbers reserved
4. Gap between `next_available` and committed height grows

**Existing Mitigation:**
- `reset_to()` when gap exceeds `MAX_BLOCK_NUMBER_GAP` (`mining.rs:2591-2592`)
- Stale reservation cleanup (`mining.rs:320-334`)

**Remaining Risk:**
- Attack can cause repeated allocator resets
- Legitimate blocks may be rejected during reset
- Free-list accumulates unused numbers

### 3.3 Selective Withholding

**Attack Vector:** Can a miner on Stream A delay Stream B blocks?

**Analysis:**

Parent hash selection uses GhostDAG tips, which include blocks from all streams:
- No stream-specific isolation in parent selection
- Stream A blocks can be parents of Stream B blocks and vice versa

**Code Reference (`consensus/mod.rs:706-740`):**
```rust
// Find all blocks that have no children (tips)
// A block is a tip if get_children(hash) returns empty
let mut tips: Vec<(Hash, u64)> = Vec::new();

// Check all blocks in blue set
for hash in &self.blue_set {
    match self.storage.get_children(hash) {
        Ok(children) => {
            if children.is_empty() {
                let score = self.blue_score.get(hash).copied().unwrap_or(0);
                tips.push((*hash, score));
            }
        }
        // ...
    }
}

// Also check red set for tips
for hash in &self.red_set {
    // ...
}
```

**Withholding Attack Scenario:**
1. Malicious miner withholds Stream A block after PoW
2. Other streams continue mining on older tips
3. Withheld block is released when strategically advantageous
4. Creates a temporary fork that may orphan other streams' blocks

**Mitigations:**
- GhostDAG's DAG structure tolerates out-of-order arrival
- Blue score ordering prioritizes earlier blocks (by score, then timestamp)

**Gap:** No time-based penalty for late-arriving blocks that were withheld.

### 3.4 Parent Hash Manipulation

**Attack Vector:** Can stream-specific parent selection be exploited?

**Analysis:**

The `ParentHashCoordinator` selects parents atomically:
- Uses a mutex to serialize selection across streams
- Selects last 3 blocks regardless of stream

**Code Reference (`mining.rs:382-395`):**
```rust
pub async fn select_parents(
    &self,
    blockchain: &Arc<RwLock<Blockchain>>,
    _stream_type: StreamType,
) -> Vec<Hash> {
    let _lock = self.coordinator_lock.lock().await;
    let blocks = blockchain.read().await.get_blocks();
    if blocks.is_empty() {
        return Vec::new();
    }
    let start_idx = if blocks.len() >= 3 { blocks.len() - 3 } else { 0 };
    blocks[start_idx..].iter().map(|b| b.hash).collect()
}
```

**Observations:**
1. `_stream_type` parameter is **unused** — no stream-specific parent selection
2. Last 3 blocks selected regardless of stream composition
3. No validation that parents are from different streams

**Exploit Scenario:**
A miner could:
1. Produce a series of Stream C blocks (fast, no PoW)
2. These blocks become the "last 3" parents
3. Higher-value Stream A/B blocks build on Stream C chain
4. Potential for economic manipulation if Stream C blocks have specific transactions

**GhostDAG Override:**
When GhostDAG is configured, parent selection uses DAG tips:
```rust
let parent_hashes = if let Some(ref ghostdag) = self.ghostdag {
    let dag = ghostdag.read().await;
    let tips = dag.get_tips();
    // ...
}
```

GhostDAG tips are selected by blue score, providing some protection.

### 3.5 Difficulty Manipulation

**Attack Vector:** Can one stream's difficulty affect another?

**Analysis:**

Difficulty is tracked **independently** per stream:

**Code Reference (`mining.rs:1682`, `mining.rs:1981`):**
```rust
// Stream A
let mut current_difficulty = pow::INITIAL_DIFFICULTY_A;

// Stream B
let mut current_difficulty = pow::INITIAL_DIFFICULTY_B;
```

**Adjustment Function (`pow.rs`):**
```rust
pub fn adjust_difficulty(current: u64, target_time: u64, actual_time: u64) -> u64 {
    // Independent adjustment based on block time
}
```

**Conclusion:** Streams have **independent** difficulty tracking. One stream's difficulty cannot directly affect another.

**However:**
- Difficulty adjustment uses local block time, not wall clock
- A stream could theoretically manipulate its reported timestamps
- Current code uses `mining_start.elapsed()` for actual time (`mining.rs:1899`, `mining.rs:2175`)

---

## 4. Liveness Denial Vectors

### 4.1 Single-Stream Failure

**Scenario:** What happens if all Stream A miners go offline?

**Analysis:**

Stream A produces ~17% of blocks (by emission analysis in `BRAIDCORE_MINING_ANALYSIS.md`). If Stream A halts:

1. **Block numbers continue:** Stream B and C can still reserve and commit blocks
2. **GhostDAG continues:** No dependency on Stream A blocks for consensus
3. **Transaction processing:** Stream B (5,000 txs/block) and C (1,000 txs/block) continue
4. **Reward loss:** 50 IDAG/block emission stops

**Code Reference:**
No code requires Stream A blocks for system operation. The architecture is **stream-agnostic** for consensus.

### 4.2 Consensus with 1 of 3 Streams

**Question:** Can consensus proceed with only 1 of 3 streams active?

**Answer:** **Yes.** GhostDAG does not require blocks from specific streams:

**Code Reference (`consensus/mod.rs:268-306`):**
```rust
while let Some(current) = queue.pop_front() {
    let children = self.storage.get_children(&current)?;
    
    for child_hash in children {
        // ...
        let block = match self.storage.get_block(&child_hash)? {
            Some(block) => block,
            None => continue,
        };
        
        let parent_scores: Vec<u64> = block.header.parent_hashes
            .iter()
            .filter(|parent_hash| self.blue_set.contains(*parent_hash))
            .filter_map(|parent_hash| self.blue_score.get(parent_hash).copied())
            .collect();
        
        if !parent_scores.is_empty() {
            let max_parent_score = parent_scores.iter().max().copied().unwrap_or(0);
            let child_blue_score = max_parent_score + 1;
            self.blue_score.insert(child_hash, child_blue_score);
            self.blue_set.insert(child_hash);
        } else {
            self.red_set.insert(child_hash);
        }
    }
}
```

The algorithm processes any block regardless of stream type. A single active stream can sustain consensus.

### 4.3 Block Allocator Deadlock

**Known Bug (Fixed):** Free-list recycling too-high numbers

**Code Reference (`mining.rs:2587-2592`):**
```rust
if block_number > current_height + MAX_BLOCK_NUMBER_GAP {
    block_allocator.reset_to(current_height + 1).await;
    // Return transactions to pool...
    continue;
}
```

**Historical Issue:**
Before the `reset_to()` fix, the free-list could accumulate block numbers higher than committed height. When these numbers were recycled:
1. Blocks received numbers far ahead of the chain
2. Gap validation failed
3. Blocks were rejected
4. Numbers returned to free-list
5. Infinite loop of invalid allocations

**Current Mitigation:**
- `reset_to()` clears the free-list and resets `next_available`
- `MAX_BLOCK_NUMBER_GAP = 50` provides reasonable bound

**Remaining Risk:**
Repeated resets during high load could cause instability.

---

## 5. Ordering and Finality Risks

### 5.1 Stream C Block Production Rate

**Question:** Can Stream C blocks (1s intervals) outpace GhostDAG ordering?

**Analysis:**

GhostDAG incremental update is O(affected blocks):
- New block affects itself and descendants
- Tip blocks (no children) only affect themselves
- Full rebuild only on empty blue set

**Code Reference (`consensus/mod.rs:356-376`):**
```rust
let affected = if children.is_empty() {
    // Tip block: skip BFS entirely
    HashSet::from([new_block_hash])
} else {
    // Block has descendants (out-of-order arrival or sync): run full BFS
    let mut affected = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(new_block_hash);
    // ...
}
```

**Stream C Impact:**
At 1 block/second, Stream C produces:
- 86,400 blocks/day
- ~6× Stream A's rate

**GhostDAG Scalability:**
- `HybridDagStorage` uses hot cache (1000 blocks) + disk
- Pruning at `finalized_depth: 500` bounds memory
- Checkpointing at `confirmations_for_checkpoint: 100`

**Conclusion:** GhostDAG can handle Stream C's rate. The incremental update avoids O(n²) scaling.

### 5.2 Blue/Red Classification Fairness

**Question:** Is blue/red classification fair across stream types?

**Analysis:**

Blue score calculation (`consensus/mod.rs:280-305`):
- Blue score = max(blue parent scores) + 1
- No stream-specific weighting
- Timestamp as secondary sort key

**Stream Production Rate Impact:**
- Stream C blocks arrive more frequently
- Each block gets a blue score based on parents
- Earlier arrival (lower timestamp) wins on equal blue score

**Potential Unfairness:**
Stream C's faster production could give it:
1. More opportunities to be selected as a tip
2. Higher likelihood of being in the K selected tips
3. More influence on future blue scores

**Mitigation:**
GhostDAG's K parameter limits tips to prevent tip flooding:
```rust
tips.into_iter().take(self.k).map(|(hash, _)| hash).collect()
```

### 5.3 Finality Guarantees

**Code Reference (`consensus/mod.rs:532-554`):**
```rust
fn finality_depth(&self) -> usize {
    let n = self.storage.confirmations_for_checkpoint();
    if n > 0 { n } else { 1 }
}

pub fn get_finalized_block_hash(&self) -> crate::error::BlockchainResult<Option<Hash>> {
    let depth = self.finality_depth();
    if self.ordering.len() <= depth {
        return Ok(None);
    }
    Ok(Some(self.ordering[depth]))
}
```

**Finality Parameters:**
- `confirmations_for_checkpoint: 100` blocks
- Blocks older than finality depth are considered finalized

**Cross-Stream Impact:**
- Finality is measured in block count, not time
- Stream C's faster production accelerates finality in block count
- But time-based finality would be similar (100 blocks ≈ 100 seconds with all streams)

---

## 6. Economic Incentive Analysis

### 6.1 Rational Miner Behavior

**Question:** Which stream maximizes revenue for a rational miner?

**Revenue Per Block:**
| Stream | Block Reward | Block Time | Revenue/Second |
|--------|--------------|------------|----------------|
| A      | 50 IDAG      | 10s        | 5 IDAG/s       |
| B      | 25 IDAG      | 5s         | 5 IDAG/s       |
| C      | 0 + fees     | 1s         | Variable       |

**Code Reference (`mining.rs:43-66`):**
```rust
pub const STREAM_A_REWARD: u128 = 50_000_000_000_000_000_000; // 50 IDAG
pub const STREAM_B_REWARD: u128 = 25_000_000_000_000_000_000; // 25 IDAG
pub const STREAM_C_REWARD: u128 = 0; // Fee-based only

pub fn get_block_reward(block_height: u64, stream: StreamType) -> u128 {
    let era = block_height / HALVING_INTERVAL;
    let base_reward = match stream {
        StreamType::StreamA => STREAM_A_REWARD,
        StreamType::StreamB => STREAM_B_REWARD,
        StreamType::StreamC => return 0,
    };
    // ...
}
```

**Analysis:**
1. **Streams A and B have equal revenue density** (5 IDAG/second)
2. **Stream C relies entirely on transaction fees**
3. **Hardware requirements differ:**
   - Stream A: ASIC-optimized Blake3
   - Stream B: CPU/GPU with memory-hard KHeavyHash
   - Stream C: Any hardware (no PoW)

**Rational Choice:**
- ASIC miner → Stream A
- GPU miner → Stream B
- No specialized hardware → Stream C (fee competition)

### 6.2 Cross-Stream MEV Opportunities

**Attack Vector:** Can MEV be extracted across streams?

**Analysis:**

Transactions are assigned to pools via round-robin:
```rust
let counter = self.tx_distribution_counter.fetch_add(1, Ordering::AcqRel);
let stream_index = counter % 3;
```

**MEV Scenario:**
1. Large swap transaction arrives
2. Assigned to Stream A pool (10s block time)
3. Attacker sees transaction in mempool
4. Attacker submits backrun to Stream C (1s block time)
5. Stream C block may be ordered before Stream A

**Mitigation:**
- `in_flight_txs` prevents same transaction in multiple streams (`mining.rs:1773-1785`)
- But this doesn't prevent related transactions across streams

### 6.3 Fee Market Imbalance

**Current State:**
- No EIP-1559-style base fee
- Fee-priority ordering (highest fee first)
- Per-stream pool limits

**Code Reference (`mining.rs:1752-1754`):**
```rust
// BPR-005: Sort by fee descending (highest fee first) before selection
pool.sort_unstable_by(|a, b| b.tx.fee.cmp(&a.tx.fee));
```

**Imbalance Scenario:**
1. High-fee transactions go to Stream A (10s blocks, more competition)
2. Low-fee transactions go to Stream C (1s blocks, less competition)
3. But Stream C has 1,000 tx/block limit vs. 10,000 for Stream A
4. Fee market fragmentation

**Code Reference (`mining.rs:68-88`):**
```rust
pub const STREAM_A_MAX_TXS: usize = 10_000;
pub const STREAM_B_MAX_TXS: usize = 5_000;
pub const STREAM_C_MAX_TXS: usize = 1_000;

pub const MAX_STREAM_A_POOL_SIZE: usize = 60_000;  // 60% of global
pub const MAX_STREAM_B_POOL_SIZE: usize = 30_000;  // 30% of global
pub const MAX_STREAM_C_POOL_SIZE: usize = 10_000;  // 10% of global
```

---

## 7. Mitigations (Existing and Recommended)

### 7.1 Existing Mitigations

| Vulnerability | Mitigation | Location | Status |
|---------------|------------|----------|--------|
| Block number exhaustion | `MAX_BLOCK_NUMBER_GAP` reset | `mining.rs:2516,2587-2592` | ✅ Implemented |
| Stale reservations | Periodic cleanup (60s timeout) | `mining.rs:343-367` | ✅ Implemented |
| Double-spend in DAG | `spent_outputs` tracking | `blockchain/mod.rs:481-510` | ✅ Implemented |
| Transaction duplication | `in_flight_txs` + `pool_tx_hashes` | `mining.rs:465-472` | ✅ Implemented |
| GhostDAG memory growth | Hybrid storage + pruning | `consensus/mod.rs:92-96` | ✅ Implemented |
| TOCTOU race condition | `BLOCK_PROCESSING_LOCK` | `blockchain/mod.rs:51-55` | ✅ Implemented |
| Pool overflow | Per-stream hard caps | `mining.rs:84-88` | ✅ Implemented |
| Fee-based eviction | ARC-008 lowest-fee eviction | `mining.rs:965-1007` | ✅ Implemented |

### 7.2 Recommended Improvements

| Issue | Recommendation | Priority | Complexity |
|-------|----------------|----------|------------|
| **Stream C flooding** | Add rate limiting per stream for block submissions (e.g., max N blocks per minute per stream) | **High** | Medium |
| **Block number starvation** | Implement per-stream quotas in `BlockNumberAllocator` | Medium | Low |
| **Priority inversion** | Add stream priority to `BLOCK_PROCESSING_LOCK` (prioritize Stream A over C) | Medium | Medium |
| **Withholding attack** | Add timestamp penalty for late-arriving blocks in GhostDAG ordering | Medium | High |
| **Parent hash manipulation** | Validate parent stream diversity (at least 1 parent from each active stream) | Low | Low |
| **Fee market fragmentation** | Implement unified fee market with EIP-1559 base fee | Low | High |
| **Cross-stream MEV** | Add transaction relationship detection in `FairnessAnalyzer` | Low | Medium |
| **Stream C ZK proofs** | Enforce ZK proof requirement when `privacy` feature enabled | **High** | Medium |

---

## 8. Summary of Risk Matrix

| Attack Vector | Severity | Likelihood | Current Mitigation | Status |
|---------------|----------|------------|-------------------|--------|
| Stream C flooding | **High** | Medium | `MAX_BLOCK_NUMBER_GAP`, queue limit | ⚠️ Partial |
| Block number exhaustion | Medium | Low | `reset_to()`, stale cleanup | ✅ Adequate |
| Selective withholding | Medium | Low | GhostDAG tolerance | ⚠️ Partial |
| Parent hash manipulation | Low | Low | GhostDAG tips selection | ✅ Adequate |
| Difficulty manipulation | Low | Very Low | Independent per-stream | ✅ Adequate |
| Single-stream failure | Low | Medium | Stream-agnostic consensus | ✅ Adequate |
| Block allocator deadlock | **High** | Very Low | `reset_to()` + `MAX_BLOCK_NUMBER_GAP` | ✅ Fixed |
| Cross-stream MEV | Medium | Medium | `in_flight_txs` | ⚠️ Partial |
| Fee market imbalance | Low | Medium | Per-stream pool limits | ⚠️ Partial |
| Priority inversion | Medium | High | None | ❌ Not addressed |

**Severity Definitions:**
- **High:** Can halt consensus or cause economic loss
- **Medium:** Can degrade performance or enable manipulation
- **Low:** Minor impact or requires significant resources to exploit

**Status Definitions:**
- ✅ **Adequate:** Mitigation exists and is effective
- ⚠️ **Partial:** Mitigation exists but has gaps
- ❌ **Not addressed:** No mitigation implemented

---

## 9. Conclusion

The BraidCore architecture presents novel attack surfaces due to its multi-stream design. The primary risks are:

1. **Stream C flooding** — The lack of PoW and fast block time enables low-cost block production
2. **Priority inversion** — Stream C blocks compete equally with higher-value Stream A blocks
3. **Cross-stream MEV** — Different block times create ordering arbitrage opportunities

The existing mitigations (block number gap limits, stale cleanup, GhostDAG ordering) provide a solid foundation. However, additional stream-aware rate limiting and priority mechanisms would strengthen the system before mainnet deployment.

**Critical Action Items:**
1. Implement per-stream rate limiting for block submissions
2. Add priority to `BLOCK_PROCESSING_LOCK` based on stream value
3. Enforce ZK proofs for Stream C when `privacy` feature is enabled
4. Consider unified fee market to prevent fragmentation

---

## Appendix A: Code Location Index

| Component | File | Key Lines |
|-----------|------|-----------|
| BraidCore definitions | `mining.rs` | 1-11, 43-78 |
| BlockNumberAllocator | `mining.rs` | 255-368 |
| BLOCK_PROCESSING_LOCK | `blockchain/mod.rs` | 51-55, 614 |
| GhostDAG consensus | `consensus/mod.rs` | 26-42, 147-206, 268-306 |
| ParentHashCoordinator | `mining.rs` | 370-416 |
| Stream A mining | `mining.rs` | 1676-1971 |
| Stream B mining | `mining.rs` | 1975-2244 |
| Stream C mining | `mining.rs` | 2247-2512 |
| process_blocks | `mining.rs` | 2519-2823 |
| Difficulty adjustment | `pow.rs` | 22-38, 331-392 |
| FairnessAnalyzer | `mining/fairness.rs` | 11-146 |
| Transaction pool limits | `mining.rs` | 68-88, 82-100 |
