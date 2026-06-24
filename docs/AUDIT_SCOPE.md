# IronDAG Security Audit Scope

## Project Overview
- **Brief**: Layer 1 blockchain in Rust with GhostDAG consensus and TriStream mining
- **Repository**: github.com/dev-irondag/irondag
- **Branch**: feature/alpha-hardening
- **Language**: Rust 1.75+
- **Total lines of code**: ~49,893 (src/ directory)

## Critical Components (Priority Order)

### 1. Consensus: GhostDAG (HIGH)
- **File**: `src/consensus/mod.rs`
- **Lines of code**: 767
- **What to audit**: Blue/red block classification, ordering correctness, finality rules
- **Known risks**: 
  - Custom implementation without formal verification
  - Incremental blue set update algorithm correctness
  - Finality depth calculation and checkpoint pruning
- **Key functions**:
  - `add_block()` - Main entry point for block addition
  - `update_blue_set()` - Full BFS blue score calculation
  - `update_blue_set_incremental()` - Optimized incremental update
  - `get_finalized_block_hash()` - Finality determination

### 2. Mining: TriStream (HIGH)
- **File**: `src/mining.rs`
- **Lines of code**: 2,901
- **What to audit**: 
  - Block reward calculation (`get_block_reward`)
  - Halving schedule correctness (`HALVING_INTERVAL`)
  - Fee burn implementation (50% - currently NOT implemented)
  - Block number allocation across streams
  - Cross-stream fairness mechanisms
- **Known risks**: 
  - Stream C flooding potential (documented in `docs/TRISTREAM_GAME_THEORY.md`)
  - `BLOCK_PROCESSING_LOCK` serializes all block commits
  - `BlockNumberAllocator` free-list recycling starvation vector
- **Constants**:
  - `STREAM_A_REWARD`: 50 IDAG
  - `STREAM_B_REWARD`: 25 IDAG
  - `STREAM_C_REWARD`: 0 (fee-only)
  - `HALVING_INTERVAL`: 12,614,400 blocks (~4 years)

### 3. P2P Networking (HIGH)
- **File**: `src/network.rs`
- **Lines of code**: 3,336
- **What to audit**: 
  - Noise Protocol handshake (see `src/noise.rs`)
  - Peer exchange and discovery
  - Ban logic and reputation scoring
  - Message authentication
  - Rate limiting (`MAX_MESSAGES_PER_MINUTE: 300`)
- **Known risks**: 
  - Eclipse attack surface
  - Peer amplification attacks
  - Partition detection bypass
- **Key structures**:
  - `PeerScore` - Reputation and quality metrics
  - Ban duration: 24 hours (`BAN_DURATION_SECS: 86400`)

### 4. RPC Layer (MEDIUM)
- **File**: `src/rpc.rs`
- **Lines of code**: 8,771
- **What to audit**: 
  - Authentication and API key validation
  - Rate limiting implementation
  - Input validation on all methods
  - Batch request limits (`RPC_MAX_BATCH_SIZE: 100`)
  - Response size limits (`RPC_MAX_RESPONSE_SIZE: 10MB`)
- **Known risks**: 
  - Resource exhaustion via large requests
  - Information leakage through error messages
  - Lock acquisition timeouts during mining bursts
- **Timeout configuration**:
  - Write operations: 30 seconds
  - Read operations: 5 seconds
  - Lock timeout: 10 seconds

### 5. EVM Execution (MEDIUM)
- **File**: `src/evm/mod.rs`
- **Lines of code**: 962
- **What to audit**: 
  - SputnikVM integration correctness
  - Gas metering accuracy
  - Precompile correctness
  - State persistence via `ApplyBackend`
  - Contract address generation (CREATE opcode)
- **Known risks**: 
  - Delegated to upstream SputnikVM — focus on integration layer
  - State inconsistency between memory and persistent storage
- **Key components**:
  - `SputnikBackend` - Implements `Backend` and `ApplyBackend` traits
  - `EvmTransactionExecutor` - Main execution engine

### 6. Storage (LOW)
- **File**: `src/storage.rs`
- **Lines of code**: 1,934
- **What to audit**: 
  - Data integrity guarantees
  - Migration framework (`DB_VERSION_KEY`, `CURRENT_DB_VERSION: 1`)
  - Pruning safety
  - Compression correctness (zstd)
- **Key features**:
  - Binary key encoding with prefix bytes
  - Batch operations for atomic writes
  - Zstd compression for values > 512 bytes

### 7. Cryptography (MEDIUM)
- **Files**: `src/pqc/`, `src/pow.rs`, `src/blockchain/block.rs`
- **What to audit**: 
  - Dilithium3/SPHINCS+ usage for post-quantum signatures
  - Kyber key exchange
  - Transaction signing (ECDSA + EIP-155)
  - Blake3 (Stream A) and KHeavyHash (Stream B) PoW
- **Known risks**: 
  - Correct usage of post-quantum primitives
  - SIMD optimizations in `pow.rs` (AVX2, SSE2)

## Out of Scope
- Frontend/explorer code
- Documentation files (except as context)
- Test code (unless testing critical paths)
- Sharding (deferred, not mainnet-critical)
- Privacy/ZK features (feature-gated, experimental)

## Known Issues (Already Documented)
- See `docs/TRISTREAM_GAME_THEORY.md` for cross-stream attack vectors
- See `docs/FEE_MARKET_ANALYSIS.md` for economic risks
- See `docs/PHASED_LAUNCH_PLAN.md` for launch risk assessment

---

## Threat Model

### 1. Consensus Attacks

#### Double-Spend
- **Description**: Attacker attempts to spend same funds twice by creating conflicting blocks
- **Current mitigation**: GhostDAG blue set selection prioritizes highest blue score chain
- **Residual risk**: MEDIUM - No formal verification of consensus algorithm

#### Selfish Mining
- **Description**: Miner withholds blocks to gain unfair advantage
- **Current mitigation**: TriStream multi-stream design reduces single-point advantage
- **Residual risk**: MEDIUM - Block reward calculation may incentivize withholding

#### Block Withholding
- **Description**: Miner discovers block but delays broadcast
- **Current mitigation**: Time-based penalties not implemented; relies on competitive mining
- **Residual risk**: HIGH - No explicit mitigation in current code

### 2. Network Attacks

#### Eclipse Attack
- **Description**: Attacker isolates victim node by controlling all peers
- **Current mitigation**: 
  - Peer scoring and reputation system
  - Ban logic for misbehaving peers
  - Maximum peers limit
- **Residual risk**: MEDIUM - Limited peer diversity in bootstrap

#### Sybil Attack
- **Description**: Attacker creates many fake identities
- **Current mitigation**: Economic cost of mining (PoW) limits Sybil creation
- **Residual risk**: LOW - PoW provides Sybil resistance

#### DoS (Denial of Service)
- **Description**: Overwhelm node with invalid messages/blocks
- **Current mitigation**:
  - Rate limiting: 300 messages/minute per peer
  - Ban after 5 invalid messages
  - Ban after 3 invalid blocks
  - Ban after 50 invalid transactions
- **Residual risk**: MEDIUM - Stream C low-cost blocks could flood network

#### Network Partition
- **Description**: Split network into isolated segments
- **Current mitigation**: Partition detection (60s no blocks = potential partition)
- **Residual risk**: MEDIUM - Automatic recovery not fully implemented

### 3. Economic Attacks

#### Fee Manipulation
- **Description**: Attacker manipulates fee market to censor transactions
- **Current mitigation**: Fee-priority ordering (highest fee first)
- **Residual risk**: HIGH - No minimum fee enforced; zero-fee transactions valid

#### Reward Gaming
- **Description**: Exploit block reward calculation for extra rewards
- **Current mitigation**: Fixed reward schedule with halving
- **Residual risk**: MEDIUM - Halving boundary conditions need verification

#### Stream Starvation
- **Description**: One stream monopolizes block production
- **Current mitigation**: Per-stream pool caps (60%/30%/10%)
- **Residual risk**: MEDIUM - Stream C (fee-only) may be economically unviable

### 4. RPC Attacks

#### Authentication Bypass
- **Description**: Gain unauthorized access to RPC endpoints
- **Current mitigation**: API key authentication on sensitive methods
- **Residual risk**: LOW - Public methods explicitly whitelisted

#### Resource Exhaustion
- **Description**: Exhaust node resources via RPC calls
- **Current mitigation**:
  - Response size limit: 10MB
  - Batch size limit: 100 requests
  - Timeout: 5s read, 30s write
- **Residual risk**: MEDIUM - Complex queries may still cause issues

#### Information Leakage
- **Description**: Extract sensitive node information
- **Current mitigation**: Error messages sanitized in production
- **Residual risk**: LOW - No sensitive data in RPC responses

### 5. Cryptographic Attacks

#### Signature Forgery
- **Description**: Forge transaction signatures
- **Current mitigation**: ECDSA with secp256k1; Dilithium3 for PQ accounts
- **Residual risk**: LOW - Industry-standard cryptography

#### Replay Attack
- **Description**: Replay transaction on different chain/fork
- **Current mitigation**: EIP-155 chain ID enforcement
- **Residual risk**: LOW - Chain ID included in signature

#### Key Compromise
- **Description**: Attacker gains access to private keys
- **Current mitigation**: Post-quantum signatures for high-value accounts
- **Residual risk**: MEDIUM - Key management outside scope

### 6. State Attacks

#### Storage Corruption
- **Description**: Corrupt database to alter balances/state
- **Current mitigation**: Sled database integrity checks
- **Residual risk**: MEDIUM - No Merkle proofs for state verification yet

#### State Inconsistency
- **Description**: Create inconsistent state between EVM and native
- **Current mitigation**: Unified state store via `StateStore`
- **Residual risk**: MEDIUM - EVM and native state separation

#### Rollback Attack
- **Description**: Force chain rollback for double-spend
- **Current mitigation**: Finality depth (100 blocks)
- **Residual risk**: LOW - Deep finality makes rollback expensive

---

## Code Quality Findings

### Dead Code Annotations (`#[allow(dead_code)]`)
Found 25+ occurrences across codebase. Key locations requiring review:
- `src/bin/node.rs:1082,1121` - Node binary
- `src/blockchain/mod.rs:55,117,124,126,128,130,134,138` - Core blockchain
- `src/sharding/mod.rs:123,132,136,535` - Sharding module
- `src/network.rs:376,381` - Network layer
- `src/rpc.rs:395` - RPC layer

### Unsafe Code Blocks
Found 6 occurrences, all in `src/pow.rs` and `src/node/mod.rs`:
- `src/pow.rs:172,177,218,243` - SIMD optimizations (AVX2/SSE2)
  - **Justification**: Performance-critical XOR operations with feature detection
  - **Safety**: Bounds checked before unsafe blocks
- `src/node/mod.rs:576` - Privacy manager initialization
  - **Justification**: Raw pointer manipulation for RPC server
  - **Risk**: Should be refactored to safe Rust

### Panicking Code Paths
Found 25+ occurrences of `unwrap()`, `expect()`, `panic!()`:

**Critical (should be reviewed)**:
- `src/rpc.rs:475,565,654,743` - `expect()` on blockchain lock acquisition
  - **Risk**: RPC thread panic if lock poisoned
  - **Recommendation**: Convert to `?` propagation

**Test code (acceptable)**:
- `src/storage.rs:1683-1792` - Test module uses `unwrap()` extensively

**Potentially safe**:
- `src/node/mod.rs:346` - Socket address parsing with fallback
- `src/node/mod.rs:488-489` - Key path conversion (validated earlier)

### Debug Artifacts
- `println!` found in `src/bin/node.rs:107-109` (banner display - acceptable)
- `println!` found in `src/pow.rs:919-1001` (mining diagnostics - should use tracing)
- Commented `println!` in `src/node/mod.rs:1643` and `src/mining.rs:1717`
- No `dbg!` calls found

### Commented-Out Code
Found in:
- `src/consensus/mod.rs:7-8` - Test module imports commented
- `src/rpc.rs` - Multiple inline code comments (acceptable documentation)

---

## Build and Run Instructions

### Prerequisites
- Rust 1.75+ (install via rustup)
- Protocol Buffers compiler (protoc)
- 4GB+ RAM recommended

### Build
```bash
cd irondag-blockchain
cargo build --release
```

### Run Tests
```bash
# Run all tests
cargo test --release

# Run with all features
cargo test --release --all-features
```

### Start a Node
```bash
# Run the node binary
cargo run --release --bin node

# With custom data directory
cargo run --release --bin node -- --data-dir /path/to/data
```

### Feature Flags
- `privacy` - Enable ZK privacy features
- `sharding` - Enable sharding (experimental)

---

## Audit Checklist

### Pre-Audit Verification
- [ ] Code compiles with `cargo check --release`
- [ ] All tests pass: `cargo test --release`
- [ ] No Clippy warnings: `cargo clippy --release -- -D warnings`
- [ ] Documentation builds: `cargo doc --no-deps`

### Critical Path Review
- [ ] GhostDAG consensus algorithm correctness
- [ ] Block reward calculation and halving
- [ ] Transaction signature verification
- [ ] P2P message authentication
- [ ] RPC authentication bypass attempts

### Security Focus Areas
- [ ] All `unsafe` blocks justified and bounded
- [ ] No `unwrap()`/`expect()` in production paths
- [ ] Rate limiting effective against DoS
- [ ] Input validation on all external entry points
- [ ] State consistency between modules

---

## Document History

| Date | Version | Changes |
|------|---------|---------|
| 2026-04-05 | 1.0 | Initial audit scope document |

---

*This document was generated for the external security audit of the IronDAG blockchain. For questions, contact the development team.*
