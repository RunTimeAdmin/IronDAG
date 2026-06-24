# Module Reference

Complete documentation for all IronDAG blockchain modules.

---

## Core Modules

### `blockchain` - Core Blockchain

**Location**: `src/blockchain/`

The core blockchain implementation with fine-grained locking for high concurrency.

#### Key Types

| Type | Description |
|------|-------------|
| `Blockchain` | Main blockchain struct with state management |
| `Block` | Block structure with header and transactions |
| `BlockHeader` | Block metadata (number, timestamp, parent hashes) |
| `Transaction` | Transaction with signature and optional privacy data |

#### Architecture

```rust
pub struct Blockchain {
    database: Option<Arc<Database>>,           // Persistent storage
    ghostdag: GhostDAG,                        // Consensus engine
    blocks: Arc<RwLock<BlocksData>>,           // Block storage
    accounts: Arc<DashMap<Address, AccountState>>, // Lock-free accounts
    cached_latest_block_number: Arc<AtomicU64>,    // Atomic height cache
    evm_executor: Option<EvmTransactionExecutor>,  // EVM integration
    // ... additional components
}
```

#### Key Methods

- `add_block(block)` - Add and validate a new block
- `get_balance(address)` - Get account balance (lock-free)
- `get_nonce(address)` - Get account nonce
- `get_blocks()` - Get all blocks
- `get_latest_block()` - Get the most recent block

---

### `consensus` - GhostDAG Consensus

**Location**: `src/consensus/`

Full GhostDAG (BlockDAG) consensus algorithm based on Kaspa's protocol.

#### Key Types

| Type | Description |
|------|-------------|
| `GhostDAG` | Main consensus engine |
| `DAGStats` | Statistics about the DAG |
| `HybridDagStorage` | Hot cache + disk storage |

#### Blue Score Calculation

Blocks are classified as "blue" (selected) or "red" (not selected) based on their connectivity to previous blue blocks:

```
Blue Score = max(blue scores of blue parents) + 1
```

#### Key Methods

- `add_block(block)` - Add block to DAG and recalculate consensus
- `get_ordered_blocks()` - Get blocks in final consensus order
- `get_blue_set()` - Get blue (selected) blocks
- `get_blue_score(hash)` - Get blue score for a block
- `is_blue(hash)` - Check if block is in blue set

---

### `evm` - Ethereum Virtual Machine

**Location**: `src/evm/`

Full EVM integration using SputnikVM (evm 0.41) for smart contract execution.

#### Key Types

| Type | Description |
|------|-------------|
| `EvmTransactionExecutor` | Main EVM executor |
| `EvmState` | In-memory EVM state |
| `ExecutionResult` | Result of EVM execution |

#### Supported Operations

- Contract deployment (CREATE)
- Contract calls (CALL)
- Storage read/write (SLOAD/SSTORE)
- Balance transfers

#### Key Methods

```rust
// Deploy a contract
pub fn deploy_contract(
    &self,
    from: Address,
    code: Vec<u8>,
    value: u128,
    gas_limit: u64,
    nonce: u64,
    block_number: u64,
    block_timestamp: u64,
) -> Result<(Address, ExecutionResult), String>

// Call a contract
pub fn call_contract(
    &self,
    from: Address,
    to: Address,
    data: Vec<u8>,
    value: u128,
    gas_limit: u64,
    block_number: u64,
    block_timestamp: u64,
) -> Result<ExecutionResult, String>
```

---

### `sharding` - Horizontal Sharding

**Location**: `src/sharding/`

Horizontal sharding for blockchain scalability with cross-shard transaction support.

#### Key Types

| Type | Description |
|------|-------------|
| `ShardManager` | Manages all shards |
| `Shard` | Individual shard with its own blockchain |
| `ShardConfig` | Shard configuration |
| `CrossShardTransaction` | Cross-shard transaction data |
| `AssignmentStrategy` | How addresses map to shards |

#### Assignment Strategies

| Strategy | Description |
|----------|-------------|
| `ConsistentHashing` | Blake3 hash of address mod shard count |
| `RoundRobin` | Distributed evenly |
| `AddressBased` | Based on address bytes |

#### Cross-Shard Flow

```
1. Source shard validates and deducts funds
2. Receipt created and sent via async channel
3. Target shard processes receipt and credits funds
4. All operations are non-blocking
```

---

### `mining` - BraidCore Mining

**Location**: `src/mining.rs`, `src/mining/`

BraidCore Mining with three parallel hash streams.

#### Mining Streams

| Stream | Algorithm | Purpose |
|--------|-----------|---------|
| Stream A | Blake3 | Fast software mining |
| Stream B | KHeavyHash | GPU-optimized |
| Stream C | Keccak256 | ASIC-resistant |

#### Key Types

| Type | Description |
|------|-------------|
| `Miner` | Main mining coordinator |
| `MiningConfig` | Mining configuration |
| `MiningStats` | Mining statistics |
| `FairnessScore` | MEV fairness metrics |

---

### `pow` - Proof of Work

**Location**: `src/pow.rs`

Proof of work implementation with multiple hash algorithms.

#### Hash Functions

```rust
pub fn hash_blake3(header: &BlockHeader, tx_root: &[u8; 32]) -> Hash
pub fn hash_kheavy(header: &BlockHeader, tx_root: &[u8; 32]) -> Hash
pub fn hash_keccak256(header: &BlockHeader, tx_root: &[u8; 32]) -> Hash
```

#### Difficulty Adjustment

- Target block time: 10 seconds
- Adjustment window: 2016 blocks
- Max adjustment: 4x up or down

---

### `storage` - Persistent Storage

**Location**: `src/storage.rs`

Persistent storage using sled embedded database.

#### Key Types

| Type | Description |
|------|-------------|
| `Database` | Main database wrapper |
| `BlockStore` | Block storage operations |
| `StateStore` | Account state storage |

#### Storage Operations

```rust
// Block storage
pub fn put(&self, block: &Block) -> Result<()>
pub fn get(&self, hash: &Hash) -> Result<Option<Block>>
pub fn get_all_blocks(&self) -> Result<Vec<Block>>

// State storage
pub fn put_balance(&self, address: &Address, balance: u128) -> Result<()>
pub fn get_balance(&self, address: &Address) -> Result<Option<u128>>
pub fn put_contract_code(&self, address: &Address, code: Vec<u8>) -> Result<()>
pub fn get_contract_code(&self, address: &Address) -> Result<Option<Vec<u8>>>
```

---

## Advanced Modules

### `account_abstraction` - Smart Contract Wallets

**Location**: `src/account_abstraction/`

ERC-4337 style account abstraction with smart contract wallets.

#### Features

- **Multi-signature wallets**: M-of-N signing
- **Social recovery**: Guardian-based account recovery
- **Spending limits**: Daily/transaction limits
- **Batch transactions**: Multiple operations in one tx
- **Gasless transactions**: Sponsor pays gas

#### Key Types

| Type | Description |
|------|-------------|
| `SmartContractWallet` | Main wallet struct |
| `WalletFactory` | Creates new wallets |
| `WalletRegistry` | Tracks all wallets |
| `MultiSigManager` | Multi-sig transaction handling |
| `SocialRecoveryManager` | Recovery operations |
| `BatchManager` | Batch transaction handling |

---

### `oracles` - Built-In Oracle Network

**Location**: `src/oracles/`

Native oracle system for price feeds and randomness.

#### Components

| Component | Purpose |
|-----------|---------|
| `OracleRegistry` | Register and manage oracle nodes |
| `PriceFeedManager` | Aggregate price data |
| `VrfManager` | Verifiable random function |
| `OracleStaking` | Stake management and slashing |

#### Configuration

```rust
pub struct OracleConfig {
    pub min_stake: u128,              // 1 IDAG default
    pub min_oracles_per_feed: usize,  // 3 default
    pub slashing_percentage: f64,     // 10% default
    pub price_update_frequency: u64,  // 60 seconds
}
```

---

### `privacy` - zk-SNARK Privacy Layer

**Location**: `src/privacy/`

Native privacy transactions using zero-knowledge proofs.

#### Components

| Component | Purpose |
|-----------|---------|
| `PrivacyManager` | Coordinate privacy operations |
| `PrivacyProver` | Generate zk-SNARK proofs |
| `PrivacyVerifier` | Verify proofs |
| `NullifierSet` | Prevent double-spending |
| `PedersenCommitment` | Hide transaction amounts |

#### Cryptographic Primitives

- **Curve**: BN254 (alt_bn128)
- **Proof System**: Groth16
- **Commitment**: Pedersen
- **Merkle Tree**: 20 levels (1M notes)

---

### `pqc` - Post-Quantum Cryptography

**Location**: `src/pqc/`

Quantum-resistant cryptographic operations.

#### Algorithms

| Algorithm | Purpose |
|-----------|---------|
| Dilithium | Digital signatures |
| Kyber | Key encapsulation |

#### Key Types

| Type | Description |
|------|-------------|
| `PqAccount` | Post-quantum account |
| `PqSignature` | Dilithium signature |
| `KyberKeyExchange` | Key exchange |
| `PqEncryption` | Hybrid encryption |

---

### `verkle` - Verkle Trees

**Location**: `src/verkle/`

Verkle tree implementation for stateless clients.

#### Key Types

| Type | Description |
|------|-------------|
| `VerkleTree` | Main tree structure |
| `VerkleState` | State backed by Verkle tree |
| `StateProof` | Proof for state access |
| `ProofVerifier` | Verify state proofs |

---

### `security` - Security

**Location**: `src/security/`

Rule-based security and fraud detection.

#### Components

| Component | Purpose |
|-----------|---------|
| `FraudDetector` | Pattern-based fraud detection |
| `RiskScorer` | Address and transaction risk scoring |
| `ForensicAnalyzer` | Fund tracing and analysis |
| `SecurityPolicyManager` | Dynamic security policies |
| `SecurityHardening` | DoS protection, rate limiting |

#### Risk Labels

| Label | Description |
|-------|-------------|
| `Honeypot` | Known scam contract |
| `Mixer` | Coin mixing service |
| `Phishing` | Known phishing address |
| `Sanctioned` | Regulatory sanctioned |
| `Clean` | No risk detected |

---

### `rpc` - JSON-RPC Server

**Location**: `src/rpc.rs`, `src/rpc/`

Ethereum-compatible JSON-RPC API.

#### Supported Methods

| Method | Description |
|--------|-------------|
| `eth_chainId` | Get chain ID |
| `eth_blockNumber` | Get current block number |
| `eth_getBalance` | Get account balance |
| `eth_getTransactionCount` | Get account nonce |
| `eth_sendRawTransaction` | Submit signed transaction |
| `eth_call` | Execute read-only call |
| `eth_getCode` | Get contract bytecode |
| `eth_getStorageAt` | Read contract storage |
| `eth_getBlockByNumber` | Get block by number |
| `eth_getBlockByHash` | Get block by hash |
| `eth_getTransactionReceipt` | Get transaction receipt |

---

### `governance` - On-Chain Governance

**Location**: `src/governance/`

Decentralized governance system.

#### Components

| Component | Purpose |
|-----------|---------|
| `GovernanceRegistry` | Proposal tracking |
| `NodeIdentity` | Node registration |
| `Longevity` | Voting power based on stake age |

---

### `network` - P2P Networking

**Location**: `src/network.rs`, `src/network/`

Peer-to-peer networking layer.

#### Features

- Block propagation
- Transaction gossip
- Peer discovery
- Chain synchronization

---

## Supporting Modules

| Module | Location | Purpose |
|--------|----------|---------|
| `types` | `src/types.rs` | Common type definitions |
| `error` | `src/error.rs` | Error types |
| `config` | `src/config.rs` | Configuration loading |
| `metrics` | `src/metrics.rs` | Prometheus metrics |
| `reputation` | `src/reputation.rs` | Peer reputation |
| `light_client` | `src/light_client.rs` | Light client support |
| `recurring` | `src/recurring/` | Recurring transactions |
| `stop_loss` | `src/stop_loss/` | Stop-loss orders |

---

## Next Steps

- [Dependencies](Dependencies) - External libraries
- [API Reference](API-Reference) - JSON-RPC documentation
- [Getting Started](Getting-Started) - Setup guide

