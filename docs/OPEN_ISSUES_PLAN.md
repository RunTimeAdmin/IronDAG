# Open Issues — Step-by-Step Plan

This plan orders **all remaining open issues** from easy wins to harder work. Each step includes effort (hours), area, and reference to the analysis doc. Already-done items are not listed.

---

## Phase 1: Easy wins (≈4–8 hours total) — **DONE**

*Low risk, quick changes. Do these first.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **1.1** ✅ | Remove debug print that leaks tx data | **15 min** | RPC | Done — Removed all WAR ROOM / sensitive `println!` in `eth_send_raw_transaction`. |
| **1.2** ✅ | API key only in header (not params) | **30 min** | RPC | Done — `verify_api_key` accepts only `X-API-Key` header. |
| **1.3** ✅ | Constant-time API key comparison | **30 min** | RPC | Done — Added `constant_time_eq()` and use it for API key check. |
| **1.4** ✅ | Response cache size limit | **1–2 h** | RPC | Done — `RESPONSE_CACHE_MAX_SIZE = 1000`; evict one entry when full before insert. |
| **1.5** ✅ | EVM gas limit upper bound | **1 h** | EVM | Done — `MAX_GAS_LIMIT = 30_000_000`; `env.gas_limit = tx.gas_limit.min(MAX_GAS_LIMIT)`. |
| **1.6** ✅ | Shard cache: evict one instead of clear | **2–3 h** | Sharding | Done — When at capacity, remove one entry (first key) then insert; no more `cache.clear()`. |

**Phase 1 outcome:** No sensitive data in logs, safer API key handling, bounded RPC cache, basic DoS mitigation, no more full shard cache wipe.

---

## Phase 2: Security and correctness (≈12–20 hours) — **DONE**

*Critical for testnet/mainnet safety.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **2.1** ✅ | Reject unsigned `eth_sendTransaction` in production | **1–2 h** | RPC | Done — `allow_unsigned_eth_send` (default false) in NodeConfig + RpcServer; `--allow-unsigned-eth-send` CLI; unsigned tx rejected with message to use `eth_sendRawTransaction`. |
| **2.2** ✅ | Require API key when configured | **30 min** | RPC | Done — Auth already required when `api_key` is Some; error message updated to "X-API-Key header" only; doc that None = open access. |
| **2.3** ✅ | Per-IP rate limiting | **3–4 h** | RPC | Done — `PerIpRateLimiter` in `rate_limit.rs`; `per_ip_rate_limiter` on RpcServer; when client IP present, rate check is per-IP. |
| **2.4** ✅ | Block-based cache invalidation | **2–3 h** | RPC | Done — Response cache entries store block number; `irondag_getDagStats` uses `blockchain_cached_block_number` so cache misses when block advances. |
| **2.5** ✅ | Storage file locking on open | **2–3 h** | Storage | Done — `Database::open` creates/locks `db.lock` via fs2; if lock held by another process, returns error. |
| **2.6** ✅ | Enforce batch gas limit | **2–3 h** | Account abstraction | Done — In `execute_batch`, after each op `total_gas` checked against `batch.gas_limit`; on exceed, batch marked failed and returns. |

**Phase 2 outcome:** No unsigned spend in production, explicit auth policy, per-IP rate limits, fresher RPC data, single-process DB guarantee, batch gas enforced.

---

## Phase 3: High-impact resilience (≈20–35 hours) — **DONE**

*Network and cross-shard reliability.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **3.1** ✅ | Public IP discovery (STUN or config) | **2–3 h** | Peering | Done — `network/stun.rs`: STUN binding request to stun.l.google.com:19302, parse XOR-MAPPED-ADDRESS; NodeConfig `try_stun_discovery`; when set and no `--advertise`, discover public IP and set as handshake addr. CLI `--try-stun`. |
| **3.2** ✅ | Cross-shard retry logic | **4–6 h** | Sharding | Done — Retry queue in ShardManager; on send_receipt failure push to queue; `start_cross_shard_retry_worker()` with exponential backoff (1s, 2s, 4s… cap 60s); after 5 retries mark cross-shard tx Failed. Node starts worker when sharding enabled. |
| **3.3** ✅ | Batch rollback on failure | **8–10 h** | Account abstraction | Done — `execute_batch` now takes `revert_fn(usize, &BatchOperationResult)`; on failure or gas exceeded calls revert in reverse order so caller can undo balance/nonce; batch marked Failed. |
| **3.4** ✅ | gRPC auth and IP for rate limit | **2–3 h** | RPC | Done — `auth_from_request()` in grpc.rs extracts `x-api-key`/`api-key` and `request.remote_addr()`; all gRPC methods pass API key and client IP to `handle_request` for same auth and per-IP rate limit as JSON-RPC. |
| **3.5** ✅ | Async messaging persistence (WAL) | **5–7 h** | Sharding | Done — `MessageProcessor::with_wal(path)`; append length+bincode(msg) on send_receipt; `replay_wal()` at startup. ShardConfig `cross_shard_wal_path`; ShardManager uses WAL when set and replays on creation. |

**Phase 3 outcome:** Better NAT support, recoverable cross-shard sends, atomic batches, gRPC aligned with JSON-RPC security, durable cross-shard messages.

---

## Phase 4: Scalability and consensus (≈25–45 hours)

*DAG and sharding scale.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **4.1** ✅ | Incremental DAG updates | **8–12 h** | Consensus | **Done** — `irondag-blockchain/src/consensus/mod.rs`: On `add_block`, if `blue_set` is non-empty we call `update_blue_set_incremental(new_block_hash)` instead of full `update_blue_set()`. Affected set = new block + all descendants (BFS via `get_children`); remove only affected from blue_set/blue_score/red_set; topologically sort affected set; recompute blue/red and scores for affected only; rebuild ordering from full blue set. First block (genesis) still does full update. Fix: new block’s children list set to `vec![]` (was incorrectly set to parent hashes). |
| **4.2** ✅ | DAG hot cache LRU (or ordered eviction) | **2–4 h** | Consensus | **Done** — `irondag-blockchain/src/consensus/storage.rs`: Replaced unordered HashMap eviction with insertion-order eviction. Added `hot_blocks_order: VecDeque<Hash>`; on `add_block` push hash to back only when new; `prune_hot_cache` evicts from front (oldest inserted) until at target size, removing from hot_blocks, hot_children, hot_blue_set, hot_blue_scores, and marking finalized. Deterministic eviction, better cache behavior than random. |
| **4.3** ✅ | Peer quality / scoring | **4–5 h** | Peering | **Done** — `irondag-blockchain/src/network.rs`: Added `PeerScore` (last_seen, success_count, failure_count) and `peer_scores: Arc<RwLock<HashMap<SocketAddr, PeerScore>>>`. `evict_lowest_scoring_peer()` evicts by lowest eviction_score (success − 2×failure), tie-break oldest last_seen. When at max_peers in `connect_peer` or on incoming accept, evict one then add new peer. `record_send_success` / `record_send_failure` from broadcast_block and broadcast_transaction; handle_peer updates last_seen/success on each processed message. On disconnect, remove from peers, connections, peer_scores. |
| **4.4** ✅ | Shard synchronization (real impl) | **8–10 h** | Sharding | **Done** — `irondag-blockchain/src/sharding/mod.rs`: `synchronize_shards()` gathers from each shard via `blockchain.get_all_accounts()`, merges by home shard (only keep (addr, state) when `get_shard_for_address(&addr) == shard_id`), stores result in `unified_state`. Added `get_unified_balance`, `get_unified_nonce`, `get_unified_state`. Cross-shard tx statuses remain updated by existing retry/send path. |
| **4.5** ✅ | Checkpointing / pruning (BlockDAG) | **6–8 h** | Consensus | **Done** — `DagStorageConfig::confirmations_for_checkpoint` (0 = off). After each `add_block`, if > 0 and `ordering.len() > confirmations`, `prune_below_checkpoint(n)` runs: blocks at indices [n..] (≥ n confirmations) are removed from blue_set/blue_score/red_set/ordering and from storage via `prune_blocks_by_hash` (hot cache + finalized_blocks). Bounds in-memory DAG to the last N blocks. |
| **4.6** ✅ | Finality mechanism | **4–5 h** | Consensus | **Done** — GhostDAG: `get_finalized_block_hash()` and `get_finalized_block_number()` using ordering (block at index `finality_depth` has that many confirmations; depth = confirmations_for_checkpoint or 1). Blockchain exposes same. RPC: `eth_getBlockByNumber` and `eth_getBlockTransactionCountByNumber` accept `"finalized"` and `"safe"` and resolve to the finalized block; fallback to latest if none. |

**Phase 4 outcome:** DAG scales to 100K+ blocks, better cache behavior, smarter peer selection, real shard sync, bounded growth and clear finality.

---

## Phase 5: Harder / long-term (≈40–80 hours)

*Atomic cross-shard, privacy, rebalancing.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **5.1** ✅ | Atomic cross-shard transactions (rollback on failure) | **8–10 h** | Sharding | **Done** — When cross-shard tx is marked Failed after max retries in the retry worker, we **refund** the source: add (value + fee) back to `tx.from` on the source shard. Ensures either both deduct and credit happen, or neither (no lost funds). Full two-phase commit (prepare then commit) deferred; current flow remains deduct → send receipt → on permanent send failure, refund. |
| **5.2** ✅ | Cross-shard timeouts | **4–5 h** | Sharding | **Done** (Feb 2026) — `sharding/mod.rs`: `cross_shard_timeout_secs` config; in `start_cross_shard_retry_worker`, pending txs exceeding timeout are marked Failed and refunded to source. Default 300s. |
| **5.3** | Social recovery time lock | **3–4 h** | Account abstraction | [SYSTEM_REVIEW](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md) 4.3 — Recovery takes effect after a delay (e.g. 24–48 h); allow cancellation by owner; optional notification. |
| **5.4** | Multisig constant-time verification | **2–3 h** | Account abstraction | [SYSTEM_REVIEW](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md) 4.2 — Verify all signatures before returning; avoid early exit; use constant-time comparison where applicable. |
| **5.5** | Storage backup / snapshot | **4–6 h** | Storage | [SYSTEM_REVIEW](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md) 2.1 — Add `backup(path)` and optionally `snapshot()`; document restore procedure. |
| **5.6** | DSI gas measurement / caps | **6–8 h** | EVM | [SYSTEM_REVIEW](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md) 3.1 — Where possible, measure or bound actual work in DSI handlers and charge gas accordingly; at minimum, add conservative upper bounds per handler. |
| **5.7** | Privacy: trusted setup + circuit | **40+ h** | Privacy | [SYSTEM_REVIEW](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md) 5.1–5.3 — Replace random key gen with a single shared trusted setup (or MPC ceremony); use proper Pedersen generators; implement full circuit constraints (nullifier, commitment, range). |

**Phase 5 outcome:** Cross-shard atomicity and timeouts, safer social recovery and multisig, backup capability, fairer DSI gas, production-grade privacy (long-term).

---

## Phase 7: Verkle Integration — Stream C State Authentication

*Integrate Verkle tree for state authentication in Stream C mining and ZK state transition circuits.*

| Step | Issue | Effort | Location | Action |
|------|--------|--------|----------|--------|
| **7.1** | State Root Wiring | **1 day** | Mining, ZK | Replace placeholder state hashes (keccak256(b"pre"/"post")) in mine_stream_c() with actual VerkleState::root_hash(). Compute post-state root as commitment to (pre_state_root + transactions_root). Graceful fallback when Verkle state not initialized. |
| **7.2** | Balance Witness Authentication | **2–3 days** | Mining, ZK, Verkle | Replace Fr::ONE placeholder balances with actual VerkleState::get_balance() lookups. Generate Verkle proofs for each balance lookup as private witnesses. Verify balance proofs against pre-state root in circuit. |
| **7.3** | In-Circuit Verkle Proof Verification | **1+ week** | ZK, Verkle | Encode Verkle tree path verification as R1CS arithmetic constraints. Add Verkle proof gadget to StateTransitionCircuit. Verify each balance proof against state root inside the ZK circuit. |
| **7.4** | KZG Commitment Upgrade | **1+ week** | Verkle, Light client | Replace placeholder verify_proof() in verkle/proof.rs with actual KZG polynomial commitment verification. Use ark-ec / ark-poly-commit stack (already in dependency tree). Reduce proof size from Merkle-path (~1KB) to KZG (~100 bytes). Enable stateless client verification. |

**Phase 7 outcome:** Stream C mining uses real Verkle state roots, balance lookups are authenticated with Verkle proofs, ZK circuit verifies Verkle proofs in-circuit, and KZG commitments enable efficient stateless client verification.

**Dependencies:** 7.1 → 7.2 → 7.3 sequential; 7.4 can run parallel with 7.3.

---

## KZG Trusted Setup

### Current State (Development)
The KZG module (`src/verkle/kzg.rs`) generates a Structured Reference String (SRS) from a deterministic seed using `KzgSrs::generate_deterministic()`. This is suitable for testing and development only. The seed is known, meaning the "toxic waste" (secret tau) is computable by anyone, which would allow proof forgery in production.

### Production Options

**Option A: Ethereum KZG Ceremony Re-use (Recommended)**
- The Ethereum EIP-4844 (Proto-Danksharding) KZG ceremony completed in 2023 with 141,416 participants
- The resulting SRS supports degree-4096 polynomials (sufficient for our 256-way Verkle tree)
- Re-using this SRS inherits the security guarantee: as long as at least 1 of 141,416 participants was honest, the toxic waste is unknown
- Implementation: Download the ceremony output, deserialize the G1/G2 points, use as our SRS
- Advantage: No ceremony logistics, battle-tested, widely trusted
- Constraint: Locked to BN254 curve (which we already use)

**Option B: IronDAG-Specific Ceremony**
- Run a new powers-of-tau ceremony specific to this project
- Allows custom degree bounds if needed
- Requires: ceremony coordinator software, participant recruitment, verification
- Timeline: 2-4 weeks for setup, 1-2 weeks for participation window
- Risk: Fewer participants means weaker security guarantee

**Option C: Universal Setup (Marlin/PLONK-style)**
- Use a universal SRS that supports any circuit up to a maximum size
- More flexible but larger SRS size
- Available via the Aztec/Zcash ceremony outputs

### Recommended Path
1. **Immediate**: Continue using deterministic SRS for testnet (clearly marked as dev-only)
2. **Pre-mainnet**: Integrate Ethereum KZG ceremony SRS (Option A)
3. **Post-mainnet**: Evaluate need for IronDAG-specific ceremony based on degree requirements

### Implementation Steps for Option A
1. Download Ethereum KZG ceremony output (trusted_setup.json, ~100MB)
2. Parse G1 and G2 points from the ceremony format
3. Validate the SRS against known checksums
4. Replace `generate_deterministic()` with `load_ceremony_srs()` in production config
5. Add config option: `--kzg-srs-path` for custom SRS file location
6. Keep `generate_deterministic()` available for `--dev` mode only

---

## Summary table (open items only)

| Phase | Focus | Steps | Est. hours |
|-------|--------|-------|------------|
| **1** | Easy wins | 1.1–1.6 | 4–8 |
| **2** | Security & correctness | 2.1–2.6 | 12–20 |
| **3** | Resilience | 3.1–3.5 | 20–35 |
| **4** | Scalability & consensus | 4.1–4.6 | 25–45 |
| **5** | Harder / long-term | 5.1–5.7 | 40–80 |
| **Total** | | | **~100–190** |

---

## Suggested order for the next few sessions

1. ~~**Phase 1** — 1.1–1.6~~ **Completed.**  
2. ~~**Phase 2** — 2.1–2.6~~ **Completed.**  
3. ~~**Phase 3** — 3.1–3.5~~ **Completed.**  
4. **Phase 4** — 4.1–4.6 ✅ **Complete.**  
5. **Phase 5** — 5.1 rollback/refund ✅; 5.2 cross-shard timeouts ✅ (Feb 2026); next: 5.3–5.7.

After that, proceed in phase order or pick by impact (e.g. 3.2 cross-shard retry, 3.3 batch rollback) as needed.

---

*Sources: [PEERING_ANALYSIS.md](PEERING_ANALYSIS.md), [CRITICAL_FIXES_PRIORITIZATION.md](CRITICAL_FIXES_PRIORITIZATION.md), [BLOCKDAG_SHARDING_ANALYSIS.md](BLOCKDAG_SHARDING_ANALYSIS.md), [SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md](SYSTEM_REVIEW_RPC_STORAGE_EVM_AA_PRIVACY.md), [CPU_OPTIMIZATION.md](CPU_OPTIMIZATION.md).*
