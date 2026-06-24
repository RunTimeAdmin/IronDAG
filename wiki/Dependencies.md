# Dependencies

Complete list of external crates and libraries used by IronDAG.

---

## Runtime Dependencies

### Core Async Runtime

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.35 | Async runtime with full features |
| `futures` | 0.3 | Future combinators and utilities |
| `async-trait` | 0.1 | Async trait support |

### Serialization

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | 1.0 | Serialization framework |
| `serde_json` | 1.0 | JSON serialization |
| `bincode` | 1.3 | Binary serialization |
| `toml` | 0.8 | TOML configuration files |

### Cryptography

| Crate | Version | Purpose |
|-------|---------|---------|
| `sha3` | 0.10 | Keccak256 hashing |
| `blake3` | 1.5 | Blake3 hashing |
| `ed25519-dalek` | 2.1 | Ed25519 signatures |
| `k256` | 0.13 | secp256k1 ECDSA |
| `rlp` | 0.5 | RLP encoding (Ethereum) |
| `rand` | 0.8 | Random number generation |
| `rand_core` | 0.6 | RNG traits |
| `aes-gcm` | 0.10 | AES-GCM encryption |

### Post-Quantum Cryptography

| Crate | Version | Purpose |
|-------|---------|---------|
| `pqcrypto-traits` | 0.3.5 | PQC traits |
| `pqcrypto-dilithium` | 0.5 | Dilithium signatures |

> Note: `pqcrypto-kyber` and `pqcrypto-sphincsplus` temporarily disabled due to Windows/MSVC build issues.

### EVM Integration

| Crate | Version | Purpose |
|-------|---------|---------|
| `evm` | 0.41 | SputnikVM - Rust EVM implementation (Berlin/Shanghai config) |

### Zero-Knowledge Proofs (arkworks)

| Crate | Version | Purpose |
|-------|---------|---------|
| `ark-bn254` | 0.4 | BN254 curve |
| `ark-groth16` | 0.4 | Groth16 proof system |
| `ark-relations` | 0.4 | Constraint relations |
| `ark-ec` | 0.4 | Elliptic curves |
| `ark-ff` | 0.4 | Finite fields |
| `ark-std` | 0.4 | Standard utilities |
| `ark-poly` | 0.4 | Polynomials |
| `ark-serialize` | 0.4 | Serialization |
| `ark-snark` | 0.4 | SNARK traits |

### Storage

| Crate | Version | Purpose |
|-------|---------|---------|
| `sled` | 0.34 | Embedded database |
| `tempfile` | 3.8 | Temporary files for testing |

### Concurrency

| Crate | Version | Purpose |
|-------|---------|---------|
| `dashmap` | 6.0 | Lock-free concurrent HashMap |
| `crossbeam-queue` | 0.3 | Concurrent queues |

### Networking (gRPC)

| Crate | Version | Purpose |
|-------|---------|---------|
| `tonic` | 0.11 | gRPC framework |
| `prost` | 0.12 | Protocol Buffers |
| `prost-types` | 0.12 | Protobuf well-known types |
| `hyper` | 1.0 | HTTP/2 support |
| `hyper-util` | 0.1 | Hyper utilities |
| `h2` | 0.4 | HTTP/2 protocol |

### Observability

| Crate | Version | Purpose |
|-------|---------|---------|
| `tracing` | 0.1 | Structured logging |
| `tracing-subscriber` | 0.3 | Log subscribers |
| `prometheus` | 0.13 | Metrics collection |

### Error Handling

| Crate | Version | Purpose |
|-------|---------|---------|
| `anyhow` | 1.0 | Flexible error handling |
| `thiserror` | 1.0 | Error derive macros |

### Utilities

| Crate | Version | Purpose |
|-------|---------|---------|
| `hex` | 0.4 | Hex encoding/decoding |
| `chrono` | 0.4 | Date/time handling |
| `config` | 0.14 | Configuration loading |

---

## Build Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `prost-build` | 0.12 | Protobuf code generation |
| `tonic-build` | 0.11 | gRPC code generation |

---

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio-test` | 0.4 | Async test utilities |

---

## Dependency Tree

```
irondag-blockchain
├── tokio (async runtime)
│   └── full features
├── evm (SputnikVM 0.41)
│   ├── core
│   └── runtime
├── ark-* (zk-SNARKs)
│   ├── ark-bn254
│   ├── ark-groth16
│   └── ...
├── sled (storage)
├── dashmap (concurrency)
├── tonic (gRPC)
│   ├── prost
│   └── hyper
├── k256 (ECDSA)
│   └── secp256k1
├── pqcrypto-* (PQC)
│   └── dilithium
└── tracing (logging)
```

---

## Feature Flags

### Optional Features

```toml
[features]
reqwest = []  # HTTP client support
```

---

## System Requirements

### Rust Version

- **Minimum**: Rust 1.92.0
- **Edition**: 2021

### Build Tools

| Platform | Requirements |
|----------|--------------|
| Windows | Visual Studio Build Tools 2022 with C++ tools |
| Linux | clang, cmake, pkg-config |
| macOS | Xcode Command Line Tools |

### External Tools

| Tool | Purpose |
|------|---------|
| `protoc` | Protocol Buffers compiler (for gRPC) |

---

## Cargo.toml

```toml
[package]
name = "irondag-blockchain"
version = "0.1.0"
edition = "2021"
authors = ["IronDAG Team"]
description = "IronDAG Protocol (IDAG) - High-performance sharded blockchain"
license = "MIT OR Apache-2.0"

[dependencies]
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha3 = "0.10"
blake3 = "1.5"
hex = "0.4"
anyhow = "1.0"
thiserror = "1.0"
async-trait = "0.1"
evm = { version = "0.41", features = ["tracing"] }
sled = "0.34"
tempfile = "3.8"
bincode = "1.3"
futures = "0.3"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
config = "0.14"
toml = "0.8"
chrono = { version = "0.4", features = ["serde"] }
prometheus = "0.13"
ed25519-dalek = { version = "2.1", features = ["rand_core"] }
k256 = { version = "0.13", features = ["ecdsa", "ecdsa-core"] }
rlp = "0.5"
rand = "0.8"
rand_core = "0.6"
dashmap = "6.0"
pqcrypto-traits = "0.3.5"
pqcrypto-dilithium = "0.5"
aes-gcm = "0.10"
crossbeam-queue = "0.3"
ark-bn254 = "0.4"
ark-groth16 = "0.4"
ark-relations = "0.4"
ark-ec = "0.4"
ark-ff = "0.4"
ark-std = "0.4"
ark-poly = "0.4"
ark-serialize = "0.4"
ark-snark = "0.4"
tonic = "0.11"
prost = "0.12"
prost-types = "0.12"
hyper = { version = "1.0", features = ["full"] }
hyper-util = { version = "0.1", features = ["full"] }
h2 = "0.4"
prost-build = "0.12"
tonic-build = "0.11"

[build-dependencies]
prost-build = "0.12"
tonic-build = "0.11"

[dev-dependencies]
tokio-test = "0.4"
```

---

## Security Considerations

### Cryptographic Libraries

All cryptographic libraries are well-audited:

| Library | Audit Status |
|---------|--------------|
| `k256` | Audited by NCC Group |
| `ed25519-dalek` | Audited by Quarkslab |
| `arkworks` | Academic peer review |
| `evm` (SputnikVM) | Production tested |
| `blake3` | IETF standardization |

### Supply Chain Security

- All dependencies from crates.io
- Cargo.lock committed for reproducible builds
- Regular dependency updates via `cargo audit`

---

## Next Steps

- [Architecture Overview](Architecture-Overview) - System design
- [Module Reference](Module-Reference) - Module documentation
- [Getting Started](Getting-Started) - Setup guide
