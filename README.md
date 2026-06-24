# IronDAG Blockchain

<div align="center">

![IronDAG Logo](brand-assets/logos/irondag_project_logo.png)

**Post-Quantum Layer 1 Blockchain**

[![License: BUSL-1.1](https://img.shields.io/badge/License-BUSL--1.1-lightgrey.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange.svg)](https://www.rust-lang.org/)
[![EVM Compatible](https://img.shields.io/badge/EVM-Compatible-blue.svg)](https://ethereum.org/)
[![Chain ID](https://img.shields.io/badge/Chain%20ID-1338-purple.svg)](https://irondag.io)
[![Status](https://img.shields.io/badge/Status-Testnet_LIVE_@_explorer.irondag.io-green.svg)](https://explorer.irondag.io)

[Website](https://irondag.io) ? [Explorer](https://irondag.io/explorer/) ? [Whitepaper](https://irondag.io/IronDAG_WHITEPAPER.html) ? [Twitter](https://x.com/DevIronDAG) ? [Wiki](https://github.com/dev-irondag/irondag/wiki)

</div>

---

## Overview

**IronDAG** is a Layer 1 blockchain featuring:

| Feature | Description |
|---------|-------------|
| **Ticker** | IDAG (Q=Quantum, MON=IronDAG) |
| **Max Supply** | 10,000,000,000 IDAG (10B); 70% mining, 30% genesis (vested) |
| **Consensus** | GhostDAG (parallel block production) + post-quantum signatures |
| **Mining** | BraidCore: Stream A (Blake3, ~10s, 50 IDAG) + Stream B (KHeavyHash, ~5s, 25 IDAG) + Stream C (fee-only). Target GPU 45% + ASIC 35% + ZK Proving 20% at mainnet |
| **Smart Contracts** | Full EVM compatibility via SputnikVM (evm 0.41) |
| **Security** | NIST post-quantum (ML-DSA/Dilithium) |
| **Deflationary** | 50% of transaction fees burned per block |

---

## Key Features

### BraidCore Mining Architecture
Three parallel mining streams (mainnet design target) — **BraidCore** is IronDAG's multi-stream mining architecture where three parallel mining streams (A, B, C) cross-reference each other at the parent level, braiding into a single GhostDAG consensus layer:

| Stream | Target Share | Hardware | Current Status |
|--------|--------------|----------|----------------|
| **GPU** | 45% | Consumer GPUs | In development (OpenCL pending) |
| **ASIC** | 35% | ASICs | Planned for mainnet |
| **ZK Proving** | 20% | ZK proof hardware | Planned for mainnet |

**Current implementation**: Stream A (Blake3, ~10s blocks) and Stream B (KHeavyHash, ~5s blocks) are both actively mining on testnet. GPU mining via OpenCL is planned but not yet implemented.

*Testnet runs BraidCore mining (Stream A at ~10s + Stream B at ~5s). Full Kaspa-style chromatic emission applies at mainnet.*

### Technical Highlights

- **GhostDAG Consensus** - DAG-based consensus allowing parallel block production
- **EVM Compatible** - Deploy Solidity contracts, use MetaMask, ethers.js
- **SIMD-Accelerated Mining** - AVX2/SSE2 optimized for performance
- **Post-Quantum Ready** - Dilithium signatures, Kyber key exchange
- **Verkle State Proofs** - Wide 256-way branching tree with KZG polynomial commitments for O(1) stateless verification; dual commitment pattern (Keccak + KZG) bridges backward compatibility with ZK-friendly proofs
- **Account Abstraction (ERC-4337)** - Smart contract wallets with flexible signature schemes and gas sponsorship
- **Privacy Layer** - Feature-gated privacy mode with dual commitments and confidential transactions
- **Parallel EVM Execution** - Multi-threaded transaction execution with conflict detection and automatic retry
- **Desktop Wallet** - Tauri-based native desktop wallet application
- **Security Hardening** - Rate limiting, API key auth, Noise Protocol encrypted P2P
- **Horizontal Sharding** - Scale with cross-shard transactions

---

## Quick Start

### Prerequisites

- **Rust** 1.75+ ([Install](https://rustup.rs))
- **protoc** (Protocol Buffers compiler) – required to build the node: [Install protoc on Windows](docs/INSTALL_PROTOC.md)
- **Node.js** 18+ (for frontend/testing)
- **Foundry** (optional, for Solidity tests): [Install Forge on Windows](docs/INSTALL_FORGE.md)

### First-Time Setup

After cloning, run the dev setup script to enable git hooks:

```bash
# Linux/macOS
bash scripts/setup-dev.sh

# Windows (PowerShell)
.\scripts\setup-dev.ps1
```

This enables the pre-commit hook that prevents accidental IP address leaks in documentation files.

### Run a Node

```bash
# Clone the repository
git clone https://github.com/dev-irondag/irondag.git
cd irondag/irondag-blockchain

# Build and run
cargo build --release
cargo run --release --bin node -- --port 9090 --single-stream
```

### Stopping nodes

**Ctrl+C** may not stop the node in some terminals (e.g. background or certain Windows shells). To stop all IronDAG node processes and free `node.exe` for rebuilds, run from the repo root:

```powershell
.\scripts\stop_local_nodes.ps1
```

This stops processes by path (irondag) and by ports 8080, 8545, 9090, 9091, 9092, 8546. It does not kill Node.js unless it was started from a path containing "irondag".

### CLI Configuration Reference

**Network & Discovery:**
- `--port <PORT>` - P2P port (default: 8080)
- `--rpc-port <PORT>` - JSON-RPC port (default: 8545)
- `--bootstrap-peer <ADDR>` - Initial peer connection (IP:PORT)
- `--advertise <ADDR>` - Public address for P2P handshake
- `--max-peers <N>` - Peer connection limit (default: 50)

**Mining:**
- `--miner-address <HEX>` - Reward address (40 hex chars, 0x prefix optional)
- `--disable-mining` - Start node without mining
- `--single-stream` - Mine with single stream only
- `--enable-stream-c` - Enable high-CPU Stream C mining

**Security & TLS:**
- `--tls-cert <PATH>` - TLS certificate for HTTPS RPC
- `--tls-key <PATH>` - TLS private key file
- `--data-dir <PATH>` - Blockchain data directory

**Chain Configuration:**
- `--chain-id <ID>` - EIP-155 chain ID (default: 1338)
- `--genesis-file <PATH>` - Custom genesis allocations (JSON)

**Privacy (requires `privacy` feature):**
- `--privacy-proving-key <PATH>` - ZK proving key
- `--privacy-verifying-key <PATH>` - ZK verifying key

**See `irondag-node --help` for the complete list.**

### Connect MetaMask

| Setting | Value |
|---------|-------|
| Network Name | IronDAG Testnet |
| RPC URL | `http://localhost:8545` |
| Chain ID | `1338` |
| Currency Symbol | `IDAG` |

---

## Project Structure

```
irondag/
??? irondag-blockchain/     # Rust blockchain implementation
?   ??? src/
?   ?   ??? blockchain/         # Core blockchain logic
?   ?   ??? consensus/          # GhostDAG consensus
?   ?   ??? evm/                # EVM integration (SputnikVM)
?   ?   ??? mining/             # BraidCore mining
?   ?   ??? network/            # P2P networking
?   ?   ??? sharding/           # Horizontal sharding
?   ?   ??? security/           # Rule-based fraud detection
?   ?   ??? rpc/                # JSON-RPC API
?   ??? tests/                  # Integration tests
??? irondag-desktop/        # Tauri desktop wallet
??? irondag-explorer/       # Block explorer frontend
??? contracts/                  # Solidity smart contracts
??? docs/                       # Documentation
```

---

## Documentation

| Document | Description |
|----------|-------------|
| [Whitepaper](IronDAG_WHITEPAPER.md) | Technical vision and architecture |
| [Tokenomics](TOKENOMICS.md) | IDAG economics: 10B supply, 70/30 split, BraidCore, fee burn, IKO summary |
| [API Reference](JSON_RPC_API_GUIDE.md) | JSON-RPC API documentation |
| [Node Setup](NODE_QUICK_START.md) | Running a node guide |
| [MetaMask Guide](METAMASK_CONNECTION_GUIDE.md) | Wallet connection |
| [Wiki](https://github.com/dev-irondag/irondag/wiki) | Complete documentation |

**Reference (Word, IKO / biz):**  
- **IDAG_Tokenomics.docx** — Full tokenomics, vesting, milestones (authoritative for IKO).  
- **IDAG_One_Pager.docx** — One-pager: differentiators, IKO structure, use of proceeds.  
- **IDAG_Technical_Overview.docx** — Technical overview.  
- **IDAG_Roadmap.docx** — Roadmap and phases.

---

## Testnet Deployment (Apr 2026)

A live two-node testnet is publicly accessible with both Stream A and Stream B actively mining:

| Component | Details |
|-----------|---------|
| **Nodes** | Primary miner node + Frankfurt sync-only node |
| **Mining** | Stream A (Blake3, ~10s blocks) + Stream B (KHeavyHash, ~5s blocks) both active |
| **Explorer** | https://explorer.irondag.io |
| **Faucet** | Enabled — mints 10 IDAG per request |
| **Block Height** | 150+, 3 connected peers |
| **P2P** | QUIC transport with Kyber post-quantum key exchange |

---

## Current Status

| Component | Status | Details |
|-----------|--------|---------|
| Core Blockchain | ✅ Operational | Full GhostDAG + BraidCore |
| Mining | ✅ Working | Real PoW with Blake3/KHeavyHash, difficulty adjustment, BraidCore parallel mining |
| Stream B Mining | ✅ Complete | KHeavyHash algorithm active on testnet with ~5s block time |
| Consensus | ✅ Complete | GhostDAG checkpoint save/load via sled |
| EVM (SputnikVM 0.41) | ✅ Complete | Full Ethereum compatibility verified |
| Parallel EVM Execution | ✅ Complete | Multi-threaded transaction execution with conflict detection and automatic retry |
| Verkle State Proofs | ✅ Complete | Wide 256-way branching tree with KZG polynomial commitments for O(1) stateless verification |
| P2P Network | ✅ Multi-node verified | Peer exchange, bootstrap discovery, advertise detection |
| P2P Encryption | ✅ Noise Protocol XX (snow crate) | Encrypted peer connections with --no-noise fallback |
| P2P Hardening | ✅ Complete | All 6 security items implemented: Noise Protocol, message auth, rate limiting, API key auth, TLS, peer reputation |
| Message Authentication | ✅ Ed25519 signatures | All P2P messages signed and verified on receive |
| JSON-RPC API | ✅ Ethereum-compatible | 30+ methods, TLS support, rate limiting |
| RPC Security | ✅ Default-on API key auth | CORS origin whitelisting, --rpc-no-auth for dev. **Note**: Known CORS wildcard issue on .31 (needs restriction) |
| gRPC API | ✅ Complete | v1 (14 methods, hex-encoded) + v2 (16 methods, binary-optimized 3.3x faster) |
| MetaMask | ✅ Compatible | Native Web3 integration |
| Sharding | ✅ Phase 1-6 complete | Cross-shard TX routing, receipt processing, unified stream sources |
| Account Abstraction | ✅ Complete | ERC-4337 smart contract wallets with flexible signature schemes and gas sponsorship |
| ZK Proving (Stream C) | ⚠️ Experimental | Groth16 circuit integrated; soft verification active but not enforced |
| Privacy (Feature-Flagged) | ✅ Complete | zk-SNARK proof verification; dual commitments, confidential transactions; requires --privacy-* flags |
| Storage | ✅ Sled with hot-cache | Production tuning (256MB cache, 1s flush), Criterion benchmarks (7x headroom) |
| Desktop Wallet | ✅ Complete | v0.2.0: encrypted keystore, auto-updater, accessibility |
| Block Explorer | ✅ Deployed | TX status, pagination, address history, DAG visualization |
| Security Hardening | ✅ Complete | P2P hardening complete (all 6 items); external audit pending |
| Gossip Protocol | ✅ Optimized | Seen-set dedup, random fanout relay, compact blocks, latency-aware routing |
| Network Resilience | ✅ Hardened | Per-peer rate limiting, partition detection, peer reputation scoring |
| Block Reward Halving | ✅ Complete | 4-year halving interval with era-based reward calculation |
| Distributed Testnet | ✅ Tooling ready | Bash/PowerShell scripts, Docker Compose, genesis template, monitoring |
| License | BUSL 1.1 | Converts to Apache 2.0 on April 1, 2030 |

> **Important**: This is **alpha/testnet software**, not production-ready. Core PoW mining is complete; consensus integration is in progress. See [TECHNICAL_STATUS.md](TECHNICAL_STATUS.md) for current status.

### Known Limitations

| Item | Status | Workaround |
|------|--------|------------|
| gRPC Service Methods | ✅ All implemented | v1 and v2 fully operational |
| Phase 6 Cross-Shard TX | ✅ Complete | Execution bridge, unified streams, e2e tested |
| Configuration Files | CLI flags only | Use `--help` for full reference |
| External Security Audit | Not yet conducted | Internal audit complete |

---

## Tokenomics Summary

| Metric | Value |
|--------|-------|
| **Max Supply** | 10,000,000,000 IDAG (10 billion) |
| **Mining** | 70% (~30-year emission; Kaspa-style smooth decay) |
| **Genesis (vested)** | 30% (Ecosystem 8%, IKO 7%, Team 5%, Dev 4%, Liquidity 3%, Marketing 2%, Treasury 1%) |
| **TGE Circulating** | 1.4% of supply |
| **Fee Burn** | 50% of tx fees burned per block |
| **IKO** | Polygon ERC-20 wrapper; mainnet swap at launch (see [IDAG_One_Pager](IDAG_One_Pager.docx)) |

See [TOKENOMICS.md](TOKENOMICS.md) for full allocations, vesting, and design rationale. Canonical IKO details: **IDAG_Tokenomics.docx**.

---

## Contributing

We welcome contributions! Please see:

- [Contributing Guide](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

---

## Community

- **Website**: [irondag.io](https://irondag.io)
- **Twitter/X**: [@DevIronDAG](https://x.com/DevIronDAG)
- **GitHub**: [dev-irondag/irondag](https://github.com/dev-irondag/irondag)
- **Explorer**: [irondag.io/explorer](https://irondag.io/explorer/)

---

## Binary Verification

All release binaries are signed for integrity verification:

**Windows**: Authenticode signed with SHA-256. Right-click the `.exe` or `.msi` → Properties → Digital Signatures to verify.

**macOS**: Signed with Apple Developer ID and notarized. Gatekeeper verifies automatically. To manually check: `codesign --verify --verbose /path/to/IronDAG.app`

**Linux**: GPG detached signatures (`.asc`) are provided alongside `.deb` and `.AppImage` files. To verify:
```
gpg --verify irondag_x.y.z_amd64.deb.asc irondag_x.y.z_amd64.deb
```

Note: Alpha releases use development certificates. SmartScreen or Gatekeeper warnings are expected — click "More info" → "Run anyway" on Windows, or right-click → Open on macOS.

---

## License

Business Source License 1.1 - see [LICENSE](LICENSE) for details.

The Licensed Work is available under the Business Source License 1.1. On the
Change Date (April 1, 2030), or the fourth anniversary of the first publicly
available distribution of this version, the license will convert to Apache
License, Version 2.0.

**Copyright © 2024-2026 IronDAG Project**

---

<div align="center">

**Built with Rust** 🦀 **| EVM Compatible** ⚡ **| Post-Quantum Ready** 🔐

</div>

---

## Created By

**David Cooper**  
CCIE #11567
