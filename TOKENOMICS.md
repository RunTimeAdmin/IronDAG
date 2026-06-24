# IronDAG Protocol — IDAG Tokenomics

**Token**: IDAG  
**Type**: Native L1 coin (Polygon-wrapped ERC-20 for IKO phase; mainnet swap at launch)  
**Status**: Testnet live; tokenomics finalized for IKO/mainnet  
**Last Updated**: February 2026

*Canonical source for allocations and vesting: **IDAG_Tokenomics.docx**. This file is the repository reflection of that document.*

---

## 1. Executive Summary

| Item | Value |
|------|--------|
| **Max Supply** | 10,000,000,000 IDAG (10 billion) |
| **Genesis Allocation** | 3,000,000,000 IDAG (30%) — all non-mining, minted at genesis with vesting |
| **Mining Emission** | 7,000,000,000 IDAG (70%) — BraidCore mining over ~30 years |
| **Consensus** | GhostDAG (Proof-of-Work) with post-quantum signatures |
| **Mining Model** | BraidCore: Target GPU 45% + ASIC 35% + ZK Proving 20%. Current: CPU-only |
| **Emission Schedule** | Smooth monthly reduction (Kaspa-style chromatic decay), effective annual halving |
| **Deflationary** | 50% of transaction fees burned per block |
| **TGE Circulating** | 1.4% of total supply (20% of IKO allocation at TGE) |

---

## 2. Token Allocation

| Allocation | % | Tokens | Vesting | Purpose |
|------------|---|--------|---------|---------|
| **BraidCore Mining** | 70% | 7,000,000,000 | ~30 years via block rewards | Network security, miner incentives |
| **Ecosystem & Grants** | 8% | 800,000,000 | 6‑mo cliff, quarterly unlock over 4 years | Developer grants, partnerships |
| **IKO Public Sale** | 7% | 700,000,000 | 20% at TGE, linear over 5 months | Kommunitas IKO community |
| **Team & Founder** | 5% | 500,000,000 | 12‑mo cliff, linear over 24 mo (36 mo total) | Founder compensation, hires |
| **Development Fund** | 4% | 400,000,000 | Milestone-based | Audit, infrastructure, engineering |
| **Liquidity** | 3% | 300,000,000 | Locked ≥12 months (e.g. DxLock) | DEX LP, paired with IKO raise |
| **Marketing & Community** | 2% | 200,000,000 | Monthly unlock over 18 months | KOL, AMAs, airdrops, content |
| **Treasury Reserve** | 1% | 100,000,000 | 12‑mo lock, then governance-controlled | CEX listings, emergency, strategic |
| **TOTAL** | 100% | 10,000,000,000 | | |

At TGE, only **1.4%** of total supply is immediately circulating (20% of the 7% IKO = 140,000,000 IDAG). All other genesis tokens are locked or vesting.

---

## 3. BraidCore Mining — Emission Design

Every block reward is split across three streams (mainnet target):

| Stream | Target Share | Hardware | Current Status |
|--------|--------------|----------|----------------|
| **GPU Mining** | 45% | Consumer GPUs (NVIDIA, AMD) | In development (OpenCL pending) |
| **ASIC Mining** | 35% | Application-specific integrated circuits | Planned for mainnet |
| **ZK Proving** | 20% | ZK proof hardware (GPU/FPGA) | Planned for mainnet |

**Current Implementation**: CPU-only mining on Streams A and B. GPU mining via OpenCL is planned but not yet implemented.

### Emission Schedule (Kaspa-style)

- **Monthly reduction factor**: (1/2)^(1/12) ≈ 0.9439 — rewards decrease ~5.6% each month.
- **Effective annual halving**: After 12 months, reward is 50% of the starting reward.
- **Rough timeline**: ~50% mined by year 4; ~90% by year 10; ~99% by year 20; effectively zero by ~year 30.

Smooth decay avoids “halving shock” and gives miners predictable economics.

### Fee Economics

- **50% of transaction fees → burned** (deflationary; increases with usage).
- **50% of transaction fees → miners** (fee revenue as block rewards decline).

---

## 4. IKO (Initial Koin Offering) — Summary

| Item | Value |
|------|--------|
| **Network** | Polygon (ERC-20 wrapper); mainnet swap at launch |
| **IKO Allocation** | 700,000,000 IDAG (7%) |
| **TGE Unlock** | 20% |
| **Vesting** | Linear over 5 months |
| **Liquidity Lock** | 12 months minimum (e.g. DxLock) |
| **Refund** | 72-hour window (per IKO terms) |

---

## 5. Design Rationale (Comparables)

Tokenomics were informed by PoW L1s with similar architecture or philosophy:

| | Kaspa (KAS) | Alephium (ALPH) | Ergo (ERG) | IronDAG (IDAG) |
|---|-------------|------------------|------------|---------------------|
| **Max Supply** | 28.7B | 1B | 97.7M | **10B** |
| **Mining %** | 100% | 86% | ~95.6% | **70%** |
| **Genesis/Premine** | 0% | 14% | ~4.4% | **30%** |
| **Emission** | Smooth monthly decay | Dynamic | Quarterly -3 ERG | **Smooth monthly decay** |
| **Fee Burning** | No | 100% | — | **50%** |
| **Consensus** | GhostDAG | PoLW | Autolykos | **GhostDAG + post-quantum** |

- **70% mining**: Balances fair distribution with IKO funding needs (audits, liquidity, hiring). Genesis allocation is fully vested with verifiable locks.
- **10B supply**: Clean cap; scarce vs. 28.7B KAS; suitable for micro-transactions and long-term mining.
- **Smooth monthly emission**: Aligns with Kaspa’s proven chromatic decay; no sudden halving shocks.
- **50% fee burn**: Deflationary pressure with usage while miners retain meaningful fee income.

---

## 6. Testnet vs. Mainnet

| Aspect | Testnet (current) | Mainnet (post-IKO) |
|--------|-------------------|--------------------|
| **Supply** | 10B hard cap enforced in code (same as mainnet); rewards for testing | 10B cap; 30% genesis + 70% emission |
| **Block rewards** | 50 IDAG (single-stream, CPU-only implementation) | BraidCore split (45% GPU, 35% ASIC, 20% ZK) — target for mainnet |
| **Fee burn** | Not implemented | 50% burned, 50% to miners |
| **Emission curve** | Fixed reward | Chromatic monthly decay |

See **IDAG_Technical_Overview.docx** and **IDAG_One_Pager.docx** for technical differentiators and **IDAG_Roadmap.docx** for milestones.

---

## 7. Reference Documents (Word)

| Document | Description |
|----------|-------------|
| **IDAG_Tokenomics.docx** | Full tokenomics, vesting, milestones (authoritative for IKO) |
| **IDAG_One_Pager.docx** | One-pager: differentiators, IKO structure, use of proceeds |
| **IDAG_Technical_Overview.docx** | Technical overview and architecture |
| **IDAG_Roadmap.docx** | Roadmap and phases |

---

**Tokenomics are operational on testnet; mainnet/IKO parameters follow the above and the canonical Word documents.**

