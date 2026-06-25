# IronDAG JSON-RPC API Reference

## Connection

- **Default endpoint**: `http://localhost:8546`
- **CORS**: Configurable via `--cors-origin` flag or `cors_origins` in TOML config
- **Authentication**: API key required by default; disable with `--rpc-no-auth` (dev only)
- **Rate limiting**: 1000 requests/minute per IP default (configurable via `--rpc-rate-limit`)
- **Burst size**: 50 requests default (configurable via `--rpc-burst-size`)

### Authentication

By default, the RPC server requires API key authentication via the `X-API-Key` header:

```bash
curl -X POST http://localhost:8546 \
  -H "Content-Type: application/json" \
  -H "X-API-Key: your-api-key" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

Public methods (no auth required):
- `eth_blockNumber`
- `net_version`
- `eth_chainId`
- `eth_syncing`
- `irondag_getDagStats`
- `irondag_getTps`
- `irondag_getBlocksByStream`
- `irondag_faucet` (test builds only)

---

## Standard Ethereum Methods

### eth_blockNumber
Returns the current latest block number.

**Parameters**: None

**Returns**: `String` - Hex block number

```json
{
  "jsonrpc": "2.0",
  "method": "eth_blockNumber",
  "params": [],
  "id": 1
}
```

### eth_getBalance
Returns the balance of an address.

**Parameters**:
- `address`: String (hex address, 0x + 40 chars)
- `blockNumber`: String ("latest", "earliest", "pending", or hex block number)

**Returns**: `String` - Hex balance in wei

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBalance",
  "params": ["0x0101010101010101010101010101010101010101", "latest"],
  "id": 1
}
```

### eth_getTransactionCount
Returns the nonce for an address.

**Parameters**:
- `address`: String (hex address)
- `blockNumber`: String (block tag or hex)

**Returns**: `String` - Hex nonce

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionCount",
  "params": ["0x0101010101010101010101010101010101010101", "latest"],
  "id": 1
}
```

### eth_getBlockByNumber
Returns block information by block number.

**Parameters**:
- `blockNumber`: String ("latest", "earliest", "pending", or hex)
- `fullTransactions`: Boolean (true = full tx objects, false = tx hashes only)

**Returns**: `Object` - Block data

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockByNumber",
  "params": ["latest", true],
  "id": 1
}
```

### eth_getBlockByHash
Returns block information by block hash.

**Parameters**:
- `blockHash`: String (0x + 64 hex chars)
- `fullTransactions`: Boolean

**Returns**: `Object` - Block data

### eth_getTransactionByHash
Returns transaction information by hash.

**Parameters**:
- `transactionHash`: String (0x + 64 hex chars)

**Returns**: `Object` - Transaction data

### eth_sendRawTransaction
Submits a signed transaction to the network.

**Parameters**:
- `signedTransaction`: String (RLP-encoded signed transaction hex)

**Returns**: `String` - Transaction hash

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransaction",
  "params": ["0xf86c..."],
  "id": 1
}
```

**Rate limit**: 100 transactions/minute per IP

### eth_sendTransaction
Submits an unsigned transaction (requires node to sign - dev only).

**Parameters**:
- `transaction`: Object with `from`, `to`, `value`, `data`, `gas`, `gasPrice`

**Returns**: `String` - Transaction hash

**Note**: Disabled by default; enable with `--allow-unsigned-eth-send` in debug builds only.

### eth_getTransactionReceipt
Returns the receipt for a transaction.

**Parameters**:
- `transactionHash`: String

**Returns**: `Object` - Receipt data with `status`, `gasUsed`, `logs`, etc.

### eth_call
Executes a read-only contract call.

**Parameters**:
- `callObject`: Object with `from`, `to`, `data`, `value`, `gas`, `gasPrice`
- `blockNumber`: String (block tag or hex)

**Returns**: `String` - Return data hex

### eth_estimateGas
Estimates gas for a transaction.

**Parameters**:
- `transaction`: Object with `from`, `to`, `data`, `value`

**Returns**: `String` - Hex gas estimate

### eth_gasPrice
Returns the current gas price.

**Parameters**: None

**Returns**: `String` - Hex gas price in wei (default: 20 gwei = 0x4a817c800)

### eth_chainId
Returns the chain ID.

**Parameters**: None

**Returns**: `String` - Hex chain ID (default: 0x539 = 1337)

### eth_syncing
Returns sync status.

**Parameters**: None

**Returns**: `Boolean` - Always false (node syncs in background)

### eth_getCode
Returns contract bytecode at an address.

**Parameters**:
- `address`: String (hex address)
- `blockNumber`: String (block tag or hex)

**Returns**: `String` - Bytecode hex

### eth_getStorageAt
Returns storage value at a slot.

**Parameters**:
- `address`: String (hex address)
- `slot`: String (hex slot, 0x + 64 chars)
- `blockNumber`: String (block tag or hex)

**Returns**: `String` - Storage value hex (32 bytes)

### eth_getBlockTransactionCountByNumber
Returns transaction count in a block.

**Parameters**:
- `blockNumber`: String (block tag or hex)

**Returns**: `String` - Hex transaction count

### eth_feeHistory
Returns fee history (stub for MetaMask compatibility).

**Parameters**:
- `blockCount`: Number or hex
- `newestBlock`: String (block tag or hex)
- `rewardPercentiles`: Array of numbers (optional)

**Returns**: `Object` - Fee history data

### eth_maxPriorityFeePerGas
Returns max priority fee per gas (Fee-priority ordering compatibility (full EIP-1559 dynamic base fee planned for future protocol upgrade)).

**Parameters**: None

**Returns**: `String` - Hex value (default: 0x0)

---

## Network Methods

### net_version
Returns the network version (chain ID as string).

**Parameters**: None

**Returns**: `String` - Chain ID

### net_peerCount
Returns the number of connected peers.

**Parameters**: None

**Returns**: `String` - Hex peer count

---

## IronDAG-Specific Methods

### DAG & Consensus

#### irondag_getDagStats
Returns GhostDAG statistics.

**Parameters**: None

**Returns**: `Object`
```json
{
  "total_blocks": 1234,
  "blue_blocks": 1200,
  "red_blocks": 34,
  "total_transactions": 5678,
  "total_size_bytes": 1234567,
  "avg_block_size": 1000,
  "avg_txs_per_block": 4.5
}
```

#### irondag_getBlueScore
Returns the blue score for a block.

**Parameters**:
- `blockHash`: String

**Returns**: `String` - Hex blue score

#### irondag_getTps
Returns transactions per second over a duration.

**Parameters**:
- `durationSeconds`: Number (default: 60)

**Returns**: `Number` - TPS value

#### irondag_getBlocksByStream
Returns blocks filtered by stream type.

**Parameters**:
- `streamType`: String ("A", "B", or "C")
- `count`: Number (max 100)

**Returns**: `Array` - Block objects

### Mining Methods

#### irondag_getMiningStatus
Returns current mining status.

**Parameters**: None

**Returns**: `Object` with mining state

#### irondag_startMining
Starts mining (if stopped).

**Parameters**: None

**Returns**: `Object` - Success status

#### irondag_stopMining
Stops mining.

**Parameters**: None

**Returns**: `Object` - Success status

#### irondag_getMiningDashboard
Returns detailed mining statistics.

**Parameters**:
- `durationSeconds`: Number (optional)

**Returns**: `Object` with stream stats, rewards, etc.

### Node Methods

#### irondag_getNodeStatus
Returns comprehensive node status.

**Parameters**: None

**Returns**: `Object` with version, uptime, peers, mining status

#### irondag_sendRawTransaction
IronDAG-native raw transaction submission.

**Parameters**:
- `signedTransaction`: String (hex)

**Returns**: `String` - Transaction hash

**Rate limit**: 100 transactions/minute per IP

### Sharding Methods

#### irondag_getShardStats
Returns sharding statistics.

**Parameters**:
- `shardId`: Number (optional, returns all shards if omitted)

**Returns**: `Object` - Shard statistics

#### irondag_getShardForAddress
Returns which shard an address belongs to.

**Parameters**:
- `address`: String (hex address)

**Returns**: `Number` - Shard ID

#### irondag_getCrossShardTransaction
Returns cross-shard transaction details.

**Parameters**:
- `txHash`: String

**Returns**: `Object` - Cross-shard transaction data

#### irondag_getCrossShardTransactions
Lists cross-shard transactions for an address.

**Parameters**:
- `address`: String
- `limit`: Number (optional, default 100)

**Returns**: `Array` - Transaction objects

### Security & Risk Methods

#### irondag_getRiskScore
Returns risk score for an address.

**Parameters**:
- `address`: String

**Returns**: `Object` with score and factors

#### irondag_getRiskLabels
Returns risk labels for an address.

**Parameters**:
- `address`: String

**Returns**: `Array` - Risk labels

#### irondag_getTransactionRisk
Analyzes transaction risk.

**Parameters**:
- `txHash`: String

**Returns**: `Object` - Risk analysis

#### irondag_traceFunds
Traces fund flow from an address.

**Parameters**:
- `address`: String
- `depth`: Number (optional, default 3)

**Returns**: `Object` - Fund trace results

#### irondag_getAddressSummary
Returns comprehensive address summary.

**Parameters**:
- `address`: String

**Returns**: `Object` with balance, transaction count, risk score

#### irondag_getAddressTransactions
Returns transactions for an address.

**Parameters**:
- `address`: String
- `limit`: Number (optional)

**Returns**: `Array` - Transaction objects

### MEV & Fairness Methods

#### irondag_getMevMetrics
Returns MEV protection metrics.

**Parameters**:
- `blockNumber`: Number or "latest"

**Returns**: `Object` - MEV metrics

#### irondag_getBlockFairness
Returns block fairness analysis.

**Parameters**:
- `blockHash`: String

**Returns**: `Object` - Fairness metrics

#### irondag_getFairnessMetrics
Returns overall fairness metrics.

**Parameters**:
- `durationBlocks`: Number (optional)

**Returns**: `Object` - Fairness statistics

#### irondag_getOrderingPolicy
Returns current transaction ordering policy.

**Parameters**: None

**Returns**: `String` - Policy name ("time_boost", "fcfs", "priority")

#### irondag_setOrderingPolicy
Sets transaction ordering policy (governance only).

**Parameters**:
- `policy`: String ("time_boost", "fcfs", "priority")

**Returns**: `Object` - Success status

### State & Light Client Methods

#### irondag_getStateRoot
Returns current state root hash.

**Parameters**: None

**Returns**: `String` - Hex state root

#### irondag_getStateProof
Returns state proof for an address.

**Parameters**:
- `address`: String
- `slots`: Array of strings (storage slots)

**Returns**: `Object` - State proof

#### irondag_verifyStateProof
Verifies a state proof.

**Parameters**:
- `proof`: Object (state proof)

**Returns**: `Boolean` - Validity

#### irondag_getLightClientSyncStatus
Returns light client sync status.

**Parameters**: None

**Returns**: `Object` - Sync progress

#### irondag_enableLightClientMode
Enables light client mode.

**Parameters**:
- `enabled`: Boolean

**Returns**: `Object` - Success status

### Post-Quantum Account Methods

#### irondag_generatePqAccount
Generates a post-quantum secure account.

**Parameters**:
- `accountType`: String ("dilithium", "falcon", "sphincs")

**Returns**: `Object` - Address and public key

#### irondag_getPqAccountType
Returns PQ account type.

**Parameters**:
- `address`: String

**Returns**: `String` - Account type or null

#### irondag_exportPqKey
Exports PQ private key (encrypted).

**Parameters**:
- `address`: String
- `password`: String

**Returns**: `String` - Encrypted key

#### irondag_importPqKey
Imports PQ private key.

**Parameters**:
- `encryptedKey`: String
- `password`: String

**Returns**: `Object` - Import result

#### irondag_createPqTransaction
Creates a PQ-signed transaction.

**Parameters**:
- `from`: String
- `to`: String
- `value`: String (hex)
- `data`: String (hex, optional)

**Returns**: `Object` - Signed transaction

### Security Policy Methods

#### irondag_addSecurityPolicy
Adds a security policy.

**Parameters**:
- `policy`: Object with `type`, `condition`, `action`

**Returns**: `Object` - Policy ID

#### irondag_removeSecurityPolicy
Removes a security policy.

**Parameters**:
- `policyId`: String

**Returns**: `Object` - Success status

#### irondag_getSecurityPolicies
Lists security policies.

**Parameters**:
- `address`: String (optional, filter by address)

**Returns**: `Array` - Policy objects

#### irondag_setPolicyEnabled
Enables/disables a policy.

**Parameters**:
- `policyId`: String
- `enabled`: Boolean

**Returns**: `Object` - Success status

#### irondag_evaluateTransactionPolicy
Evaluates a transaction against policies.

**Parameters**:
- `transaction`: Object

**Returns**: `Object` - Evaluation results

### Governance & Node Registry

#### irondag_getNodeRegistry
Returns node registry information.

**Parameters**: None

**Returns**: `Object` - Registered nodes

#### irondag_getNodeLongevity
Returns node longevity score.

**Parameters**:
- `nodeId`: String

**Returns**: `Object` - Longevity metrics

#### irondag_registerNode
Registers a node (requires stake).

**Parameters**:
- `nodeInfo`: Object with `address`, `stake`, `metadata`

**Returns**: `Object` - Registration result

### Account Abstraction Methods

#### irondag_createWallet
Creates a smart contract wallet.

**Parameters**:
- `owners`: Array of addresses
- `threshold`: Number (signatures required)

**Returns**: `Object` - Wallet address and config

#### irondag_getWallet
Returns wallet information.

**Parameters**:
- `address`: String

**Returns**: `Object` - Wallet details

#### irondag_getOwnerWallets
Returns wallets owned by an address.

**Parameters**:
- `owner`: String

**Returns**: `Array` - Wallet addresses

#### irondag_isContractWallet
Checks if address is a contract wallet.

**Parameters**:
- `address`: String

**Returns**: `Boolean`

### Multi-Signature Methods

#### irondag_createMultisigTransaction
Creates a multi-sig transaction.

**Parameters**:
- `wallet`: String
- `to`: String
- `value`: String
- `data`: String (optional)

**Returns**: `Object` - Transaction ID

#### irondag_addMultisigSignature
Adds signature to multi-sig transaction.

**Parameters**:
- `txId`: String
- `signature`: String

**Returns**: `Object` - Success status

#### irondag_getPendingMultisigTransactions
Returns pending multi-sig transactions.

**Parameters**:
- `wallet`: String

**Returns**: `Array` - Pending transactions

#### irondag_validateMultisigTransaction
Validates multi-sig transaction.

**Parameters**:
- `txId`: String

**Returns**: `Object` - Validation result

### Social Recovery Methods

#### irondag_initiateRecovery
Initiates wallet recovery.

**Parameters**:
- `wallet`: String
- `newOwner`: String

**Returns**: `Object` - Recovery ID

#### irondag_approveRecovery
Approves a recovery request.

**Parameters**:
- `recoveryId`: String

**Returns**: `Object` - Success status

#### irondag_getRecoveryStatus
Returns recovery status.

**Parameters**:
- `recoveryId`: String

**Returns**: `Object` - Recovery state

#### irondag_completeRecovery
Completes recovery after threshold.

**Parameters**:
- `recoveryId`: String

**Returns**: `Object` - Success status

#### irondag_cancelRecovery
Cancels a recovery request.

**Parameters**:
- `recoveryId`: String

**Returns**: `Object` - Success status

### Batch Transaction Methods

#### irondag_createBatchTransaction
Creates a batch transaction.

**Parameters**:
- `transactions`: Array of transaction objects

**Returns**: `Object` - Batch ID

#### irondag_executeBatchTransaction
Executes a batch transaction.

**Parameters**:
- `batchId`: String

**Returns**: `Object` - Execution result

#### irondag_getBatchStatus
Returns batch transaction status.

**Parameters**:
- `batchId`: String

**Returns**: `Object` - Status and results

#### irondag_estimateBatchGas
Estimates gas for batch.

**Parameters**:
- `transactions`: Array of transaction objects

**Returns**: `String` - Hex gas estimate

### Parallel EVM Methods

#### irondag_enableParallelEVM
Enables/disables parallel EVM.

**Parameters**:
- `enabled`: Boolean

**Returns**: `Object` - Success status

#### irondag_getParallelEVMStats
Returns parallel EVM statistics.

**Parameters**: None

**Returns**: `Object` - Performance metrics

#### irondag_estimateParallelImprovement
Estimates speedup from parallel execution.

**Parameters**:
- `transactions`: Array of transaction objects

**Returns**: `Object` - Estimated improvement

### Oracle Methods

#### irondag_registerOracle
Registers as an oracle.

**Parameters**:
- `stake`: String (hex wei)
- `metadata`: Object

**Returns**: `Object` - Oracle ID

#### irondag_unregisterOracle
Unregisters an oracle.

**Parameters**:
- `oracleId`: String

**Returns**: `Object` - Success status

#### irondag_getOracleInfo
Returns oracle information.

**Parameters**:
- `oracleId`: String

**Returns**: `Object` - Oracle details

#### irondag_getOracleList
Returns list of registered oracles.

**Parameters**:
- `activeOnly`: Boolean (optional)

**Returns**: `Array` - Oracle objects

#### irondag_getPrice
Returns current price for a feed.

**Parameters**:
- `feedId`: String

**Returns**: `Object` - Price data

#### irondag_getPriceHistory
Returns price history.

**Parameters**:
- `feedId`: String
- `startTime`: Number
- `endTime`: Number

**Returns**: `Array` - Price points

#### irondag_getPriceFeeds
Returns available price feeds.

**Parameters**: None

**Returns**: `Array` - Feed IDs

#### irondag_requestRandomness
Requests VRF randomness.

**Parameters**:
- `seed`: String (hex)

**Returns**: `Object` - Request ID

#### irondag_getRandomness
Retrieves VRF randomness.

**Parameters**:
- `requestId`: String

**Returns**: `Object` - Randomness value

### Recurring Transaction Methods

#### irondag_createRecurringTransaction
Creates a recurring transaction.

**Parameters**:
- `to`: String
- `value`: String
- `intervalSeconds`: Number
- `executions`: Number (optional, 0 = unlimited)

**Returns**: `Object` - Recurring TX ID

#### irondag_cancelRecurringTransaction
Cancels a recurring transaction.

**Parameters**:
- `recurringId`: String

**Returns**: `Object` - Success status

#### irondag_getRecurringTransaction
Returns recurring transaction details.

**Parameters**:
- `recurringId`: String

**Returns**: `Object` - Recurring TX details

#### irondag_getRecurringTransactions
Lists recurring transactions for an address.

**Parameters**:
- `address`: String

**Returns**: `Array` - Recurring transactions

#### irondag_pauseRecurringTransaction
Pauses a recurring transaction.

**Parameters**:
- `recurringId`: String

**Returns**: `Object` - Success status

#### irondag_resumeRecurringTransaction
Resumes a paused recurring transaction.

**Parameters**:
- `recurringId`: String

**Returns**: `Object` - Success status

### Stop-Loss Methods

#### irondag_createStopLoss
Creates a stop-loss order.

**Parameters**:
- `token`: String
- `triggerPrice`: String
- `amount`: String

**Returns**: `Object` - Stop-loss ID

#### irondag_cancelStopLoss
Cancels a stop-loss order.

**Parameters**:
- `stopLossId`: String

**Returns**: `Object` - Success status

#### irondag_getStopLoss
Returns stop-loss details.

**Parameters**:
- `stopLossId`: String

**Returns**: `Object` - Stop-loss details

#### irondag_getStopLossOrders
Lists stop-loss orders for an address.

**Parameters**:
- `owner`: String

**Returns**: `Array` - Stop-loss orders

#### irondag_updateStopLossPrice
Updates trigger price.

**Parameters**:
- `stopLossId`: String
- `newPrice`: String

**Returns**: `Object` - Success status

#### irondag_pauseStopLoss
Pauses a stop-loss order.

**Parameters**:
- `stopLossId`: String

**Returns**: `Object` - Success status

#### irondag_resumeStopLoss
Resumes a paused stop-loss.

**Parameters**:
- `stopLossId`: String

**Returns**: `Object` - Success status

### Privacy Methods (requires `privacy` feature)

#### irondag_createPrivateTransaction
Creates a private (shielded) transaction.

**Parameters**:
- `to`: String
- `value`: String
- `zkProof`: String

**Returns**: `Object` - Transaction hash

**Note**: Requires `privacy` feature enabled

#### irondag_verifyPrivacyProof
Verifies a privacy proof.

**Parameters**:
- `proof`: String
- `publicInputs`: Array

**Returns**: `Boolean` - Validity

**Note**: Requires `privacy` feature enabled

#### irondag_proveBalance
Creates a zero-knowledge balance proof.

**Parameters**:
- `address`: String
- `minBalance`: String

**Returns**: `Object` - Proof data

**Note**: Requires `privacy` feature enabled

#### irondag_getPrivacyStats
Returns privacy layer statistics.

**Parameters**: None

**Returns**: `Object` - Privacy metrics

**Note**: Requires `privacy` feature enabled

### Snapshot Methods

#### irondag_createSnapshot
Creates a state snapshot.

**Parameters**:
- `blockNumber`: Number (optional, defaults to latest)

**Returns**: `Object` - Snapshot ID

#### irondag_listSnapshots
Lists available snapshots.

**Parameters**: None

**Returns**: `Array` - Snapshot metadata

#### irondag_getSnapshotInfo
Returns snapshot information.

**Parameters**:
- `snapshotId`: String

**Returns**: `Object` - Snapshot details

#### irondag_restoreSnapshot
Restores from a snapshot.

**Parameters**:
- `snapshotId`: String

**Returns**: `Object` - Success status

### Test Methods (test builds only)

#### irondag_faucet
Requests testnet tokens.

**Parameters**:
- `address`: String
- `amount`: String (optional)

**Returns**: `Object` - Transaction hash

**Note**: Only available in test builds

#### irondag_addTestBlock
Adds a test block.

**Parameters**:
- `transactions`: Array (optional)

**Returns**: `Object` - Block hash

**Note**: Only available in test builds

#### irondag_createTestTransaction
Creates a test transaction.

**Parameters**:
- `from`: String
- `to`: String
- `value`: String

**Returns**: `Object` - Transaction hash

**Note**: Only available in test builds

---

## Error Handling

### Standard JSON-RPC Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON received |
| -32600 | Invalid Request | JSON is not a valid Request object |
| -32601 | Method not found | Method does not exist |
| -32602 | Invalid params | Invalid method parameters |
| -32603 | Internal error | Internal JSON-RPC error |

### Application-Specific Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32001 | Block not found | Requested block does not exist |
| -32002 | Transaction not found | Requested transaction does not exist |
| -32003 | Invalid address format | Address format is invalid |
| -32004 | Invalid transaction | Transaction validation failed |
| -32005 | Rate limit exceeded | Too many requests |
| -32006 | Resource unavailable | Lock timeout, node busy |

### Rate Limit Response

When rate limited, the server returns:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32005,
    "message": "Rate limit exceeded"
  },
  "id": null
}
```

Transaction submission rate limit:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32005,
    "message": "Transaction submission rate limit exceeded"
  },
  "id": null
}
```

---

## gRPC API

In addition to JSON-RPC, IronDAG supports gRPC for high-performance binary communication.

### gRPC v1 (Port 50051)
- HTTP/2 with binary protobuf
- Same methods as JSON-RPC

### gRPC v2 (Binary Optimized)
- 3.3x faster than v1
- Direct binary types (no hex encoding)
- Native uint64 fields

### gRPC Methods

- `GetBlockNumber`
- `GetBlockByNumber`
- `GetBlockByHash`
- `GetBalance`
- `GetTransactionCount`
- `SendRawTransaction`
- `GetTransactionByHash`
- `GetTransactionReceipt`
- `GetDagStats`
- `GetPeerCount`
- `GetGasPrice`
- `Call`
- `EstimateGas`
- `GetCode`
- `GetStorageAt`
- `GetBlocksBatch`
