# IronDAG Blockchain – Critical Security Patches

**Last Updated: 2026-04-10 (B3 Hardening Sprint, Testnet Verified)**

This document maps the critical security patches from the system review to the current codebase and confirms mitigations.

---

## Patch #1: RPC Authentication Bypass (CRITICAL) — **MITIGATED**

**Threat:** Anyone could submit transactions spending from any address if unsigned transactions were accepted.

**Status:** Addressed by existing RPC behavior and Phase 2 hardening.

### Current Protections

1. **`eth_sendTransaction` (Metamask-style)**  
   - **Location:** `irondag-blockchain/src/rpc.rs` (~1173–1395)  
   - **Behavior:** When `allow_unsigned_eth_send` is `false` (default), the handler **rejects** requests that do not include `r`, `s`, and `v` with a clear error directing users to sign and use `eth_sendRawTransaction`.  
   - **Config:** `NodeConfig::allow_unsigned_eth_send` (default `false`); CLI `--allow-unsigned-eth-send` for dev only.

2. **`eth_sendRawTransaction` (signed hex payload)**  
   - **Location:** `irondag-blockchain/src/rpc.rs` (~1400–1695)  
   - **Behavior:**  
     - Decodes RLP and ECDSA signature; recovers signer and sets `tx.from` from the recovered address (no client-controlled `from`).  
     - Calls `tx.verify_signature()`; on failure returns `"Invalid signature: verification failed"`.  
     - Checks balance and nonce for the **recovered** sender before adding to the mining pool.  
   - There is **no** path that executes a raw `Transaction` with a client-chosen `from` without signature verification.

3. **Other transaction entry points**  
   - `irondag_send_raw_transaction`, `irondag_create_test_transaction`, `irondag_faucet` all call `tx.verify_signature()` before accepting the transaction.

**Conclusion:** There is no standalone `execute_transaction(transaction)` that adds to a pool or executes without verification. Patch #1 is satisfied by the current design; no further code change is required for this item.

---

## Patch #2: Privacy System Trusted Setup (CRITICAL) — **ADDRESSED**

**Threat:** Each node generated its own proving/verifying keys with `generate_keys(thread_rng())`, so proofs from one node could not be verified by another.

**Status:** Addressed by optional key loading from file (trusted setup output).

### Implementation

- **PrivacyConfig** (`src/privacy/mod.rs`): Optional `proving_key_path` and `verifying_key_path`. When set, keys are **loaded from file** instead of generated.
- **NodeConfig** (`src/node/mod.rs`): Optional `privacy_proving_key_path` and `privacy_verifying_key_path`. When both are set and privacy is enabled, keys are loaded via `load_keys_from_paths`; otherwise keys are generated locally with a **warning** that cross-node verification requires a shared trusted setup.
- **CLI:** `--privacy-proving-key <path>` and `--privacy-verifying-key <path>`.
- **Keys** (`src/privacy/keys.rs`): `load_keys_from_paths(proving_key_path, verifying_key_path)` reads both files and calls existing `load_keys_from_bytes`.
- **Production:** Run a one-time trusted setup (e.g. using `privacy/keys.rs::generate_keys` or a ceremony), serialize with `serialize_keys`, write to files, and distribute the same files to all nodes. Start the node with both `--privacy-proving-key` and `--privacy-verifying-key` (or set in config).

---

## Patch #3: Gas Metering (HIGH) — **ADDRESSED**

**Threat:** DSI (Direct Storage Intercept) handlers could report arbitrary `gas_used`, leading to undercharging or DoS.

**Status:** Addressed by capping DSI-reported gas.

### Implementation

- **Location:** `irondag-blockchain/src/evm/mod.rs` (`execute_via_dsi`).  
- **Change:** DSI `gas_used` is capped to the transaction’s `gas_limit` and to a fixed upper bound per call (`MAX_DSI_GAS_PER_CALL`), so a single DSI invocation cannot report more than the tx limit or the cap.  
- **Note:** The codebase uses selector-based DSI handlers (e.g. storage get/set), not the document’s opcode list (ZkVerify, AiInference, etc.). The mitigation is a conservative cap on reported gas rather than full dynamic metering.

---

## Patch #4: Batch Transaction Rollback (HIGH) — **ADDRESSED**

**Threat:** Partial batch execution could leave state partially applied if a later transaction failed (no atomicity).

**Status:** Addressed by existing batch API: rollback is implemented via a revert callback.

### Current Design

- **Location:** `irondag-blockchain/src/account_abstraction/batch.rs` (`execute_batch`).
- **Behavior:** `execute_batch(batch_id, execute_fn, revert_fn)` runs operations via `execute_fn`. On failure or gas exceeded it calls `revert_fn(i, result)` in **reverse order** for each completed operation, then marks the batch failed. Callers supply `revert_fn` to restore state (e.g. from snapshots or inverse operations).
- **Note:** The document’s design (TransactionExecutor, RollbackState, StateDB snapshot/restore) is not implemented in-core; atomicity is achieved by requiring the caller to implement revert in the callback. For in-process batch execution this is sufficient.

---

## Patch #5: API Key Security (CRITICAL) — **ADDRESSED**

**Threat:** API keys stored in plaintext in memory and config; keys in URL/params could be logged.

**Status:** Addressed: only a hash of the key is stored; key accepted only via header; constant-time verification.

### Implementation

- **Location:** `irondag-blockchain/src/rpc.rs`.
- **Changes:**
  - **Storage:** `api_key` replaced with `api_key_hash: Option<[u8; 32]>`. When a key is set (`with_auth`, `with_chain_id_and_auth`, `with_rate_limit_and_auth`, `with_rate_limit_sharding_and_auth`, `set_api_key`), it is hashed with `hash_api_key(key)` (Blake3) and only the hash is stored; the plain key is never retained in memory.
  - **Verification:** `verify_api_key` accepts the key only from the **X-API-Key** header (not from URL or query params). It hashes the provided value and compares it to the stored hash using constant-time comparison.
  - **Config:** Callers/config that previously held a plain API key should pass it only at startup into the above setters; for persistent config, only the hash could be stored (e.g. in a separate secure config) if desired; the in-process server never keeps the plain key.

---

## Patch #6: Timing Attack Protection (HIGH) — **ADDRESSED**

**Threat:** Non-constant-time comparison of API keys (and other secrets) could leak information via timing.

**Status:** Addressed for API key path.

### Implementation

- **Location:** `irondag-blockchain/src/rpc.rs`.
- **Behavior:** `constant_time_eq` is used for all API key checks. With Patch #5, verification compares `hash(provided_key)` to the stored hash in constant time. Signature/multisig verification elsewhere was not changed; consider an audit if those paths compare secrets.

---

## Summary

| Patch | Severity | Status |
|-------|----------|--------|
| #1 RPC auth bypass | CRITICAL | Mitigated (existing RPC + Phase 2) |
| #2 Privacy trusted setup | CRITICAL | Addressed (load keys from file) |
| #3 DSI gas metering | HIGH | Addressed (gas cap in DSI path) |
| #4 Batch rollback | HIGH | Addressed (revert callback in batch API) |
| #5 API key security | CRITICAL | Addressed (hash-only storage, header-only, constant-time) |
| #6 Timing attacks | HIGH | Addressed (constant-time API key verification) |

**Rollback:** If key loading causes issues, set `proving_key_path` and `verifying_key_path` to empty/unset to revert to per-node key generation. If hash-only API keys break existing config, callers can continue to pass the same key string into `with_auth` / `set_api_key`; verification still works (hash of provided key vs stored hash).

---

## Deployment Status

All security patches deployed and verified on two-node Testnet cluster (srv1296980 + srv1296981) as of April 10, 2026.
