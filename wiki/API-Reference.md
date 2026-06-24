# API Reference

IronDAG implements an Ethereum-compatible JSON-RPC API on port 8545.

---

## Connection

### HTTP

```
http://localhost:8545
```

### Chain ID

```
1338 (0x53a)
```

---

## Standard Ethereum Methods

### eth_chainId

Returns the chain ID.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_chainId",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x53a"
}
```

---

### eth_blockNumber

Returns the current block number.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_blockNumber",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x10"
}
```

---

### eth_getBalance

Returns the balance of an address.

**Parameters:**
1. `address` - 20-byte address
2. `block` - Block number or "latest"

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBalance",
  "params": ["0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B", "latest"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x56bc75e2d63100000"
}
```

---

### eth_getTransactionCount

Returns the nonce of an address.

**Parameters:**
1. `address` - 20-byte address
2. `block` - Block number or "latest"

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionCount",
  "params": ["0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B", "latest"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x5"
}
```

---

### eth_sendRawTransaction

Submits a signed transaction.

**Parameters:**
1. `data` - Signed transaction data (RLP encoded)

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransaction",
  "params": ["0xf86c..."],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x..."
}
```

---

### eth_call

Executes a read-only contract call.

**Parameters:**
1. `transaction` - Transaction object
2. `block` - Block number or "latest"

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_call",
  "params": [{
    "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B",
    "to": "0x1234567890123456789012345678901234567890",
    "data": "0x2e64cec1"
  }, "latest"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x0000000000000000000000000000000000000000000000000000000000000064"
}
```

---

### eth_getCode

Returns contract bytecode.

**Parameters:**
1. `address` - Contract address
2. `block` - Block number or "latest"

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getCode",
  "params": ["0x1234567890123456789012345678901234567890", "latest"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x6080604052..."
}
```

---

### eth_getStorageAt

Returns contract storage at a slot.

**Parameters:**
1. `address` - Contract address
2. `slot` - Storage slot (32 bytes)
3. `block` - Block number or "latest"

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getStorageAt",
  "params": [
    "0x1234567890123456789012345678901234567890",
    "0x0",
    "latest"
  ],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x0000000000000000000000000000000000000000000000000000000000000064"
}
```

---

### eth_getBlockByNumber

Returns block by number.

**Parameters:**
1. `block` - Block number or "latest"
2. `fullTransactions` - Include full transaction objects

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockByNumber",
  "params": ["latest", true],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "number": "0x10",
    "hash": "0x...",
    "parentHash": "0x...",
    "timestamp": "0x...",
    "transactions": [...]
  }
}
```

---

### eth_getBlockByHash

Returns block by hash.

**Parameters:**
1. `hash` - Block hash
2. `fullTransactions` - Include full transaction objects

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockByHash",
  "params": ["0x...", true],
  "id": 1
}
```

---

### eth_getTransactionReceipt

Returns transaction receipt.

**Parameters:**
1. `hash` - Transaction hash

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionReceipt",
  "params": ["0x..."],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "transactionHash": "0x...",
    "blockNumber": "0x10",
    "blockHash": "0x...",
    "status": "0x1",
    "gasUsed": "0x5208",
    "contractAddress": null
  }
}
```

---

### eth_estimateGas

Estimates gas for a transaction.

**Parameters:**
1. `transaction` - Transaction object

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_estimateGas",
  "params": [{
    "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B",
    "to": "0x1234567890123456789012345678901234567890",
    "value": "0x0",
    "data": "0x..."
  }],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x5208"
}
```

---

### eth_gasPrice

Returns current gas price.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_gasPrice",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x3b9aca00"
}
```

---

## Network Methods

### net_version

Returns network ID.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "net_version",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "1338"
}
```

---

### net_listening

Returns if node is listening.

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": true
}
```

---

### net_peerCount

Returns peer count.

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x5"
}
```

---

## Web3 Methods

### web3_clientVersion

Returns client version.

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "IronDAG/v0.1.0"
}
```

---

## IronDAG-Specific Methods

### mshw_getMiningStats

Returns mining statistics.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "mshw_getMiningStats",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "hashrate": "1000000",
    "blocksFound": 100,
    "streamA": { "blocks": 40, "hashrate": "300000" },
    "streamB": { "blocks": 35, "hashrate": "400000" },
    "streamC": { "blocks": 25, "hashrate": "300000" }
  }
}
```

---

### mshw_getShardStats

Returns sharding statistics.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "mshw_getShardStats",
  "params": [],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "shardCount": 4,
    "shards": [
      { "id": 0, "blockCount": 100, "txPoolSize": 50 },
      { "id": 1, "blockCount": 98, "txPoolSize": 45 }
    ]
  }
}
```

---

### mshw_getRiskScore

Returns risk score for an address.

**Parameters:**
1. `address` - Address to analyze

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "mshw_getRiskScore",
  "params": ["0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B"],
  "id": 1
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "score": 0.15,
    "label": "Low Risk",
    "factors": ["new_address", "small_transactions"]
  }
}
```

---

## Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid request | Invalid request object |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Invalid parameters |
| -32603 | Internal error | Internal server error |
| -32000 | Server error | Generic server error |

---

## MetaMask Configuration

### Network Settings

| Setting | Value |
|---------|-------|
| Network Name | IronDAG Testnet |
| RPC URL | http://localhost:8545 |
| Chain ID | 1338 |
| Currency Symbol | IDAG |
| Block Explorer | http://localhost:3000 |

---

## Code Examples

### JavaScript (ethers.js)

```javascript
const { ethers } = require('ethers');

const provider = new ethers.JsonRpcProvider('http://localhost:8545');

// Get balance
const balance = await provider.getBalance('0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B');
console.log('Balance:', ethers.formatEther(balance));

// Get block number
const blockNumber = await provider.getBlockNumber();
console.log('Block:', blockNumber);

// Send transaction
const wallet = new ethers.Wallet(privateKey, provider);
const tx = await wallet.sendTransaction({
  to: '0x1234567890123456789012345678901234567890',
  value: ethers.parseEther('1.0')
});
await tx.wait();
```

### Python (web3.py)

```python
from web3 import Web3

w3 = Web3(Web3.HTTPProvider('http://localhost:8545'))

# Get balance
balance = w3.eth.get_balance('0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B')
print(f'Balance: {w3.from_wei(balance, "ether")} IDAG')

# Get block number
block = w3.eth.block_number
print(f'Block: {block}')
```

### curl

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

---

## Next Steps

- [Getting Started](Getting-Started) - Setup guide
- [Architecture Overview](Architecture-Overview) - System design
- [Module Reference](Module-Reference) - Module documentation
