# CPU Optimization — Implemented Changes

Summary of changes applied from the CPU Usage and Optimization Analysis to reduce load on 4-core nodes.

## Implemented (Phases 1–3)

| Change | File | Effect |
|--------|------|--------|
| **Stream C block time** | `mining.rs` | `STREAM_C_BLOCK_TIME`: 100ms → 1s. Lowers lock churn and Stream C CPU. |
| **Stream C optional** | `node/mod.rs`, `bin/node.rs` | `enable_stream_c` in `NodeConfig` (default **false**). CLI: `--enable-stream-c`. Stream C only runs when enabled. |
| **Stream B target time** | `pow.rs` | `STREAM_B_TARGET_TIME`: 1s → 5s. Fewer KHeavyHash blocks, less CPU/memory bandwidth. |
| **Stream B sleep** | `mining.rs` | Empty-txs sleep uses `Duration::from_secs(pow::STREAM_B_TARGET_TIME)`. |
| **KHeavyHash memory** | `pow.rs` | `KHEAVY_MEMORY_SIZE`: 1MB → 256KB. Lower memory bandwidth use. |
| **KHeavyHash passes** | `pow.rs` | `KHEAVY_PASSES`: 3 → 2. Less work per hash. |
| **Stream B parallel mining** | `mining.rs` | `mine_stream_b` uses `pow::mine_block_parallel(..., 100_000, None)` instead of `mine_block`. Uses all cores for Stream B PoW. |
| **Network read timeout** | `network.rs` | Timeout no longer fixed 1s; uses exponential backoff: start 100ms, double on timeout (max 5s), reset to 100ms on successful read. Cuts idle CPU spinning. |

## Not implemented (optional / later)

- **CPU affinity** (Linux-only, libc)
- **Fairness analysis caching**
- **Lock-free transaction pool**
- **Adaptive throttling / CPU monitoring**

## Usage

- **Default (TriStream, Stream C off):** `./node` — Stream A + B only; Stream C disabled.
- **Enable Stream C:** `./node --enable-stream-c`
- **Single-stream (Stream A only):** `./node --single-stream`

## Expected impact

- **Before:** ~400% CPU (4 cores saturated), high lock contention, memory bandwidth pressure.
- **After (default):** Lower CPU (Stream C off, Stream B 5s, KHeavy 256KB/2 passes, parallel Stream B, network backoff). Target range ~100–200% on 4-core for typical loads.

*See the original CPU Usage and Optimization Analysis and Implementation Guide for full rationale and optional phases.*
