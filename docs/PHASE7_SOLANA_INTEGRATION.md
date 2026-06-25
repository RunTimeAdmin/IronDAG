# Phase 7 — Solana Integration & ZK Bridge

**Status:** Planning  
**Prerequisites:** Phases 1–6 complete ✅  
**Estimated duration:** 8–12 weeks  
**Architecture spec:** `docs/SOLANA_HYBRID_ARCHITECTURE.md`

---

## Objective

Deploy $IDAG as an SPL token on Solana, launch via Fjord Foundry LBP, establish
Realms DAO governance, and build a zero-knowledge bridge that relays governance
decisions trustlessly from Solana to the IronDAG native chain.

---

## Milestones

### 7.1 — SPL Token + Vesting Infrastructure
**Effort:** 2–3 days  
**Status:** Pending

| Task | Tool | Notes |
|---|---|---|
| Mint $IDAG as SPL Token-2022 | `spl-token` CLI | 10B supply, decide freeze authority |
| Create Squads 3-of-5 multisig | Squads | Core team keys; controls treasury |
| Set up team vesting streams | Streamflow Finance | 12mo cliff, 24mo linear |
| Set up ecosystem grant streams | Streamflow Finance | 6mo cliff, 48mo quarterly |
| Set up development fund | Streamflow Finance | Milestone-based unlock |
| Lock treasury reserve | Streamflow Finance | 12mo lock, then Realms-controlled |

**Deliverables:** SPL token live on Solana mainnet; all locked allocations verifiable on-chain.

---

### 7.2 — Realms DAO Setup
**Effort:** 1–2 days  
**Status:** Pending  
**Dependency:** 7.1 complete

| Task | Notes |
|---|---|
| Create IronDAG Realm | Community token: $IDAG SPL; Council: Squads multisig |
| Configure vote thresholds | 1% of circulating supply to create proposal |
| Configure vote duration | 72 hours vote window |
| Configure execution delay | 24 hours (bridge relay window) |
| Define governance instruction schema | Typed payloads for each IronDAG parameter |
| Test end-to-end governance proposal | Submit → vote → execute (devnet) |

**Deliverables:** Live Realms DAO with full governance flow tested on devnet.

---

### 7.3 — Fjord Foundry LBP Application
**Effort:** 1 day to apply; 1–2 weeks review  
**Status:** Pending  
**Dependency:** 7.1 complete (token must exist to apply)

| Task | Notes |
|---|---|
| Complete Fjord project application | Requires: token address, docs, socials, team info |
| Prepare LBP parameters | 99/1 weight, 72hr, ~$500 USDC seed, price range |
| Prepare sale page content | Project description, tokenomics summary, links |
| Prepare post-LBP Raydium pool | IDAG/USDC CLMM pool seeded from raised capital |

**LBP parameters:**
```
Start weight:   99% IDAG / 1% USDC
End weight:     50% IDAG / 50% USDC
Duration:       72 hours
Seed capital:   ~$500 USDC
Sale supply:    700,000,000 IDAG (7% of total)
Accepted token: USDC
```

**Deliverables:** Fjord application submitted; LBP ready to launch on approval.

---

### 7.4 — IronDAG Governance RPC + Parameter Store
**Effort:** 3–5 days  
**Status:** Pending  
**Files to create/modify:**
- `irondag-blockchain/src/governance/mod.rs` — NEW
- `irondag-blockchain/src/rpc/mod.rs` — ADD `irondag_applyGovernance`
- `irondag-blockchain/src/bin/node.rs` — wire GovernanceScheduler into mining loop

**Tasks:**

1. **Create `src/governance/mod.rs`**
   - `GovernanceScheduler` struct with `BTreeMap<u64, GovernanceAction>`
   - `GovernanceAction` enum covering all governable parameters
   - `schedule(block_height, action)` and `actions_at(block_height)` methods

2. **Add `irondag_applyGovernance` RPC method**
   - Accept: proof, verifier_key, proposal_id, parameter, value, effective_block
   - Phase 7a: validate relayer multisig signatures (interim trust model)
   - Phase 7b: validate ProveKit WHIR proof (ZK trust model)
   - Minimum `effective_block` = current_block + 10

3. **Wire into mining loop**
   - After each block commit, call `GovernanceScheduler::actions_at(block_number)`
   - Apply any pending actions before computing next block reward

4. **Emit governance events**
   - `GovernanceApplied { proposal_id, parameter, value, block }` in block data

**Deliverables:** Node accepts and applies governance messages; tested via unit + integration tests.

---

### 7.5 — MVP Bridge: Trusted Relayer (Phase 7a)
**Effort:** 5–7 days  
**Status:** Pending  
**Dependency:** 7.2, 7.4 complete

A lightweight Rust binary that watches Solana for finalized Realms proposals and
submits signed governance messages to IronDAG nodes. This is the interim trust model
(honest about its assumption: requires 3-of-5 relayer keys to co-sign).

**New binary:** `irondag-blockchain/src/bin/bridge_relayer.rs`

```
1. Subscribe to Realms governance program on Solana via WebSocket (Solana RPC)
2. When proposal reaches Succeeded + Executed:
   a. Fetch proposal account + instruction data (the governance payload)
   b. Build GovernanceMessage { proposal_id, parameter, value, effective_block }
   c. Sign with this relayer's key
   d. Broadcast to other relayers (P2P or shared endpoint)
   e. Collect threshold signatures (3-of-5)
   f. POST to each IronDAG node's irondag_applyGovernance RPC
```

**Dependencies:**
- `solana-client` crate for Solana RPC
- `solana-sdk` for account deserialization
- `spl-governance` crate for Realms proposal parsing

**Deliverables:** End-to-end: governance vote on Solana devnet → IronDAG testnet applies parameter change.

---

### 7.6 — ZK Bridge: ProveKit Circuit (Phase 7b)
**Effort:** 3–4 weeks  
**Status:** Future (after 7.5 proven)  
**Reference:** `D:\IRONDAG\ProveKit\`

Replaces the trusted relayer with a zero-knowledge proof. Any participant can generate
the proof; IronDAG nodes verify it without trusting the submitter.

**Circuit:** `bridge/src/main.nr` (Noir)

Proves:
1. A Solana block header chain is valid (rolling window of N headers)
2. A Realms proposal account exists at the latest state root
3. The proposal reached `Succeeded` status with sufficient vote weight
4. The governance payload matches the proposal's instruction data

**Rust integration:**

Using ProveKit's Rust FFI:
```rust
// In irondag-blockchain/src/governance/verifier.rs
use provekit_ffi::{verify_proof, ProofBytes, VerifierKey};

pub fn verify_governance_proof(
    proof: &[u8],
    verifier_key: &[u8],
    public_inputs: &GovernancePublicInputs,
) -> Result<bool, GovernanceError> {
    verify_proof(proof, verifier_key, &public_inputs.encode())
        .map_err(GovernanceError::ProveKitError)
}
```

**Steps:**
1. Study ProveKit examples in `D:\IRONDAG\ProveKit\noir-examples\`
2. Design Solana header chain verification in Noir
3. Design Realms account proof verification in Noir
4. Implement `bridge.nr` circuit
5. Generate proving/verifier keys with `provekit-cli prepare`
6. Integrate verifier via FFI into IronDAG node
7. Build proof generation tool (standalone binary or ProveKit CLI wrapper)
8. Replace relayer signature check in `irondag_applyGovernance` with proof check
9. End-to-end test on testnet

**Deliverables:** Trustless bridge operational; relayer binary deprecated.

---

## Summary Timeline

```
Week 1:   7.1 SPL token mint + Streamflow vesting + Squads
Week 1:   7.3 Fjord application submitted
Week 2:   7.2 Realms DAO setup (devnet testing)
Week 3:   7.4 IronDAG governance RPC + GovernanceScheduler
Week 4:   7.5 Bridge relayer binary (trusted MVP)
Week 4:   7.5 End-to-end test: Solana devnet → IronDAG testnet
Week 5:   7.3 LBP launch (pending Fjord approval)
Week 6+:  7.6 Begin ProveKit Noir circuit design
Week 8+:  7.6 ZK bridge testnet
Week 10+: 7.6 ZK bridge mainnet; relayer deprecated
```

---

## Risk Register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Fjord Foundry review takes >2 weeks | Medium | Begin all other work in parallel; LBP is last step |
| ProveKit Solana header circuit complexity | High | Start with simplified trusted-relayer MVP; ZK is Phase 7b |
| SPL Token-2022 compatibility with Realms | Low | Realms supports Token-2022; test on devnet first |
| Governance RPC exploited before ZK bridge | Medium | Rate-limit RPC; require relayer threshold; audit before mainnet |
| IronDAG parameter change causes consensus issue | Medium | `effective_block` minimum 10 blocks ahead; rollback mechanism |

---

## Files Reference

| File | Action |
|---|---|
| `TOKENOMICS.md §4` | Updated — Fjord LBP replaces Kommunitas IKO |
| `docs/SOLANA_HYBRID_ARCHITECTURE.md` | New — full architecture specification |
| `docs/PHASE7_SOLANA_INTEGRATION.md` | This document |
| `irondag-blockchain/src/governance/mod.rs` | To be created (7.4) |
| `irondag-blockchain/src/bin/bridge_relayer.rs` | To be created (7.5) |
| `D:\IRONDAG\ProveKit\` | Reference implementation for ZK proving |
