# IronDAG Blockchain - Testing Plan

**Focus:** Feature verification (5) and integration testing (6)  
**Last updated:** 2026-02-16

---

## 5. Feature Verification (20–40 hours)

### 5.1 Authentication (API key hashing, constant-time comparison)

| Test | Method | Expected | Status |
|------|--------|----------|--------|
| No API key when auth required | `handle_request(..., None, ...)` for net_peerCount | Error: "Unauthorized" | Run `cargo test rpc_auth_rate_limit` |
| Wrong API key | `handle_request(..., Some("wrong"), ...)` | Error: "Unauthorized" | Run `cargo test rpc_auth_rate_limit` |
| Correct API key | `handle_request(..., Some(key), ...)` | Success | Run `cargo test rpc_auth_rate_limit` |
| Key only in header (not params) | N/A | Verified in code: `verify_api_key` uses header only | — |
| Constant-time comparison | N/A | Verified in code: `constant_time_eq` used | — |

**Note:** Node does not expose `--api-key` via CLI. Auth is testable via unit tests using `RpcServer::with_auth()`. For live-node testing, add `--api-key <key>` CLI flag.

---

### 5.2 Rate Limiting (PerIpRateLimiter)

| Test | Method | Expected | Status |
|------|--------|----------|--------|
| Per-IP token bucket | Burst requests from same IP | First N succeed, then rate limited | Run `cargo test rpc_auth_rate_limit` |
| Different IPs | Requests from different IPs | Each IP has own bucket | Unit test |
| Rate limit error code | On exceed | `-32005` (RPC_RATE_LIMITED) | Unit test |

**Note:** Per-IP limiter requires `set_per_ip_rate_limiter()` on RpcServer. Node does not enable it by default. Config has `rpc_rate_limit` but it may not wire to PerIpRateLimiter.

---

### 5.3 Unsigned Transaction Rejection

| Test | Method | Expected | Status |
|------|--------|----------|--------|
| eth_sendTransaction unsigned | Send unsigned tx when `allow_unsigned=false` | Reject with message to use eth_sendRawTransaction | Manual or RPC test |
| eth_sendRawTransaction signed | Send signed raw tx | Accept | Manual |

**Default:** `allow_unsigned_eth_send = false`. Use `--allow-unsigned-eth-send` for dev only.

---

### 5.4 gRPC Auth

| Test | Method | Expected | Status |
|------|--------|----------|--------|
| gRPC with x-api-key | gRPC call with metadata | Same auth as JSON-RPC | Requires grpcurl or client |
| gRPC without key when required | gRPC call without metadata | Unauthorized | — |

---

### 5.5 Cross-Shard (WAL, Retry, Refund)

| Test | Method | Expected | Status |
|------|--------|----------|--------|
| WAL persistence | Enable sharding with `cross_shard_wal_path`, send cross-shard tx, restart | Replay WAL on startup | Run `cargo test` sharding tests |
| Retry queue | Simulate target shard failure | Retries with backoff, eventually refund | Sharding integration test |
| Atomic refund | Max retries exceeded | Source shard refunded | Sharding test |
| Timeout + refund | Cross-shard tx pending > timeout | Mark Failed, refund source | Sharding test |

**Tests:** `sharding_integration.rs`, `sharding_e2e.rs`, `sharding_basic_test.rs`

---

## 6. Integration Testing (40–60 hours)

### 6.1 Multi-Node Sync

| Test | Steps | Expected | Status |
|------|-------|----------|--------|
| 2-node sync | Node 1 miner (8080/8545), Node 2 sync (8082/8546), peer 127.0.0.1:8080 | Both sync, block heights within 1–5 | Run `scripts/integration_two_nodes.ps1` |
| 3-node mesh | 3 nodes, full mesh | All sync | Manual or script |
| Peer count | After sync | net_peerCount ≥ 1 on each | — |
| Block propagation | Mine on Node 1 | Node 2 sees new blocks within seconds | — |

**Port note:** Node 1 uses sync server on P2P+1 (8081). Use 8082 for Node 2 P2P to avoid conflict.

---

### 6.2 Network Partition

| Test | Steps | Expected | Status |
|------|-------|----------|--------|
| Split and rejoin | Run 3 nodes, disconnect Node 2 from Node 1, mine on both sides, reconnect | Eventually reconcile (resync) | Manual |
| Fork detection | Create fork, reconnect | Clear and resync from longer chain | — |

---

### 6.3 Cross-Shard Integration (if sharding enabled)

| Test | Steps | Expected | Status |
|------|-------|----------|--------|
| Cross-shard tx flow | Enable shards, send tx from shard A to shard B | Deduct on A, credit on B | Run sharding tests |
| Receipt processing | start_receipt_processing called | Receipts processed | Requires Phase 6 wiring |

---

### 6.4 Load / Stress

| Test | Steps | Expected | Status |
|------|-------|----------|--------|
| TX throughput | Send many txs, mine | No stalls, blocks include txs | — |
| CPU stability | Run 1 hour | CPU ~120–150%, no growth | — |
| Memory stability | Run 24 hours | No leak | — |

---

## Test Execution

### Run Unit / Integration Tests

```bash
cd irondag-blockchain
cargo test
cargo test rpc_auth_rate_limit
cargo test sharding
```

### Run 2-Node Integration

```powershell
# From repo root
.\scripts\integration_two_nodes.ps1
```

### Manual RPC Tests (node running on 127.0.0.1:8545)

```bash
# Block number (no auth needed by default)
curl -X POST http://127.0.0.1:8545 -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# With API key (when node has --api-key)
curl -X POST http://127.0.0.1:8545 -H "Content-Type: application/json" -H "X-API-Key: your-key" -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

---

## Gaps & Recommendations

1. **CLI for auth/rate limit:** Add `--api-key <key>` and `--rpc-rate-limit <n>` to enable live-node auth testing.
2. **Phase 6 sharding:** Call `start_receipt_processing` at startup when sharding enabled; route txs to shard_manager.
3. **Automated load test:** Add `tests/load_stress.rs` for sustained TX load.
4. **Network partition test:** Add script or test for partition + rejoin scenario.

---

## Files

| File | Purpose |
|------|---------|
| `irondag-blockchain/tests/rpc_auth_rate_limit.rs` | Auth and rate-limit unit tests |
| `scripts/integration_two_nodes.ps1` | 2-node sync test (fixed ports) |
| `docs/TESTING_PLAN.md` | This plan |
