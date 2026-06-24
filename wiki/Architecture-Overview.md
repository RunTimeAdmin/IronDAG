# Architecture Overview

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           IronDAG Node                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │  JSON-RPC   │  │   gRPC      │  │   P2P       │  │  Metrics    │    │
│  │  Server     │  │   Server    │  │   Network   │  │  (Prometheus)│    │
│  │  (8545)     │  │   (50051)   │  │             │  │  (9090)     │    │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └─────────────┘    │
│         │                │                │                              │
│         └────────────────┴────────────────┘                              │
│                          │                                               │
│  ┌───────────────────────▼───────────────────────────────────────────┐  │
│  │                     Transaction Pool                               │  │
│  │  - Pending transactions                                            │  │
│  │  - Gas price ordering                                              │  │
│  │  - MEV protection                                                  │  │
│  └───────────────────────┬───────────────────────────────────────────┘  │
│                          │                                               │
│  ┌───────────────────────▼───────────────────────────────────────────┐  │
│  │                    TriStream Mining                                │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐                         │  │
│  │  │ Stream A │  │ Stream B │  │ Stream C │                         │  │
│  │  │ (Blake3) │  │(KHeavyHash)│ │(Keccak256)│                        │  │
│  │  └────┬─────┘  └────┬─────┘  └────┬─────┘                         │  │
│  │       └─────────────┼─────────────┘                                │  │
│  └─────────────────────┼─────────────────────────────────────────────┘  │
│                        │                                                 │
│  ┌─────────────────────▼─────────────────────────────────────────────┐  │
│  │                    GhostDAG Consensus                              │  │
│  │  - Blue/Red block classification                                   │  │
│  │  - Block ordering by blue score                                    │  │
│  │  - Hybrid storage (hot cache + disk)                               │  │
│  └─────────────────────┬─────────────────────────────────────────────┘  │
│                        │                                                 │
│  ┌─────────────────────▼─────────────────────────────────────────────┐  │
│  │                      Blockchain Core                               │  │
│  │  - Block validation                                                │  │
│  │  - Transaction processing                                          │  │
│  │  - State management (DashMap for lock-free reads)                  │  │
│  └─────────────────────┬─────────────────────────────────────────────┘  │
│                        │                                                 │
│  ┌─────────────────────▼─────────────────────────────────────────────┐  │
│  │                   EVM Executor (SputnikVM 0.41)                     │  │
│  │  - Contract deployment                                             │  │
│  │  - Contract calls                                                  │  │
│  │  - Storage management                                              │  │
│  └─────────────────────┬─────────────────────────────────────────────┘  │
│                        │                                                 │
│  ┌─────────────────────▼─────────────────────────────────────────────┐  │
│  │                    Persistent Storage (sled)                       │  │
│  │  - Blocks        - Accounts        - Contract code                 │  │
│  │  - Transactions  - Nonces          - Contract storage              │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Component Relationships

### Core Data Flow

```
Transaction → Pool → Mining → Block → Consensus → Blockchain → Storage
                                ↓
                              EVM
                                ↓
                           State Changes
```

### Module Dependencies

```
                    ┌──────────────┐
                    │     lib      │
                    └──────┬───────┘
                           │
    ┌──────────────────────┼──────────────────────┐
    │                      │                      │
    ▼                      ▼                      ▼
┌────────┐          ┌──────────┐           ┌──────────┐
│ types  │          │  error   │           │  config  │
└────┬───┘          └────┬─────┘           └────┬─────┘
     │                   │                      │
     └───────────────────┴──────────────────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
         ▼               ▼               ▼
    ┌─────────┐    ┌──────────┐    ┌──────────┐
    │ storage │    │blockchain│    │   pow    │
    └────┬────┘    └────┬─────┘    └────┬─────┘
         │              │               │
         └──────────────┼───────────────┘
                        │
              ┌─────────┼─────────┐
              │         │         │
              ▼         ▼         ▼
         ┌────────┐ ┌────────┐ ┌────────┐
         │  evm   │ │consensus│ │ mining │
         └────────┘ └────────┘ └────────┘
                        │
              ┌─────────┼─────────┐
              │         │         │
              ▼         ▼         ▼
         ┌────────┐ ┌────────┐ ┌────────┐
         │sharding│ │ network│ │  rpc   │
         └────────┘ └────────┘ └────────┘
```

---

## Concurrency Model

### Fine-Grained Locking

The blockchain uses a fine-grained locking strategy to maximize concurrency:

| Data | Lock Type | Purpose |
|------|-----------|---------|
| Blocks | `RwLock` | Coordinated block updates |
| Accounts | `DashMap` | Lock-free concurrent reads |
| Block Height | `AtomicU64` | Lock-free height queries |
| GhostDAG | Hybrid | Hot cache + disk storage |

### Lock-Free Operations

```rust
// Account balance reads are lock-free (DashMap)
pub fn get_balance(&self, address: Address) -> u128 {
    self.accounts.get(&address)
        .map(|state| state.balance)
        .unwrap_or(0)
}

// Block height is atomic (no lock needed)
pub fn get_latest_block_number(&self) -> u64 {
    self.cached_latest_block_number.load(Ordering::Acquire)
}
```

---

## Storage Architecture

### Hybrid Storage Model

```
┌─────────────────────────────────────────────────────┐
│                   Hot Cache (RAM)                    │
│  - Recent 1000 blocks                               │
│  - Active accounts                                  │
│  - Pending transactions                             │
├─────────────────────────────────────────────────────┤
│                 Cold Storage (sled)                  │
│  - Finalized blocks (depth > 500)                   │
│  - Historical state                                 │
│  - Contract code and storage                        │
└─────────────────────────────────────────────────────┘
```

### Database Keys

| Prefix | Data Type |
|--------|-----------|
| `block:` | Block data |
| `tx:` | Transaction data |
| `state:balance:` | Account balances |
| `state:nonce:` | Account nonces |
| `state:code:` | Contract bytecode |
| `state:storage:` | Contract storage |

---

## Network Architecture

### P2P Protocol

- **Transport**: TCP with noise encryption
- **Discovery**: Kademlia DHT
- **Gossip**: Block and transaction propagation
- **Sync**: Request/response for missing blocks

### Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 8545 | HTTP | JSON-RPC API |
| 8546 | WebSocket | JSON-RPC subscriptions |
| 50051 | gRPC | Internal RPC |
| 30303 | TCP | P2P network |
| 9090 | HTTP | Prometheus metrics |

---

## Security Layers

```
┌─────────────────────────────────────────┐
│           Rate Limiting                  │
│  - Per-IP request limits                │
│  - Burst protection                     │
├─────────────────────────────────────────┤
│           IP Filtering                   │
│  - Whitelist/blacklist                  │
│  - Geographic restrictions              │
├─────────────────────────────────────────┤
│         Fraud Detection                  │
│  - Pattern matching                     │
│  - Risk scoring                         │
│  - Anomaly detection                    │
├─────────────────────────────────────────┤
│        Transaction Validation            │
│  - Signature verification               │
│  - Balance checks                       │
│  - Nonce validation                     │
└─────────────────────────────────────────┘
```

---

## Next Steps

- [Module Reference](Module-Reference) - Detailed module documentation
- [Dependencies](Dependencies) - External libraries used
- [API Reference](API-Reference) - JSON-RPC API guide
