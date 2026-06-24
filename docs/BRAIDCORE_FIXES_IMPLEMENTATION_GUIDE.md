# BraidCore Mining Fixes — Implementation Guide

This guide documents the implementation of fixes for the BraidCore mining issues identified in the deep-dive analysis. The fixes address block hanging and stream competition by adding coordination and reservation-based block numbering.

---

## Overview

**Root causes addressed:**

1. **Block number gaps** — Failed blocks no longer consume permanent block numbers; reservations are released on validation failure.
2. **No stream competition rules** — `StreamPriority` defines ordering (A > B > C); can be extended for acceptance rules.
3. **Parent hash race conditions** — `ParentHashCoordinator` serializes parent selection and validates before use.

---

## Summary of Changes

### 1. New structures (`irondag-blockchain/src/mining.rs`)

- **`StreamPriority`** — Enum (StreamA=3, StreamB=2, StreamC=1); `from_stream_type()` for mapping. Used for logging and future “best block wins” logic.
- **`BlockNumberAllocator`** — Reserve block number before validation; `confirm()` on success, `release()` on failure. `set_next_available()` to sync from chain at startup.
- **`ParentHashCoordinator`** — Mutex-guarded `select_parents(blockchain, stream_type)` and `are_parents_valid(blockchain, parents)` to reduce races between streams.

### 2. Modified structures

- **`MiningManager`** — Replaced `block_counter: Arc<AtomicU64>` with `block_allocator: Arc<BlockNumberAllocator>` and `parent_coordinator: Arc<ParentHashCoordinator>`.
- **`process_blocks()`** — Takes `block_allocator` and `parent_coordinator`; syncs allocator from chain height; uses reserve → add_block → confirm or release; no more `fetch_add`-only numbering.

### 3. Constructors updated

- **`new()`**, **`with_node_registry()`**, **`with_sharding()`** — Each creates `BlockNumberAllocator::new(0)` and `ParentHashCoordinator::new()`, passes them into the spawned `process_blocks` task, and stores them in `Self`.
- **`clone_for_mining()`** — Clones `block_allocator` and `parent_coordinator` instead of `block_counter`.

### 4. Stream mining (A, B, C)

- Parent selection replaced with:
  - `parent_hashes = self.parent_coordinator.select_parents(&self.blockchain, stream_type).await`
  - If non-empty: `are_parents_valid(...)`; if invalid, short sleep and `continue`.
- So all three streams use coordinated, validated parent hashes.

### 5. `start_mining_streams()`

- Replaced block-counter sync with allocator sync: `get_stats()` and, if allocator is behind chain height, `block_allocator.set_next_available(current_height)`.

---

## BlockNumberAllocator API

```rust
// Reserve (before validation)
let num = block_allocator.reserve(stream_type).await;

// On success
block_allocator.confirm(block_number).await;

// On validation failure
block_allocator.release(block_number, stream_type).await;

// At startup (in process_blocks)
block_allocator.set_next_available(max_block_num + 1);
```

---

## ParentHashCoordinator API

```rust
let parent_hashes = parent_coordinator
    .select_parents(&blockchain, stream_type)
    .await;

if !parent_hashes.is_empty() {
    let valid = parent_coordinator
        .are_parents_valid(&blockchain, &parent_hashes)
        .await;
    if !valid {
        continue; // retry next iteration
    }
}
```

---

## Testing

- **Single-stream** — Unchanged; one stream uses allocator and coordinator without contention.
- **Multi-stream** — Run without `--single-stream` and monitor:
  - Block numbers stay sequential (no gaps).
  - Logs show reserve/confirm/release when failures occur.
  - No persistent “stuck” state from parent or block-number issues.

Suggested manual test:

```bash
cd irondag-blockchain
cargo run --release --bin node -- --single-stream   # baseline
cargo run --release --bin node                      # tri-stream
```

---

## Expected behavior

| Before | After |
|--------|--------|
| Failed block consumes block number → gap | Failed block → `release()` → no permanent gap |
| Streams read parents independently → races | Coordinator serializes parent selection |
| No formal priority | StreamPriority (A > B > C) available for future use |

---

## Remaining / optional (from original guide)

- **Stream-specific transaction pools** — Phase 3; reduces tx contention between streams.
- **GhostDAG-based acceptance** — Use blue scores to accept/reject blocks in multi-stream mode.
- **Dedicated test file** — e.g. `tests/braidcore_fixes.rs` for allocator and coordinator unit tests.

---

## Files touched

- **irondag-blockchain/src/mining.rs** — All new types, `MiningManager` changes, `process_blocks` reserve/confirm/release, parent coordinator usage in `mine_stream_a/b/c`.

---

*Implementation completed per BraidCore Mining Fixes guide. See `docs/BRAIDCORE_DEEP_DIVE.md` for the problem analysis.*
