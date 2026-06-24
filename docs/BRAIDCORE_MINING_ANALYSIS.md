# BraidCore Mining — Code vs Documentation Analysis

**Date:** February 2026  
**Scope:** Implementation in `irondag-blockchain` vs TOKENOMICS.md / IDAG_Tokenomics.docx

---

## 1. What the Code Implements

### Stream definitions (code)

| Stream | Algorithm    | Block time | Max txs/block | Block reward | Hardware target |
|--------|--------------|------------|----------------|---------------|------------------|
| **A**  | Blake3       | 10 s       | 10,000         | **50 IDAG**   | ASIC             |
| **B**  | KHeavyHash   | 1 s        | 5,000          | **25 IDAG**   | CPU/GPU          |
| **C**  | Keccak256*   | 100 ms     | 1,000          | **0 IDAG**    | Fee-based only   |

\*Stream C uses default Keccak256 for block hash; no ZK proof generation or verification is implemented.

**Sources:** `src/mining.rs` (rewards, block times, loop), `src/pow.rs` (Blake3, KHeavyHash), `src/blockchain/mod.rs` (hash by stream type), `src/types.rs` (StreamType enum).

### Behaviour

- **Real PoW:** Stream A and B use actual proof-of-work (`pow::mine_block` with difficulty). Difficulty is adjusted per stream toward target block times (10s / 1s).
- **Stream C:** Builds a block every 100 ms, collects fees, reward = 0. No ZK proof; hash is Keccak256. Effectively a timer-based, fee-only stream.
- **Single-stream mode:** `--single-stream` runs only Stream A (reduces CPU; used on testnet).
- **No supply cap:** Rewards are fixed 50 / 25 / 0 per block with no cap or decay in code.
- **No fee burn:** Fee handling is not split 50% burn / 50% miner in the codebase.

---

## 2. What the Documentation Says (Tokenomics)

### BraidCore shares (TOKENOMICS.md / Word)

- **GPU Mining:** 45% of block reward emission  
- **ASIC Mining:** 35% of block reward emission  
- **ZK Proving:** 20% of block reward emission  

Plus: Kaspa-style chromatic decay, ~30-year emission, 1B total supply, 50% of fees burned.

---

## 3. Gaps and Mismatches

### 3.1 Reward split: code vs 45/35/20

- **Doc:** GPU 45%, ASIC 35%, ZK 20% of *each* block reward (or of total emission).
- **Code:** Fixed 50 IDAG (A), 25 IDAG (B), 0 (C) *per block*, with different block times.

**Effective emission per 10 seconds (current code):**

- Stream A (ASIC): 1 block × 50 = **50 IDAG**
- Stream B (GPU): 10 blocks × 25 = **250 IDAG**
- Stream C (ZK): 100 blocks × 0 = **0 IDAG**

So, for block rewards only, the ratio is **A : B : C = 50 : 250 : 0** → ~**17% : 83% : 0%**.

- **ASIC (A)** gets much *less* than the doc’s 35% (code ≈17%).
- **GPU (B)** gets much *more* than the doc’s 45% (code ≈83%).
- **ZK (C)** gets 0% of block rewards; doc says 20%.

So the **implemented reward split does not match the 45/35/20 design**. Aligning code to tokenomics would require either:

- Changing block rewards and/or block times so that the *time-averaged* emission matches 45% GPU, 35% ASIC, 20% ZK, or  
- Introducing an explicit “reward pool” split (e.g. 45/35/20) and feeding it from a single emission schedule.

### 3.2 Stream C (ZK) — no 20% and no ZK

- **Doc:** ZK Proving 20% of emission; ZK proof hardware.
- **Code:** Stream C has 0 block reward and no ZK proof logic; blocks are hashed with Keccak256 and produced on a 100 ms timer. So:
  - The “20%” for ZK is not implemented.
  - Stream C is fee-only and not a real ZK-proving stream yet.

### 3.3 Emission schedule and supply cap

- **Doc:** Smooth monthly decay (e.g. (1/2)^(1/12)), ~30-year emission, 1B total supply.
- **Code:** Constant 50/25/0 per block; no decay, no cap. Chromatic decay and supply cap are **not implemented**.

### 3.4 Fee burn

- **Doc:** 50% of transaction fees burned, 50% to miners.
- **Code:** No 50/50 fee split or burn logic in the reviewed paths. Fee burn is **not implemented**.

---

## 4. What Matches

- **Stream roles:** A = ASIC (Blake3), B = CPU/GPU (KHeavyHash), C = fee-only — roles are consistent with the narrative, except C is not ZK.
- **Block times:** 10s / 1s / 100ms and max txs (10k / 5k / 1k) match the design described in comments and docs.
- **PoW:** Real mining with difficulty adjustment for A and B; implementation is sound for testnet.
- **Single-stream mode:** Fits “Stream A only” / resource-constrained use and is implemented.

---

## 5. Recommendations

| Priority | Item | Action |
|----------|------|--------|
| **High** | Align emission split with 45/35/20 | Define target emission ratio (GPU 45%, ASIC 35%, ZK 20%); adjust block rewards and/or block times (or add a shared emission pool) so time-averaged emission matches. Document “current testnet = simplified (e.g. A only or A+B)” if not yet at 45/35/20. |
| **High** | ZK stream and 20% | Either (a) implement real ZK proof generation/verification for Stream C and assign 20% of emission to it, or (b) keep C as fee-only and update docs to “GPU 45%, ASIC 35%, fee-only (C) 20%” only when C is given a reward share in code. |
| **Medium** | Emission schedule | Implement chromatic decay (e.g. monthly factor) and a 1B supply cap so mainnet matches tokenomics. |
| **Medium** | Fee burn | Implement 50% of fees burned, 50% to miners per block. |
| **Low** | Naming consistency | Doc uses “GPU / ASIC / ZK”; code uses Stream A/B/C and “ASIC / CPU/GPU / ZK”. Add a one-line mapping in README or tokenomics (A=ASIC, B=GPU, C=ZK) so code and doc are obviously aligned. |

---

## 6. File Reference

| Area | Files |
|------|--------|
| Rewards, block times, loops | `irondag-blockchain/src/mining.rs` |
| PoW (Blake3, KHeavyHash, difficulty) | `irondag-blockchain/src/pow.rs` |
| Stream type, hash by stream | `irondag-blockchain/src/types.rs`, `src/blockchain/mod.rs` |
| Single-stream / CLI | `irondag-blockchain/src/node/mod.rs`, `src/bin/node.rs` |
| Tokenomics (target design) | `TOKENOMICS.md`, `IDAG_Tokenomics.docx` |

---

**Bottom line:** BraidCore is implemented with three streams and correct PoW for A and B, but the **reward split does not match the documented 45/35/20**, Stream C has **no ZK and no 20% share**, and **chromatic decay, supply cap, and fee burn** are not in the code. Updating either the code or the docs (and clarifying "testnet vs mainnet") will avoid future confusion.
