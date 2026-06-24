# IronDAG Blockchain Architecture

This document provides comprehensive architecture diagrams for the IronDAG blockchain node.

---

## Diagram 1: System Architecture (Node Components)

A block diagram showing the major node components and their relationships:

```mermaid
graph TB
    subgraph "IronDAG Node"
        CLI["CLI / TOML Config"]
        NODE["Node Orchestrator<br/>(node/mod.rs)"]
        
        subgraph "Core Layer"
            BC["Blockchain<br/>blocks: RwLock&lt;BlocksData&gt;<br/>accounts: DashMap&lt;Addr, AccountState&gt;"]
            GHOST["GhostDAG Consensus<br/>K=4 (configurable)<br/>blue_set / red_set / blue_score"]
            STORE["Storage (Sled)<br/>Versioned binary keys<br/>Zstd compression"]
            VERKLE["Verkle State Proofs<br/>Dual Commitment<br/>KZG + Keccak"]
        end
        
        subgraph "Execution Layer"
            EVM["SputnikVM 0.41<br/>Config::shanghai() / PUSH0 verified<br/>EvmState + SputnikBackend"]
            PEVM["Parallel EVM<br/>Conflict detection<br/>Dependency graph"]
        end
        
        subgraph "Mining Layer"
            MM["MiningManager"]
            SA["Stream A<br/>Blake3 PoW<br/>10s blocks, 50 IDAG"]
            SB["Stream B<br/>B3MemHash PoW<br/>5s blocks, 25 IDAG"]
            SC["Stream C<br/>ZK Proof<br/>1s blocks, fees only"]
            POOL["Transaction Pool<br/>SegQueue (lock-free)<br/>100K capacity"]
        end
        
        subgraph "Network Layer"
            NET["NetworkManager<br/>TCP P2P (port 8080)<br/>Ed25519 signed messages"]
            SYNC["Headers-First Sync<br/>Orphan Pool"]
            PEER["Peer Scoring<br/>Latency (median of 5)<br/>Rate limiting"]
        end
        
        subgraph "API Layer"
            RPC["JSON-RPC Server<br/>(port 8545)<br/>Eth-compatible + idag_*"]
            AUTH["API Key Auth<br/>Rate Limiting<br/>Response Cache (LRU)"]
        end
        
        subgraph "Cryptography"
            PQC["Post-Quantum<br/>Dilithium3 signatures<br/>Kyber key exchange*"]
            ECDSA["ECDSA secp256k1<br/>EIP-155 signatures"]
        end
    end
    
    CLI --> NODE
    NODE --> BC
    NODE --> MM
    NODE --> NET
    NODE --> RPC
    
    BC --> GHOST
    BC --> STORE
    BC --> EVM
    EVM --> PEVM
    
    STORE --> VERKLE
    VERKLE --> |"State Proofs"| SC
    
    MM --> SA
    MM --> SB
    MM --> SC
    MM --> POOL
    MM --> BC
    
    NET --> SYNC
    NET --> PEER
    NET --> BC
    
    RPC --> AUTH
    RPC --> BC
    RPC --> MM
    RPC --> NET
    
    BC --> PQC
    BC --> ECDSA
    NET --> PQC
```

> **Note:** * Kyber key exchange is feature-flagged and optional. Enable via `--features kyber` in Cargo.toml. Includes HKDF domain separation and session caching.

---

## Diagram 2: Transaction Lifecycle

A sequence/flow diagram showing a transaction from submission to finality:

```mermaid
flowchart LR
    subgraph "1. Submission"
        RPC_IN["JSON-RPC<br/>eth_sendRawTransaction"]
        P2P_IN["P2P Gossip<br/>NewTransaction msg"]
    end
    
    subgraph "2. Validation"
        SIG["Signature Verify<br/>(ECDSA / Dilithium3)"]
        NONCE["Nonce Check<br/>(sequential or queue)"]
        BAL["Balance Check<br/>(value + fee ≤ balance)"]
        GAS["Gas/Fee Check<br/>(gas_limit ≤ 30M, fee > 0)"]
        DATA["Data Size Check<br/>(≤ 128 KB)"]
    end
    
    subgraph "3. Mempool"
        DEDUP["Pool Dedup<br/>(pool_tx_hashes)"]
        READY["Ready Queue<br/>(nonce matches)"]
        FUTURE["Future Queue<br/>(nonce gap ≤ 64)"]
    end
    
    subgraph "4. Block Assembly"
        SELECT["Fee-Priority Selection"]
        TRIM["Trim to 4MB<br/>(binary search)"]
        INFLIGHT["In-Flight Guard<br/>(cross-stream dedup)"]
    end
    
    subgraph "5. Execution"
        POW["PoW Mining<br/>(Blake3 / B3MemHash)"]
        PROC["process_blocks()<br/>BLOCK_PROCESSING_LOCK"]
        EVM_EXEC["EVM Execution<br/>(if contract tx)"]
        STATE["State Update<br/>DashMap + Sled"]
    end
    
    subgraph "6. Consensus"
        DAG["GhostDAG Insert<br/>Blue score calc"]
        ORDER["Canonical Ordering"]
        BCAST["Broadcast to Peers"]
    end
    
    RPC_IN --> SIG
    P2P_IN --> SIG
    SIG --> NONCE --> BAL --> GAS --> DATA
    DATA --> DEDUP
    DEDUP --> READY
    DEDUP --> FUTURE
    FUTURE -.->|"gap fills"| READY
    READY --> SELECT --> TRIM --> INFLIGHT
    INFLIGHT --> POW --> PROC --> EVM_EXEC --> STATE
    STATE --> DAG --> ORDER --> BCAST
```

---

## Diagram 3: GhostDAG Consensus

A diagram showing DAG structure and blue/red set selection:

```mermaid
graph TB
    subgraph "GhostDAG (K=4)"
        G["Genesis Block<br/>blue_score: 0"]
        
        A1["Block A1<br/>Stream A<br/>blue_score: 1"]
        B1["Block B1<br/>Stream B<br/>blue_score: 1"]
        
        A2["Block A2<br/>Stream A<br/>blue_score: 2"]
        B2["Block B2<br/>Stream B<br/>blue_score: 2"]
        C1["Block C1<br/>Stream C<br/>blue_score: 2"]
        
        A3["Block A3<br/>Stream A<br/>blue_score: 3"]
        
        G --> A1
        G --> B1
        A1 --> A2
        A1 --> B2
        B1 --> B2
        B1 --> C1
        A2 --> A3
        B2 --> A3
        C1 --> A3
    end
    
    subgraph "Ordering Algorithm"
        direction TB
        S1["1. New block arrives with parent hashes"]
        S2["2. Calculate blue score (weighted reachability)"]
        S3["3. Classify: Blue Set (honest) vs Red Set (attacker)"]
        S4["4. Incremental update (O(affected) not O(n²))"]
        S5["5. Final ordering by blue score (canonical chain)"]
        S1 --> S2 --> S3 --> S4 --> S5
    end
    
    subgraph "Storage"
        HOT["Hot DAG (RAM)<br/>Recent 1000 blocks"]
        COLD["Finalized (Sled disk)<br/>500+ blocks old"]
        CKPT["Checkpoints<br/>Every 100 confirmations"]
        HOT --> COLD
        HOT --> CKPT
    end

    style A1 fill:#4a9eff
    style A2 fill:#4a9eff
    style A3 fill:#4a9eff
    style B1 fill:#4a9eff
    style B2 fill:#4a9eff
    style C1 fill:#ff6b6b
    style G fill:#50c878
```

**Legend:**
- **Blue** = Blue Set (honest majority)
- **Red** = Red Set
- **Green** = Genesis

---

## Diagram 4: BraidCore Mining Architecture

```mermaid
graph TB
    subgraph "MiningManager"
        ALLOC["BlockNumberAllocator<br/>AtomicU64 (shared counter)"]
        POOL["Transaction Pool<br/>SegQueue + RwLock&lt;Vec&gt;<br/>Fee-based eviction<br/>10-min TTL"]
        INFLIGHT["In-Flight Dedup<br/>HashSet&lt;[u8;32]&gt;"]
        POOLDEDUP["Pool Dedup<br/>pool_tx_hashes"]
        FUTURE["Future Nonce Queue<br/>BTreeMap per sender<br/>Max 64 ahead, 16/sender"]
    end
    
    subgraph "Stream A - Primary (Active)"
        SA_MINE["Blake3 PoW Loop"]
        SA_BLOCK["Max 10,000 TXs<br/>10s target block time<br/>Reward: 50 IDAG"]
    end
    
    subgraph "Stream B - GPU-friendly (Active)"
        SB_MINE["B3MemHash PoW Loop"]
        SB_BLOCK["Max 5,000 TXs<br/>5s target block time<br/>Reward: 25 IDAG"]
    end
    
    subgraph "Stream C - ZK (Experimental)"
        SC_MINE["ZK Proof Generation"]
        SC_BLOCK["Max 1,000 TXs<br/>1s target block time<br/>Reward: fees only"]
    end
    
    CHAN["mpsc Channel<br/>BlockSubmission"]
    PROC["process_blocks()<br/>Validate → Execute → Persist"]
    
    POOL --> SA_MINE
    POOL --> SB_MINE
    POOL --> SC_MINE
    
    INFLIGHT --> SA_MINE
    INFLIGHT --> SB_MINE
    INFLIGHT --> SC_MINE
    
    ALLOC --> SA_MINE
    ALLOC --> SB_MINE
    ALLOC --> SC_MINE
    
    SA_MINE --> SA_BLOCK --> CHAN
    SB_MINE --> SB_BLOCK --> CHAN
    SC_MINE --> SC_BLOCK --> CHAN
    
    CHAN --> PROC
    PROC --> |"Add to chain"| BC["Blockchain"]
    PROC --> |"Insert"| DAG["GhostDAG"]
    PROC --> |"Broadcast"| NET["Network"]
    PROC --> |"Promote"| FUTURE
```

---

## Diagram 5: P2P Network Flow

```mermaid
sequenceDiagram
    participant A as Node A (Miner)
    participant B as Node B (Sync)
    participant C as Node C (Peer)
    
    Note over A,C: Connection & Handshake
    A->>B: TCP Connect (port 8080)
    A->>B: Handshake { node_id, listen_addr, pub_key }
    B->>A: Handshake { node_id, listen_addr, pub_key }
    Note over A,B: Session established (Ed25519 signed msgs)
    
    Note over A,C: Block Propagation (Gossip)
    A->>A: Mine block (Stream A, Blake3 PoW)
    A->>B: NewBlock { block }
    A->>C: NewBlock { block }
    B->>B: Validate & add to chain + DAG
    B->>C: NewBlock { block } (re-gossip)
    
    Note over A,C: Transaction Propagation
    B->>A: NewTransaction { tx }
    A->>A: Validate, add to mempool
    A->>C: NewTransaction { tx } (re-gossip, dedup via tx_seen)
    
    Note over A,C: Headers-First Sync
    B->>A: RequestHeaders { start: 0, count: 500 }
    A->>B: Headers { headers: [...] }
    B->>B: Verify header chain
    B->>A: RequestBlocks { from: 0, count: 100 }
    A->>B: Blocks { blocks: [...] }
    
    Note over A,C: Orphan Resolution
    C->>A: NewBlock { block with unknown parents }
    A->>A: Add to OrphanPool
    A->>C: RequestMissingParents { hashes: [parent1, parent2] }
    C->>A: Blocks { blocks: [parent1, parent2] }
    A->>A: Process orphans (parents now available)
```

---

## Diagram 6: Storage Architecture

```mermaid
graph TB
    subgraph "Application Layer"
        BC["Blockchain"]
        EVM["EVM Executor"]
        GHOST["GhostDAG"]
    end
    
    subgraph "Storage API (storage.rs)"
        KV["Key-Value API<br/>get / set / scan_prefix / batch_write"]
        KEY["Key Format<br/>[VERSION=0x01][TYPE][entity_bytes]"]
        COMPRESS["Zstd Compression<br/>(values > 512 bytes)"]
        MIGRATE["StorageMigrator<br/>v0 (string prefix) → v1 (binary)"]
    end
    
    subgraph "Key Prefixes"
        B01["0x01: Balance"]
        B02["0x02: Nonce"]
        B03["0x03: Contract Code"]
        B04["0x04: Contract Storage"]
        B05["0x05: DAG Children"]
        B07["0x07: Block"]
        B08["0x08: Block Height"]
        B09["0x09: Transaction"]
    end
    
    subgraph "Sled Database"
        DB["sled::Db<br/>Embedded B-tree<br/>ACID transactions"]
        CACHE["Page Cache: 256MB"]
        FLUSH["Flush: every 1000ms"]
        LOCK["File Lock (fs2)<br/>Single-process safety"]
    end
    
    BC --> KV
    EVM --> KV
    GHOST --> KV
    KV --> KEY --> COMPRESS --> DB
    KEY --> B01
    KEY --> B02
    KEY --> B03
    KEY --> B04
    KEY --> B05
    KEY --> B07
    DB --> CACHE
    DB --> FLUSH
    DB --> LOCK
```

---

## Diagram 7: Verkle State Proofs and Stream C

Verkle trees provide the foundation for IronDAG's stateless verification architecture,
enabling light clients to verify state transitions without storing the full blockchain state.

```mermaid
graph TB
    subgraph "State Change Flow"
        TX["Transaction<br/>(state modification)"]
        UPDATE["Verkle Tree Update<br/>(incremental commitment)"]
        DUAL["Dual Commitment"]
        KECCAK["Keccak Commitment<br/>(backward compatible)"]
        KZG["KZG Polynomial Commitment<br/>(ZK-friendly)"]
    end

    subgraph "Proof Consumers"
        LC["Light Client Proof<br/>(traditional verification)"]
        ZK["Stream C ZK Input<br/>(public input to circuit)"]
    end

    TX --> UPDATE
    UPDATE --> DUAL
    DUAL --> KECCAK
    DUAL --> KZG
    KECCAK --> LC
    KZG --> ZK
```

### Stateless Verification

Verkle trees replace Merkle-Patricia trees with polynomial commitments, reducing proof
size from O(log n) to O(1):

| Proof Type | Size | Verification |
|------------|------|--------------|
| Merkle Proof | ~1KB (256 hashes) | 256 Keccak hashes |
| Verkle Proof | ~200 bytes | 1 pairing check |
| KZG Verkle Proof | 32 bytes | 1 pairing check |

Light clients can verify any state value (balance, nonce, contract storage) by:
1. Holding only the current Verkle root (32 bytes)
2. Receiving a Verkle proof for the specific state path
3. Verifying the proof against the known root

### Dual Commitment Pattern

The `DualCommitment` structure bridges backward compatibility with ZK-friendly proofs:

- **Keccak Commitment**: Traditional hash-based commitment for backward compatibility
  with existing light client infrastructure. Used for standard state verification.

- **KZG Polynomial Commitment**: Zero-knowledge-friendly commitment enabling efficient
  in-circuit verification. Used by Stream C for ZK proof generation.

This dual approach allows the same Verkle tree to serve both traditional light clients
and advanced ZK proving systems without maintaining separate data structures.

### Stream C Integration

Stream C (ZK proof mining) uses Verkle state roots as public inputs to the
`StateTransitionCircuit`. The integration flow:

1. **Pre-state Root**: Current state root before transaction execution (public input)
2. **State Transition**: Transaction modifies balances, nonces, contract storage
3. **Post-state Root**: New state root after transaction execution (public input)
4. **Verkle Proof**: Proves that specific state values (e.g., sender balance)
   are committed to by the pre-state root
5. **ZK Proof**: Circuit proves correct state transition from pre to post root

The circuit includes:
- `VerklePathWitness`: Witness data for in-circuit path verification
- `verify_verkle_path_gadget()`: MiMC-based hash gadget for path verification
- Balance proofs: Authenticate sender/receiver balances from Verkle state

### Proof Efficiency in ZK Circuits

Traditional Merkle proofs require hundreds of hash operations inside the ZK circuit,
each consuming constraints. KZG-based Verkle proofs achieve:

- **Constant proof size**: 32 bytes regardless of tree depth
- **Constant verification**: Single pairing check (few constraints)
- **MiMC hashing**: ~50 constraints per balance proof vs. thousands for Merkle

This efficiency makes state proofs practical within the constraint budget of
Stream C's zero-knowledge circuits.

---

## Concurrency Model

- `blocks`: `RwLock<BlocksData>` — write-heavy during mining
- `accounts`: `DashMap<Address, AccountState>` — lock-free concurrent reads (RPC)
- `cached_latest_block_number`: `AtomicU64` — zero-lock height queries
- `BLOCK_PROCESSING_LOCK`: `parking_lot::Mutex` — prevents TOCTOU during validation
- All std Mutex/RwLock acquisitions use `.unwrap_or_else(|e| e.into_inner())` for poison recovery

---

## Known Limitations

- **Sharding**: Placeholder framework — all transactions execute on shard 0
- **DAG Finality**: No BFT finality gadget; deep reorgs possible with sufficient hashrate
- **PQ Crypto**: Kyber key exchange feature-flagged (optional, enable with `--features kyber`)
- **EVM Revision**: SputnikVM 0.41 with `Config::shanghai()`; PUSH0 (EIP-3855) verified; other Shanghai/Cancun changes untested

---

## Diagram 7: Synchronization Resilience (IBD & Fork Recovery)

```mermaid
flowchart TD
    subgraph IBD["Initial Block Download"]
        DETECT_IBD["Peer height >> local height"] --> PAUSE["⏸️ Pause Mining<br/>(AtomicBool flag)"]
        PAUSE --> CLEAR_CHECK{"Local height > 0<br/>and divergence > 50?"}
        CLEAR_CHECK -->|Yes| CLEAR_SLED["Clear all sled data<br/>(blocks, DAG edges,<br/>accounts, contracts)"]
        CLEAR_CHECK -->|No| DOWNLOAD
        CLEAR_SLED --> DOWNLOAD["Download blocks<br/>in batches of 128"]
        DOWNLOAD --> PROCESS["Process batch:<br/>add_block() each block"]
        PROCESS --> ORPHAN_CHECK{"Unresolvable<br/>parents?"}
        ORPHAN_CHECK -->|Some/All orphaned| RETAIN["Save to orphan pool<br/>(up to 10,000)"]
        ORPHAN_CHECK -->|All added| RETRY_ORPHANS["Retry accumulated<br/>orphans"]
        RETAIN --> ADVANCE["Advance to next<br/>batch range"]
        RETRY_ORPHANS --> MORE{"More blocks<br/>on peer?"}
        ADVANCE --> MORE
        MORE -->|Yes| DOWNLOAD
        MORE -->|No| FINAL_RETRY["Final orphan<br/>resolution pass"]
        FINAL_RETRY --> RESUME["▶️ Resume Mining<br/>(scopeguard)"]
    end

    style PAUSE fill:#ff9800,color:#000
    style RESUME fill:#4caf50,color:#000
    style CLEAR_SLED fill:#f44336,color:#fff
    style RETAIN fill:#2196f3,color:#fff
```

### Mining Pause During IBD

When a node detects that a peer has significantly more blocks, it enters IBD:

1. **All 3 BraidCore Mining streams pause** via a shared `AtomicBool` flag
2. Each stream checks `syncing.load(Ordering::Acquire)` before each round
3. Blocks are downloaded and processed without interference from local mining
4. Mining **resumes automatically** after sync completes — guaranteed by `scopeguard`
5. Prevents DAG tip contamination from concurrent block production

### Orphan Block Retention

During batch sync, blocks with unresolvable parents are retained:

1. Orphaned blocks saved to `accumulated_orphans` (cap: 10,000 blocks)
2. After each batch where blocks are added, accumulated orphans are retried
3. Final orphan resolution pass at end of sync loop
4. Prevents loss of valid blocks that arrive before their parents

### Fork Recovery with Comprehensive Storage Clear

When `peer_height > local_height + 50` and `local_height > 0`:

1. `clear_for_resync()` clears all in-memory state (blocks, GhostDAG, accounts)
2. `BlockStore::clear_all()` clears ALL sled prefixes:
   - `BLOCK (0x07)` — block data
   - `CHILDREN (0x05)` / `PARENTS (0x06)` — DAG edges
   - `BALANCE (0x01)` / `NONCE (0x02)` — account state
   - `CONTRACT (0x03)` / `CONTRACT_STORAGE (0x04)` — contract data
   - `BLOCK_HEIGHT (0x08)` — height index
3. Sync restarts from block 0 with a clean slate

### Sync Stall Prevention

When an entire batch contains only orphaned blocks:

1. Batch is not discarded — orphans saved for later retry
2. Sync advances `current` to `highest_in_batch + 1`
3. Next batch is requested from peer
4. Prevents indefinite stall on all-orphaned responses

