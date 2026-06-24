# Implementation Verification — Codebase Evidence

**Date:** 2026-02-16  
**Branch:** master  
**Purpose:** Refute the claim that "NONE of 21 claimed implementations were found." All items below exist in the current codebase.

---

## 1. TriStream — reserve/confirm/release in process_blocks

| Item | Location | Evidence |
|------|----------|----------|
| `reserve()` | `mining.rs:93-94` | `block_allocator.reserve(stream_type).await` |
| `release()` | `mining.rs:1042` | `block_allocator.release(block_number, stream_type).await` on validation failure |
| `confirm()` | `mining.rs:1058` | `block_allocator.confirm(block_number).await` on success |
| Integration | `mining.rs:1017-1058` | `process_blocks` calls reserve → (validate) → confirm or release |

---

## 2. Stream C — 1 second (not 100ms)

| Item | Location | Evidence |
|------|----------|----------|
| STREAM_C_BLOCK_TIME | `mining.rs:37` | `pub const STREAM_C_BLOCK_TIME: Duration = Duration::from_secs(1);` |
| Comment | `mining.rs:36` | "Stream C: 1s (was 100ms) to reduce lock churn" |

---

## 3. Security — API key, constant-time, unsigned tx rejection

| Item | Location | Evidence |
|------|----------|----------|
| `hash_api_key()` | `rpc.rs:44` | `fn hash_api_key(key: &str) -> [u8; 32]` |
| `constant_time_eq()` | `rpc.rs:31` | `fn constant_time_eq(a: &[u8], b: &[u8]) -> bool` |
| API key hashing usage | `rpc.rs:379,386,471,638` | `server.api_key_hash = Some(hash_api_key(&api_key))` |
| Constant-time check | `rpc.rs:691-692` | `constant_time_eq(&provided_hash[..], &stored_hash[..])` |
| `allow_unsigned_eth_send` | `rpc.rs:300,366,642` | Field + `set_allow_unsigned_eth_send()` |
| Rejection logic | `rpc.rs:1207` | `if !self.allow_unsigned_eth_send && !has_signature` |
| NodeConfig | `node/mod.rs:51,79` | `allow_unsigned_eth_send: false` (default) |
| CLI | `bin/node.rs:127` | `--allow-unsigned-eth-send` for dev |

---

## 4. BlockDAG — incremental updates, finality

| Item | Location | Evidence |
|------|----------|----------|
| `update_blue_set_incremental()` | `consensus/mod.rs:235` | Full implementation |
| Called from add_block | `consensus/mod.rs:98` | `self.update_blue_set_incremental(hash)?` |
| `get_finalized_block_hash()` | `consensus/mod.rs:377` | GhostDAG method |
| `get_finalized_block_number()` | `consensus/mod.rs:386-387` | Via finalized hash |
| Blockchain exposure | `blockchain/mod.rs:1695-1701` | `get_finalized_block_number/hash` |
| RPC usage | `rpc.rs:1104,1735` | Accepts "finalized"/"safe" block params |

---

## 5. Sharding — retry queue, WAL, atomic refund

| Item | Location | Evidence |
|------|----------|----------|
| `retry_queue` | `sharding/mod.rs:118,213,227,276,602` | `Mutex<VecDeque<CrossShardRetryEntry>>` |
| `start_cross_shard_retry_worker()` | `sharding/mod.rs:220` | Spawns retry loop |
| `cross_shard_wal_path` | `sharding/mod.rs:47,58,185` | Config + WAL path |
| `replay_wal()` | `async_messaging.rs:191` | `pub fn replay_wal(&self)` |
| WAL replay at startup | `sharding/mod.rs:193` | `m.replay_wal()` when path set |
| Refund on max retries | `sharding/mod.rs:257-271` | `refund_from`, `set_balance` |
| Timeout + refund (Phase 5.2) | `sharding/mod.rs:285-315` | `to_timeout`, refund source |

---

## 6. Peering — STUN, peer scoring, peer exchange

| Item | Location | Evidence |
|------|----------|----------|
| STUN module | `network/stun.rs` | Full RFC 5389 client, `discover_public_addr()` |
| STUN usage | `node/mod.rs:240-241` | `stun::discover_public_addr()` when `try_stun_discovery` |
| CLI | `bin/node.rs:129-130` | `--try-stun` |
| `PeerScore` | `network.rs` | `last_seen`, `success_count`, `failure_count` |
| `peer_scores` | `network.rs:175,198,238,252,431,442,513,554` | `HashMap<SocketAddr, PeerScore>` |
| `evict_lowest_scoring_peer()` | `network.rs:254-255` | Evicts by score when at max_peers |
| Peer exchange | `network.rs` | `peer_connect_tx`, connect requests |

---

## 7. RPC & gRPC — rate limiting, gRPC auth

| Item | Location | Evidence |
|------|----------|----------|
| `PerIpRateLimiter` | `rpc/rate_limit.rs:72` | `pub struct PerIpRateLimiter` |
| `per_ip_rate_limiter` | `rpc.rs:251,341,428,508,583,647` | Field + `set_per_ip_rate_limiter()` |
| Per-IP check | `rpc.rs:822` | `(&self.per_ip_rate_limiter, client_ip)` |
| gRPC `auth_from_request()` | `rpc/grpc.rs:12` | Extracts `x-api-key`, `api-key`, `remote_addr()` |
| gRPC methods using auth | `rpc/grpc.rs:48,73,102,131,168,203` | All pass API key + IP to `handle_request` |

---

## 8. Storage — file locking

| Item | Location | Evidence |
|------|----------|----------|
| `db.lock` | `storage.rs:270,278,284` | `fs2::FileExt`, exclusive lock on open |
| Lock path | `storage.rs:284` | `path.join("db.lock")` |

---

## 9. Account abstraction — batch rollback

| Item | Location | Evidence |
|------|----------|----------|
| `revert_fn` | `account_abstraction/batch.rs:340,344` | `revert_fn: impl Fn(usize, &BatchOperationResult)` |
| Rollback on failure | `batch.rs:374,385,402` | `revert_fn(i, r)` in reverse on failure/gas exceeded |

---

## Summary

| Category | Claimed | Found | Status |
|----------|---------|-------|--------|
| TriStream reserve/confirm/release | ❌ | ✅ | **EXISTS** |
| Stream C 1s | ❌ | ✅ | **EXISTS** |
| hash_api_key, constant_time_eq | ❌ | ✅ | **EXISTS** |
| allow_unsigned_eth_send | ❌ | ✅ | **EXISTS** |
| BlockDAG incremental | ❌ | ✅ | **EXISTS** |
| BlockDAG finality | ❌ | ✅ | **EXISTS** |
| Sharding retry/WAL/refund | ❌ | ✅ | **EXISTS** |
| STUN | ❌ | ✅ | **EXISTS** |
| Peer scoring | ❌ | ✅ | **EXISTS** |
| PerIpRateLimiter | ❌ | ✅ | **EXISTS** |
| gRPC auth | ❌ | ✅ | **EXISTS** |
| Storage db.lock | ❌ | ✅ | **EXISTS** |
| Batch revert_fn | ❌ | ✅ | **EXISTS** |

**Conclusion:** All 21+ claimed implementations exist on **master**. The "NONE were found" analysis likely searched the wrong branches, used incorrect paths, or had tooling/visibility issues.
