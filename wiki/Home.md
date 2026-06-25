# IronDAG Blockchain Wiki

**IronDAG Protocol** | Ticker: **IDAG**

High-performance sharded blockchain with BraidCore Mining Architecture and GhostDAG consensus.

---

## Quick Links

| Section | Description |
|---------|-------------|
| [Architecture Overview](Architecture-Overview) | System design and component relationships |
| [Module Reference](Module-Reference) | Complete module documentation |
| [Dependencies](Dependencies) | External crates and libraries |
| [API Reference](API-Reference) | JSON-RPC API documentation |
| [Getting Started](Getting-Started) | Setup and running nodes |

---

## Project Structure

```
irondag-blockchain/
├── src/
│   ├── blockchain/          # Core blockchain (blocks, transactions)
│   ├── consensus/           # GhostDAG consensus engine
│   ├── evm/                 # Ethereum Virtual Machine integration
│   ├── sharding/            # Horizontal sharding
│   ├── mining/              # BraidCore Mining
│   ├── network/             # P2P networking
│   ├── rpc/                 # JSON-RPC server
│   ├── security/            # AI-driven security & fraud detection
│   ├── account_abstraction/ # Smart contract wallets (ERC-4337)
│   ├── oracles/             # Built-in oracle network
│   ├── privacy/             # zk-SNARK privacy layer
│   ├── pqc/                 # Post-quantum cryptography
│   ├── verkle/              # Verkle trees for stateless clients
│   ├── governance/          # On-chain governance
│   ├── recurring/           # Recurring transactions
│   ├── stop_loss/           # Stop-loss orders
│   ├── storage/             # Persistent storage (sled)
│   └── ...
├── tests/                   # Integration tests
├── examples/                # Example code
└── proto/                   # gRPC protocol definitions
```

---

## Key Features

### Core Blockchain
- **GhostDAG Consensus**: DAG-based consensus for parallel block production
- **BraidCore Mining**: Three parallel mining streams (Blake3, KHeavyHash, Keccak256)
- **Horizontal Sharding**: Scale with multiple shards and cross-shard transactions

### Smart Contracts
- **Full EVM Compatibility**: Deploy Solidity contracts via SputnikVM (evm 0.41)
- **MetaMask Ready**: EIP-155 signing, standard JSON-RPC

### Advanced Features
- **Account Abstraction**: Smart contract wallets, multi-sig, social recovery
- **Privacy Layer**: zk-SNARKs for private transfers (Groth16 on BN254)
- **Post-Quantum Cryptography**: ML-KEM-768 (FIPS 203) key exchange, ML-DSA-65 (FIPS 204) signatures, SLH-DSA (FIPS 205) hash-based signatures
- **Built-in Oracles**: Native price feeds and VRF randomness
- **AI Security**: Fraud detection, risk scoring, forensic analysis

---

## Status

**Current Phase**: Development/Testnet

See [Technical Status](Technical-Status) for detailed implementation status.

---

## License

MIT License - Copyright (c) 2026 IronDAG Protocol

