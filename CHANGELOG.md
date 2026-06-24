# IronDAG Blockchain - Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.x-testnet] - 2026-04-10

### Testnet LIVE Status
- **Infrastructure**: Two-node cluster operational (srv1296980 miner + srv1296981 sync)
- **Mining**: BraidCore active: Stream A (Blake3, ~10s) and Stream B (KHeavyHash, ~5s) mining. Stream C available via --enable-stream-c flag
- **Block Height**: 150+ and growing
- **P2P**: QUIC transport fully operational (TLS, gossip, sync, Kyber key exchange)
- **Explorer**: Live at https://explorer.irondag.io
- **Faucet**: Enabled, mints 10 IDAG per request via direct balance credit
- **RPC**: All standard eth_* methods + irondag_faucet + irondag_getBlocksByStream operational

### Fixed
- Gossip handshake endianness (to_le_bytes -> to_be_bytes) preventing peer sync
- Catch-up loop retry (10x with 2s delay instead of immediate give-up)
- Sync continuity filter cascade (boundary blocks at from_block now pass validation)
- TOFU identity auto-registration for QUIC-connected peers
- Difficulty oscillation (MAX_DIFFICULTY 32->28, 1.5x/0.67x damping)
- Faucet enabled for production (direct mint via set_balance)

### Known Issues
- CORS wildcard on .31 reflects any origin (needs restriction)

## [0.3.x-hotfix] - 2026-04-08 — Sync Resilience Hardening

### Fixed — Sync Stall Prevention
- **235d484** – Prevent sync stall on all-orphaned block batches (`network.rs`)
- **da30fc6** – Prevent sync stall on all-orphaned batches in `full_sync_quic`/`full_sync` (`sync.rs`)

### Fixed — Block Retention and Parent Resolution
- **cb79df4** – Retain orphaned blocks across sync batches for parent resolution (`sync.rs`)
  - Orphan pool persists across batches; parents resolved when they arrive later

### Fixed — Fork Recovery and Storage Integrity
- **16105c7** – Clear all sled data (DAG edges, account state) during chain fork resync (`storage.rs`, `blockchain/mod.rs`)
- **f669ff4** – Clear local chain when peer has significantly more blocks to prevent DAG tip contamination (`sync.rs`)

### Fixed — IBD Mining Safety
- **cb1b2b4** – Pause mining during IBD to prevent DAG tip contamination from concurrent block production (`mining.rs`, `sync.rs`, `node/mod.rs`)
  - All three BraidCore mining streams pause automatically during Initial Block Download
  - Mining resumes after IBD completes; uses `scopeguard` for safety

---

## [0.3.x] - 2026-04-03

### Added
- Noise Protocol XX encryption for P2P connections (snow 0.9)
- --no-noise CLI flag for backward compatibility
- ZK state transition circuit (Groth16/BN254) for Stream C
- ZK proving key generation binary (zk_setup)
- ZK proof generation in Stream C mining (behind `privacy` feature flag)
- ZK proof verification in block validation (soft enforcement)
- Sled production tuning with StorageConfig
- Criterion storage benchmarks
- BUSL 1.1 license (Change Date: April 1, 2030)

### Fixed
- --rpc-no-auth CLI flag restored (was broken as no-op)
- Noise peer integration with 5s handshake timeout

### Added - Testing Plan & Verification (Feb 16, 2026)
- **docs/TESTING_PLAN.md** – Feature verification (auth, rate limit, cross-shard) and integration testing plan
- **docs/IMPLEMENTATION_VERIFICATION_CODEBASE.md** – Codebase evidence for all claimed implementations
- **RPC auth/rate limit tests** – `rpc_auth_rate_limit.rs`: 6 tests for API key auth, PerIpRateLimiter
- **scripts/integration_two_nodes.ps1** – 2-node sync integration script (ports 8080/8082)
- **scripts/local_two_nodes.ps1** – Node 2 port 8081→8082 to avoid sync server conflict

### Added - Phase 6: Shard State Synchronization (Feb 16, 2026)
- **Documentation**: Phases 1–5 (cross-shard flow) now fully documented in `SHARDING_OPTIMIZATION.md` and `docs/PHASE6_COMPLETION_PLAN.md`
- **StateSync message handler** – Receives cross-shard block height notifications, updates `cross_shard_block_heights`
- **`record_shard_block_height()`** / **`get_cross_shard_block_height()`** – Track known block heights per shard
- **`broadcast_block_height()`** / **`send_state_sync()`** – Main chain broadcasts block height to all shards when block mined
- **Mining integration** – `process_blocks` notifies ShardManager of new block height when sharding enabled
- **Phase 6 completion plan** – `docs/PHASE6_COMPLETION_PLAN.md` documents remaining gaps (start_receipt_processing, tx routing, ordering, catch-up)

### Changed - Token Ticker Rebrand (Jan 31, 2026)
- **MSHW → IDAG**: Rebranded token ticker across entire codebase
  - **Q** = Quantum (Post-Quantum Cryptography differentiator)
  - **MON** = IronDAG (brand tie-in)
- Updated 52 files: Rust source, smart contracts, desktop app, explorer, documentation, wiki
- Smart contract symbol now `IDAG` in `IronDAGToken.sol`
- **NOT changed**: `MSHWSYNC` protocol magic bytes (binary network identifier - changing would break network compatibility)

### Fixed - PoW Tests (Jan 31, 2026)
- Fixed 3 failing `pow::tests` that used difficulty=100 but got clamped to MAX_DIFFICULTY=16 (dev cap)

### Added - Multi-Node Sync Fix (Jan 24, 2026)
- **Storage Clear on Resync**: `clear_for_resync()` now clears both memory AND sled storage
- **BlockStore.clear_all()**: New method to remove all blocks from persistent storage
- **GhostDAG.clear()**: Reset DAG state including genesis, blue/red sets, scores
- **HybridDagStorage.clear()**: Clear hot cache and finalized blocks
- **HTTP API Port Auto-Derivation**: `http_api_port = p2p_port + 10` when not explicitly set
- **--peer Flag**: Fixed parsing bug - now properly connects to specified peers

### Fixed - Multi-Node Sync (Jan 24, 2026)
- **Parent Hash Validation**: Old genesis in storage blocked acceptance of peer's chain
- **HTTP Port Conflict**: Auto-derive HTTP API port to prevent binding conflicts
- **Peer Connection**: --peer flag was in skip list but value was never parsed
- **Fork Detection**: Full resync now properly clears storage before accepting peer blocks

### Tested
- **Multi-Node Sync**: Node 2 synced 394 blocks from Node 1 (395 blocks)
- **Fork Detection**: Chain fork detection and full resync working correctly
- **Storage Clear**: Verified old genesis removed before accepting peer chain

### Added - Performance Optimizations (Jan 2026)
- **SIMD-Accelerated Mining**: AVX2/SSE2 XOR operations with scalar fallback for cross-platform support
- **Zero-Allocation Mining Loops**: Pre-computed header bytes, thread-local buffers
- **Parallel Mining Optimization**: Clone once per thread instead of per iteration
- **RPC Validation Helpers**: Centralized `validate_address`, `validate_hash`, `parse_address`, `parse_hash`
- **RPC Error Code Constants**: Standard JSON-RPC 2.0 error codes (-32700, -32600, etc.)
- **RPC Metrics**: Request count, duration histogram, error tracking via Prometheus
- **Sync Performance**: O(1) short ID lookup via HashMap, cached timestamp, FIFO orphan eviction, move semantics

### Changed - Phase 2.4: Smart Contract Interaction
- **eth_getStorageAt RPC Method**: Read contract storage slots by address and position
- **Database Storage Layer**: Persistent contract storage with composite keys `[address + storage_key]`
- **EVM Storage Interface**: Executor methods for reading persistent storage
- **Storage Value Handling**: 32-byte storage values returned as hex strings
- **Contract Bytecode Persistence**: Verified 634-character bytecode persistence

### Fixed
- **Storage Access**: Implemented missing database integration for contract storage
- **EVM Executor**: Added database-backed storage methods
- **RPC Integration**: Connected storage layer to JSON-RPC API
- **Duplicate Function Definitions**: Removed redundant parse_address/parse_hash in rpc.rs

### Performance
- **Mining**: SIMD XOR ~3-5x faster than scalar operations
- **Sync IBD**: Pre-allocation and O(1) lookups reduce memory churn
- **RPC**: Validation happens once at entry point, errors tracked for observability

### Documentation
- Added `PHASE2_4_COMPLETE.md` with implementation details
- Added `SANITY_CHECK_ANALYSIS.md` with pending verification items
- Updated `STATUS.md` with Phase 2.4 completion
- Updated `TECHNICAL_STATUS.md` with contract interaction status
- Updated `README.md` to reflect smart contract capabilities
- Updated `CHANGELOG.md` with optimization commits

### Next Priorities
- ✅ **COMPLETED**: Contract interaction testing (read operations)
- ✅ **COMPLETED**: Performance optimizations (SIMD, RPC, sync)
- ✅ **COMPLETED**: Multi-node synchronization (storage clear, fork detection)
- ✅ **COMPLETED**: Testing plan and RPC auth/rate-limit tests
- ⚠️ **PENDING**: Feature verification and integration testing (docs/TESTING_PLAN.md)
- ⚠️ **PENDING**: Storage write persistence verification (sanity check)
- ⚠️ **PENDING**: ERC-20 token deployment and testing
- **NEXT**: Phase 6 sharding completion: start_receipt_processing, tx routing
- Production mining configuration (remove dev mode difficulty override)

---

## [0.2.0] - 2026-01-15

### Added - Phase 2.2: MetaMask/Web3 Compatibility
- **EIP-155 Transaction Hashing**: Full support for chain ID replay protection
- **ECDSA Signature Verification**: Support for both legacy (v=27/28) and EIP-155 formats
- **Smart Contract Deployment**: Verified working via ethers.js and MetaMask
- **HTTP Buffer Size**: Increased to 1MB to support large contract deployment payloads
- **JSON-RPC Batch Requests**: Full support for ethers.js batch request format
- **Connection Stability**: Added `Connection: close` header to prevent ECONNRESET errors
- **Transaction Hash Matching**: Transaction hashes now match MetaMask/ethers.js expectations exactly

### Fixed
- **Mining Difficulty**: Forced difficulty to 1 for instant dev block generation (commit d2ae747)
- **Block Validation**: Correct hash algorithm selection per stream type (Blake3/KHeavyHash/Keccak256) (commit d2ae747)
- **Transaction Pool**: Re-add transactions to pool if block rejected to prevent loss (commit d2ae747)
- **RPC Methods**: Improved eth_getBlockByNumber, eth_sendRawTransaction, eth_getTransactionReceipt (commit bfcd41e)
- **HTTP Payload Handling**: Fixed ECONNRESET for contract deployments (commit 6e523d4)

### Documentation
- Updated `TECHNICAL_STATUS.md` with Phase 2.2 completion details
- Updated `NEXT_PRIORITIES.md` with new priorities and completed achievements
- Updated `README.md` to reflect Web3 compatibility
- Updated `STATUS.md` with Jan 15 achievements

### Technical Details
- **Commit bfcd41e**: EIP-155 transaction hashing and signature verification fixes
- **Commit d2ae747**: Mining & validation fixes for instant dev blocks
- **Commit 6e523d4**: Keep-Alive fixes for contract deployment
- **Commit b5a87f9**: Documentation updates

---

## [0.1.0] - 2026-01-14

### Added - Phase 2.1: Fine-Grained Locking Architecture
- **Lock-Free Account Storage**: Implemented DashMap for unlimited concurrent reads
- **Separate Lock Domains**: Split monolithic blockchain lock into independent domains
- **Async/Sync Safety**: Added tokio::task::block_in_place wrappers
- **RPC Performance**: Achieved <100ms response time (was 30+ seconds)
- **Zero Lock Contention**: Mining, RPC, stats, and desktop app all concurrent

### Changed
- Refactored blockchain state management for better concurrency
- Progressive testing approach (RPC-only → Stats → Mining → Full system)

### Fixed
- RPC timeout issues during mining operations
- Desktop app hanging/freezing
- Lock contention between concurrent operations

### Performance
- **Before**: 30+ second RPC timeouts, complete blocking during mining
- **After**: <100ms RPC response, zero contention, unlimited concurrent operations
- **Tested**: 1700+ blocks mined with active transaction processing

### Documentation
- Added `PHASE2_FINE_GRAINED_LOCKING_COMPLETE.md` with complete architecture details
- Updated `TECHNICAL_STATUS.md` with Phase 2.1 results
- Updated `STATUS.md` with performance metrics

---

## [0.0.9] - 2026-01-13

### Added
- Release packaging scripts for Windows node distribution
- Desktop application installers (MSI and NSIS)
- Quick start guide for non-technical users
- `start_node.bat` launcher for easy node startup

### Documentation
- Created `QUICK_START.md` for user-friendly getting started guide
- Created `RELEASE_READY.md` with distribution checklist
- Added `release/README.txt` for node package instructions

---

## [0.0.8] - 2026-01-12

### Added
- MetaMask connectivity testing and verification
- EVM integration for smart contract support
- JSON-RPC API compatibility with Ethereum tools
- Chain ID configuration (default: 0x53a / 1338)

### Fixed
- net_version parse error handling
- MetaMask RPC method compatibility

### Documentation
- Created `METAMASK_CONNECTION_GUIDE.md`
- Updated `JSON_RPC_API_GUIDE.md` with tested methods

---

## [0.0.7] - 2026-01-10

### Added
- Automatic port conflict resolution for HTTP API and RPC servers
- Port fallback mechanism (tries up to 10 alternative ports)
- Improved error handling for "address already in use" errors

### Fixed
- **Port Conflict Issue**: Fixed "Only one usage of each socket address... is normally permitted" error (commit 3174110)
- Node now automatically finds available ports and continues running

### Changed
- Both HTTP API and RPC servers now have automatic port conflict detection
- No manual intervention needed for port conflicts

---

## Earlier Versions

### [0.0.6] - Core Blockchain Implementation
- Real Proof-of-Work mining (Blake3, KHeavyHash)
- GhostDAG consensus with persistent storage
- Hybrid storage (RAM + sled database)
- Transaction processing and validation
- BraidCore mining architecture

### [0.0.5] - Storage & Persistence
- Integrated sled database for block storage
- Hybrid RAM + disk storage architecture
- Block pruning and caching
- State persistence across restarts

### [0.0.4] - Network Layer
- P2P networking with multi-node communication
- Block propagation
- Transaction propagation
- Peer discovery

### [0.0.3] - Consensus Layer
- GhostDAG implementation
- DAG block structure
- Block validation
- Parent block selection

### [0.0.2] - Mining Implementation
- BraidCore mining (3 parallel streams)
- Stream A: ASIC mining (Blake3, 10s blocks)
- Stream B: CPU mining (KHeavyHash, 5s blocks) — GPU via OpenCL planned
- Stream C: ZK proofs (1s blocks) — BraidCore architecture
- Block rewards and tokenomics

### [0.0.1] - Initial Release
- Basic blockchain structure
- Transaction creation and signing
- Block creation
- Genesis block
- Python proof of concept

---

## Release Notes Format

Each version includes:
- **Added**: New features and capabilities
- **Changed**: Changes to existing functionality
- **Fixed**: Bug fixes and corrections
- **Performance**: Performance improvements and metrics
- **Documentation**: Documentation updates
- **Technical Details**: Commit hashes and technical implementation notes

---

## Versioning

We use [Semantic Versioning](https://semver.org/):
- **MAJOR**: Incompatible API changes
- **MINOR**: New functionality (backwards compatible)
- **PATCH**: Bug fixes (backwards compatible)

---

**Last Updated**: 2026-04-10  
**Current Version**: 0.3.x-testnet (Testnet LIVE)  
**Next Version**: 0.4.0 (Production Candidate)
