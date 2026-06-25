# IronDAG — Solana Hybrid Architecture

**Status:** Design phase (Phase 7)  
**Last Updated:** June 2026  
**Reference implementation:** `D:\IRONDAG\ProveKit` (cloned from github.com/worldfnd/ProveKit)

---

## Overview

IronDAG operates as a hybrid two-layer system:

```
[Solana Layer]          — Governance + Liquidity + Token Distribution
       |
  [ZK Bridge]          — Trustless state relay via zero-knowledge proofs (ProveKit)
       |
[IronDAG Native L1]    — Dual-Stream BraidCore Execution + B3MemHash PoW + ML-KEM-768 P2P
```

This architecture solves the cold-start problem of bootstrapping a new L1:
- Solana provides immediate liquidity, community tooling, and governance infrastructure
- IronDAG runs its proprietary execution engine independently
- The ZK bridge connects them trustlessly — no relayer multisig required

---

## Layer 1: Solana (Front of House)

### $IDAG SPL Token

| Property | Value |
|---|---|
| Standard | SPL Token-2022 |
| Total supply | 10,000,000,000 IDAG |
| Freeze authority | To be determined (keep for compliance or burn for trustlessness) |
| Transfer fee | Optional (Token-2022 feature) |

**Mint command:**
```bash
spl-token create-token --program-id TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb
spl-token create-account <MINT_ADDRESS>
spl-token mint <MINT_ADDRESS> 10000000000
```

### Public Sale — Fjord Foundry LBP

See `TOKENOMICS.md §4` for full parameters. Key points:
- 99/1 starting weight (IDAG/USDC) — requires only ~$500 seed capital
- 72-hour duration with natural price decay
- Post-LBP: raised USDC + remaining tokens → permanent Raydium CLMM pool

### Vesting — Streamflow Finance

All locked allocations (team, treasury, ecosystem, development) are managed via
Streamflow Finance on-chain, governed by a Squads multisig. This provides:
- On-chain verifiable vesting schedules
- Non-custodial, no trusted third party
- Visible on Streamflow dashboard for community transparency

### Governance — Realms DAO

```
Realm:              IronDAG
Community token:    $IDAG SPL
Council:            Squads 3-of-5 (core team veto)
Proposal threshold: 1% of circulating supply
Vote duration:      72 hours
Execution delay:    24 hours (bridge relay window)
```

Governance proposals encode IronDAG parameter changes as typed instruction payloads.
When a proposal reaches `Succeeded + Executed` state on Solana, the ZK bridge relays
the cryptographic proof to IronDAG nodes, which apply the change at a specified block height.

**Governable IronDAG parameters:**
- `b3memhash_difficulty_target` — mining difficulty bounds
- `braid_stream_ratio` — fast/slow stream weighting
- `block_reward_adjustment` — emission schedule modifications
- `fee_floor` — minimum transaction fee
- `kyber_key_rotation_period` — ML-KEM-768 key rotation cadence
- `finality_depth` — confirmation depth for checkpoint pruning

---

## Layer 2: ZK Bridge (The Connective Tissue)

The bridge relays finalized Solana governance state to IronDAG nodes using
zero-knowledge proofs — no trusted relayer, no multisig attestation required.

### Reference Implementation

**ProveKit** (World Foundation) is the proving toolkit used to build the bridge circuit.

```
Local clone:  D:\IRONDAG\ProveKit
Source:       github.com/worldfnd/ProveKit
License:      MIT
Stack:        Rust + Noir circuits + WHIR proof system (Spartan-based)
```

ProveKit compiles Noir circuits to R1CS constraints and generates WHIR proofs with
SIMD-accelerated field arithmetic. It supports Rust FFI, enabling direct embedding
into the IronDAG node binary.

### How It Works

```
1. Governance vote finalizes on Solana (Realms proposal: Succeeded + Executed)
         |
2. Off-chain prover (any participant) fetches:
   - Solana block headers (rolling window for light client)
   - Realms proposal account state
   - Vote records proving quorum
         |
3. Prover runs ProveKit circuit:
   - Input:  Solana account proof + block headers + proposal payload
   - Output: WHIR proof that the proposal is finalized and the payload is authentic
         |
4. Proof + payload submitted to IronDAG via new RPC method: irondag_applyGovernance
         |
5. IronDAG node verifies proof using embedded ProveKit verifier (Rust FFI)
   - No trust in the submitter required — math guarantees correctness
         |
6. If valid: parameter change scheduled at effective_block height
   If invalid: rejected silently
```

### IronDAG RPC: `irondag_applyGovernance`

New JSON-RPC method added to the IronDAG node:

```json
{
  "jsonrpc": "2.0",
  "method": "irondag_applyGovernance",
  "params": [{
    "proof":           "<ProveKit WHIR proof, base64>",
    "verifier_key":    "<ProveKit .pkv key, base64>",
    "proposal_id":     "<Solana Realms proposal pubkey>",
    "parameter":       "b3memhash_difficulty_target",
    "value":           "22",
    "effective_block": 88400
  }],
  "id": 1
}
```

The node:
1. Deserializes the proof and verifier key
2. Calls `provekit_verify(proof, verifier_key, public_inputs)` via FFI
3. Checks `effective_block` is in the future (minimum 10 blocks ahead)
4. Schedules the parameter change via `GovernanceScheduler`
5. Emits a `GovernanceApplied` event in the next mined block

### Noir Circuit Design

The bridge circuit (`bridge.nr`) proves:
1. The proposal account exists at a known Solana block hash
2. The block hash is part of a valid Solana header chain
3. The proposal reached `Succeeded` status with sufficient vote weight
4. The payload (parameter + value) matches the proposal's instruction data

```noir
// Simplified sketch — full circuit TBD
fn main(
    block_headers: [BlockHeader; WINDOW_SIZE],
    proposal_account: AccountProof,
    vote_records: [VoteRecord; MAX_VOTES],
    payload: GovernancePayload,
) -> pub GovernanceProofOutput {
    // 1. Verify header chain
    verify_header_chain(block_headers);
    // 2. Verify account proof against latest header
    verify_account_proof(proposal_account, block_headers[WINDOW_SIZE-1].state_root);
    // 3. Verify vote quorum
    let vote_weight = sum_votes(vote_records, proposal_account.governance_config);
    assert(vote_weight >= proposal_account.governance_config.vote_threshold);
    // 4. Bind payload to proposal
    assert(hash(payload) == proposal_account.instruction_data_hash);
    GovernanceProofOutput { proposal_id: proposal_account.pubkey, payload }
}
```

---

## Layer 3: IronDAG Native L1 (Back of House)

The IronDAG node runs independently. Mining, transaction execution, and P2P networking
operate without any dependency on Solana. The bridge is additive — IronDAG functions
fully without it; governance is simply an additional input channel when active.

### Governance Parameter Store

New module: `irondag-blockchain/src/governance/mod.rs`

```rust
pub struct GovernanceScheduler {
    pending: BTreeMap<u64, GovernanceAction>,  // block_height → action
}

pub enum GovernanceAction {
    SetDifficultyTarget(u8),
    SetBraidStreamRatio(u8, u8),
    SetFeeFloor(u64),
    SetKyberRotationPeriod(u64),
    SetFinalityDepth(u64),
}
```

At each block, the mining loop checks `GovernanceScheduler::actions_at(block_height)` and applies any scheduled changes before computing the block reward.

---

## MVP Rollout (Phase 7)

See `docs/PHASE7_SOLANA_INTEGRATION.md` for the full milestone plan.

**Critical path:**
```
Week 1:  Mint $IDAG SPL, set up Streamflow vesting, configure Squads multisig
Week 2:  Apply for Fjord Foundry (1-2 week review); set up Realms DAO in parallel
Week 3:  Implement irondag_applyGovernance RPC + GovernanceScheduler in node
Week 4:  MVP bridge: trusted relayer (multisig threshold) as interim before ZK
Week 5:  Begin Noir circuit for Solana state proof (using ProveKit)
Week 6:  LBP launch on Fjord Foundry
Week 8+: ZK bridge testnet; replace relayer with proof-based verification
```

### Trust Model Progression

```
Phase 7a (MVP):  Trusted multisig relayer (3-of-5 keys co-sign governance messages)
                 → Fast to build, honest about trust assumption, works immediately

Phase 7b (ZK):   ProveKit WHIR proof replaces relayer signatures entirely
                 → Trustless, no single point of failure, mathematically verifiable
```

---

## Repository Layout

```
D:\IRONDAG\
  ProveKit\                    — Reference ZK proving toolkit (World Foundation)
    provekit\                  — Core proving engine
    recursive-verifier\        — On-chain Groth16 verifier export (bridge piece)
    noir-examples\             — Working circuit examples
    docs\                      — Architecture documentation

  mondoshawan\
    irondag-blockchain\
      src\
        governance\            — TO BE CREATED: GovernanceScheduler, param store
        rpc\                   — ADD: irondag_applyGovernance method
    docs\
      SOLANA_HYBRID_ARCHITECTURE.md   — This document
      PHASE7_SOLANA_INTEGRATION.md    — Phase 7 roadmap
```

---

## Related Documents

| Document | Description |
|---|---|
| `TOKENOMICS.md §4` | Fjord Foundry LBP parameters and vesting structure |
| `docs/PHASE7_SOLANA_INTEGRATION.md` | Milestone plan and implementation order |
| `D:\IRONDAG\ProveKit\README.md` | ProveKit usage and circuit examples |
| `D:\IRONDAG\ProveKit\docs\` | ProveKit architecture deep-dive |
| `D:\IRONDAG\ProveKit\recursive-verifier\` | On-chain verifier for Groth16 wrapping |
| `D:\IRONDAG\ProveKit\noir-examples\` | Working Noir circuit examples to learn from |
