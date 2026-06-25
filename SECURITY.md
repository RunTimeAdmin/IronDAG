# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| 0.3.x-testnet | Testnet (Current) |
| < 1.0   | :x:                |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them via one of the following methods:

### Email Security Team
- **Email**: security@irondag.io
- **Subject**: "Security Vulnerability Report"
- **Response Time**: We aim to respond within 48 hours

### Security Advisory Format
Please include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)
- Your contact information

### What to Expect
- **Acknowledgment**: Within 48 hours
- **Initial Assessment**: Within 7 days
- **Fix Timeline**: Depends on severity
- **Disclosure**: After fix is deployed (coordinated disclosure)

### Severity Levels
- **Critical**: Remote code execution, fund loss, network compromise
- **High**: Significant impact on security or functionality
- **Medium**: Moderate impact with workarounds
- **Low**: Minor issues with minimal impact

### Responsible Disclosure
We follow responsible disclosure practices:
- We will credit you for reporting (if desired)
- We will work with you to understand and resolve the issue
- We will coordinate public disclosure after a fix is available

### Bug Bounty Program
**Status**: Coming at mainnet launch

We plan to launch a bug bounty program after mainnet launch. Rewards will be based on severity and impact.

---

## Cryptographic Design

### NIST Post-Quantum Standards

IronDAG implements all three finalized NIST post-quantum cryptography standards:

| Standard | Algorithm | Use |
|----------|-----------|-----|
| FIPS 203 | ML-KEM-768 | P2P session key establishment |
| FIPS 204 | ML-DSA-65 | Block signing, transaction authentication |
| FIPS 205 | SLH-DSA-SHA2-128f | Alternative account type (hash-only trust) |

**ML-KEM-768 (FIPS 203):** Implemented via the `ml-kem` Rust crate — pure Rust, no C bindings, works identically on Windows, Linux, and macOS. Used exclusively in the P2P handshake for forward-secret session key establishment. Not involved in block or transaction data.

**ML-DSA-65 (FIPS 204):** Implemented via `pqcrypto-mldsa`. The recommended default account type (`PqAccount::new_dilithium3()`). All block producer signatures use ML-DSA-65 when a PQ account is configured.

**SLH-DSA-SHA2-128f (FIPS 205):** Implemented via `pqcrypto-sphincsplus`. Available as `PqAccount::new_sphincsplus()` for operators who prefer security assumptions based solely on hash function collision resistance, independent of lattice problem hardness.

### Crypto-Agility

The protocol is designed so cryptographic algorithms can be upgraded across the network without a disruptive hard fork:

- **`hash_version: u8` in `BlockHeader`** — The hash algorithm identifier is included as an input to the PoW hash itself. Current value: `0x01` (BLAKE3). When a governance action schedules a new hash algorithm at a future block height, all nodes switch deterministically and the version byte changes, making the upgrade verifiable from block data alone.

- **`capabilities: u32` bitmask in P2P handshake** — Each node advertises which algorithms it supports (`CAP_ML_KEM_768`, `CAP_ML_DSA_65`, `CAP_SLH_DSA`, `CAP_BLAKE3_POW`, `CAP_B3MEMHASH`, `CAP_COMPACT_BLOCKS`). This enables the network to observe capability adoption before enforcing a new minimum, preventing a hard split.

### Classical Security Layer

- **ECDSA secp256k1** (EIP-155): All Ethereum-compatible transaction signing. Chain ID 11567 enforced for replay protection.
- **AES-256-GCM**: Session encryption after ML-KEM key establishment.
- **BLAKE3**: Block and Merkle hashing; B3MemHash (memory-hard BLAKE3 variant) for Stream B PoW.

### Scope for Bug Reports

The following are in scope for security reports:
- Any bypass of ML-KEM handshake authentication
- Signature forgery in ML-DSA-65 or SLH-DSA paths
- Chain ID replay attacks
- RPC authentication bypass
- PoW verification bypass
- Consensus rule violations leading to chain splits

---

**Thank you for helping keep IronDAG secure!**

---

Copyright (c) 2024-2025 IronDAG Contributors  
Licensed under the BUSL-1.1 License
