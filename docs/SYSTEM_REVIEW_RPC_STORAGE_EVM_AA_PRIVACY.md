# IronDAG Blockchain — Comprehensive System Review

**Review Date:** 2024  
**Scope:** RPC layer, storage, EVM, account abstraction, privacy/ZK  
**Total code analyzed:** ~12,000 lines across 5 major systems  
**Issues identified:** 27 (critical, high, medium, low)

---

## Summary by system

| System            | Lines | Critical | Severity   |
|-------------------|-------|----------|------------|
| RPC layer         | 8,245 | 10       | CRITICAL   |
| Storage           | 1,364 | 4        | MEDIUM     |
| EVM               | 2,090 | 5        | HIGH       |
| Account abstraction | 2,984 | 5     | HIGH       |
| Privacy/ZK        | 1,520 | 3        | CRITICAL   |
| **Total**         | 16,203| **27**   | —          |

---

## 1. RPC layer — CRITICAL

### Critical

- **1.1 Authentication bypass:** No API key configured → all methods open; `verify_api_key` returns `true` when `api_key` is `None`. *Location: rpc.rs ~642–672.*
- **1.2 Unsigned transactions accepted:** `eth_sendTransaction` builds and submits tx without signature; comment: "For now, we'll accept unsigned transactions for testing". Anyone can spend any address. *Location: rpc.rs ~1153–1240.*
- **1.3 API key in params:** API key accepted in JSON-RPC params → risk of leaking via logs, referrer, proxies. *Same block as 1.1.*

### High

- **1.4 Rate limiter global:** Single global token bucket, not per-IP → one client can exhaust limit for everyone. *rpc/rate_limit.rs.*
- **1.5 Response cache never invalidated:** TTL-only (e.g. 100 ms), no block-based invalidation → stale chain/balance data. *rpc.rs ~279, 283, 1750–1774.*

### Medium

- **1.6 Debug print leaks tx data:** `println!` of raw params in `eth_sendRawTransaction`. *rpc.rs ~1371.*
- **1.7 gRPC no auth:** gRPC handlers call JSON-RPC with `None` for API key and no client IP. *rpc/grpc.rs ~28–100.*
- **1.8 Lock timeout:** Fixed 10 s blockchain read timeout; can fail under load with no retry/queue. *rpc.rs ~32, 610–628.*

### Low

- **1.9 Response cache unbounded:** `HashMap` cache with no size/eviction → possible memory growth. *rpc.rs ~279, 340.*
- **1.10 Public methods hardcoded:** Public method set not configurable; e.g. `irondag_faucet` is public. *rpc.rs ~304–311, 410–417.*

**Recommendations (critical):** Require API key when configured; remove support for unsigned `eth_sendTransaction` (or restrict to testnets); accept API key only in header; use constant-time comparison; avoid storing keys in plaintext; consider hashing/rotation.

---

## 2. Storage — MEDIUM

- **2.1 No backup/snapshot:** No backup, snapshot, restore, or checkpointing → full data loss on disk failure. *storage.rs.*
- **2.2 No file locking:** `sled::open(path)` with no lock → multiple processes can open same DB and corrupt it. *storage.rs ~274–281.*
- **2.3 Compression level fixed:** `COMPRESSION_LEVEL = 3` not configurable. *storage.rs ~24–27.*
- **2.4 Batch vs in-memory state:** Batch commits to storage only; in-memory blockchain state can diverge; no rollback on partial failure. *storage.rs ~206–272.*

---

## 3. EVM — HIGH

- **3.1 DSI bypasses gas metering:** DSI handlers return fixed gas (e.g. `SET_VALUE`) instead of measuring; expensive ops can be undercharged. *evm/mod.rs ~152–300.*
- **3.2 Storage changes not persisted:** revm limitation; storage maps empty in result; only DSI-handled storage reliably persisted. *evm/mod.rs ~676–740.*
- **3.3 Gas price from fee/gas_limit:** `env.gas_price = tx.fee / tx.gas_limit` can misrepresent gas price and diverge from Ethereum semantics. *evm/mod.rs ~656–658.*
- **3.4 No gas limit validation:** `env.gas_limit = tx.gas_limit` with no upper bound → OOM/DoS risk. *evm/mod.rs ~656.*
- **3.5 DSI handler registry hardcoded:** No runtime configuration or custom handlers. *evm/mod.rs ~152–300.*

---

## 4. Account abstraction — HIGH

- **4.1 Batch rollback not implemented:** On failure, earlier operations in the batch remain applied; no atomic all-or-nothing. *account_abstraction/batch.rs.*
- **4.2 Multisig verification timing:** Signature verification flow can leak information via early returns; constant-time comparison recommended. *account_abstraction/multisig.rs.*
- **4.3 Social recovery no time lock:** Recovery executes immediately; no delay, notification, or cancellation window. *account_abstraction/social_recovery.rs.*
- **4.4 Batch gas limit not enforced:** `gas_used` accumulated but not checked against `gas_limit` during execution. *account_abstraction/batch.rs.*
- **4.5 Batch ID collision possible:** ID from wallet + nonce + operations; no timestamp; theoretical collision. *account_abstraction/batch.rs ~100–150.*

---

## 5. Privacy / ZK — CRITICAL

- **5.1 Trusted setup replaced by random keys:** `generate_keys` uses `generate_random_parameters_with_reduction` per node; comment: "In production, use a trusted setup ceremony". Different nodes have different keys → proofs from one node do not verify on another; privacy system unusable. *privacy/keys.rs ~20–50.*
- **5.2 Pedersen with deterministic generators:** Generators from `hash_to_g1(b"pedersen_g")` etc.; comment: "For now, use deterministic generators". Same generators everywhere → commitments can be brute-forced or linked. *privacy/commitment.rs ~125–160.*
- **5.3 Circuit simplified:** Nullifier and commitment not properly constrained; comments "In production, use proper …". Double-spend and fake transfers possible; circuit does not enforce intended invariants. *privacy/circuit.rs.*

---

## Recommendations (prioritized)

**Critical (before any production use):**

- **Authentication (RPC):** Mandatory API key when configured; no API key in params (header only); constant-time comparison; no unsigned `eth_sendTransaction` in production.
- **Privacy:** Real trusted setup for zk-SNARKs; proper generators for Pedersen; full circuit constraints (nullifier, commitment, range proofs).
- **Rate limiting:** Per-IP (or per-client) limits and abuse handling.

**High (before mainnet):**

- Batch execution: rollback on failure, atomicity.
- Gas: actual measurement for DSI; enforce gas limits (EVM and batch).
- Social recovery: time lock, notification, cancellation.
- EVM: gas limit validation; fix or document storage persistence vs revm.

**Medium:**

- Storage: backup/snapshot, file locking.
- Caches: block-based invalidation, size limits.
- gRPC: align with JSON-RPC auth and rate limiting.

**Low:**

- Remove debug prints and canary logs; configurable public methods; adaptive timeouts/retries.

---

## Effort and risk (from review)

- **Critical:** 40–60 h — production deployment not possible until addressed.
- **High:** 30–50 h — required for mainnet.
- **Medium:** 20–30 h — stability and operability.
- **Total:** ~90–140 h (3–4 weeks for an experienced team).

**Risk:** Current state is not suitable for production. After critical fixes, testnet may be viable; after high-priority fixes, mainnet may be viable; full remediation supports enterprise-grade deployment.

---

*Document version: 1.0. See source files for exact locations and current code. Paths above are indicative (e.g. `rpc.rs`, `storage.rs`, `evm/mod.rs`, `account_abstraction/`, `privacy/`).*
