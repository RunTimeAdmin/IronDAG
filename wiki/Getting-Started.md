# Getting Started

Guide to setting up and running a IronDAG node.

---

## Prerequisites

### System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| CPU | 4 cores | 8+ cores |
| RAM | 8 GB | 16+ GB |
| Storage | 50 GB SSD | 200+ GB NVMe |
| Network | 10 Mbps | 100+ Mbps |

### Software Requirements

| Software | Version | Purpose |
|----------|---------|---------|
| Rust | 1.92.0+ | Compilation |
| Node.js | v22.19.0+ | Frontend/tools |
| Python | 3.12+ | POC/scripts |

### Platform-Specific

**Windows:**
- Visual Studio Build Tools 2022
- C++ build tools and Windows SDK

**Linux:**
```bash
sudo apt install build-essential clang cmake pkg-config
```

**macOS:**
```bash
xcode-select --install
```

---

## Installation

### 1. Install Rust

```bash
# Linux/macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Windows
# Download from https://www.rust-lang.org/tools/install
```

### 2. Clone Repository

```bash
git clone https://github.com/dev-irondag/irondag.git
cd irondag
```

### 3. Install Protocol Buffers (for gRPC)

**Windows:**
```powershell
# Download from https://github.com/protocolbuffers/protobuf/releases
# Add to PATH
```

**Linux:**
```bash
sudo apt install protobuf-compiler
```

**macOS:**
```bash
brew install protobuf
```

### 4. Build

```bash
cd irondag-blockchain
cargo build --release
```

---

## Running a Node

### Quick Start

```bash
cd irondag-blockchain
cargo run --release --bin node
```

### With Configuration

Create `config.toml`:

```toml
[node]
data_dir = "./data"
log_level = "info"

[rpc]
host = "127.0.0.1"
port = 8545
cors_origins = ["*"]

[mining]
enabled = true
threads = 4

[network]
listen_port = 30303
bootstrap_nodes = []
```

Run with config:

```bash
cargo run --release --bin node -- --config config.toml
```

### Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--config` | Config file path | None |
| `--data-dir` | Data directory | `./data` |
| `--rpc-port` | JSON-RPC port | 8545 |
| `--p2p-port` | P2P network port | 30303 |
| `--mining` | Enable mining | false |
| `--log-level` | Log verbosity | info |

---

## Verify Node is Running

### Check RPC

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
```

Expected response:
```json
{"jsonrpc":"2.0","id":1,"result":"0x53a"}
```

### Check Block Height

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

---

## Connect MetaMask

1. Open MetaMask
2. Click network dropdown → "Add Network"
3. Enter settings:

| Setting | Value |
|---------|-------|
| Network Name | IronDAG Testnet |
| RPC URL | http://localhost:8545 |
| Chain ID | 1338 |
| Currency Symbol | IDAG |

4. Click "Save"

---

## Test Account

A pre-funded test account is available:

| Field | Value |
|-------|-------|
| Address | `0x742d35Cc6634C0532925a3b844Bc9e7595f8dE1B` |
| Balance | 100 IDAG |

> Note: Private key available in testnet configuration. Do not use for mainnet.

---

## Deploy a Contract

### Using JavaScript

```javascript
const { ethers } = require('ethers');

const provider = new ethers.JsonRpcProvider('http://localhost:8545');
const wallet = new ethers.Wallet(PRIVATE_KEY, provider);

// SimpleStorage contract
const abi = [
  "function setValue(uint256 _value) public",
  "function getValue() public view returns (uint256)"
];
const bytecode = "0x608060405234801561001057600080fd5b50...";

const factory = new ethers.ContractFactory(abi, bytecode, wallet);
const contract = await factory.deploy();
await contract.waitForDeployment();

console.log('Contract deployed at:', await contract.getAddress());
```

### Using Foundry

```bash
forge create src/SimpleStorage.sol:SimpleStorage \
  --rpc-url http://localhost:8545 \
  --private-key $PRIVATE_KEY
```

---

## Running Tests

### Unit Tests

```bash
cd irondag-blockchain
cargo test
```

### Integration Tests

```bash
cargo test --test integration_test
```

### With Timeout (Windows)

```powershell
.\run_with_timeout.ps1 "cargo test" 300
```

---

## Monitoring

### Prometheus Metrics

Metrics available at `http://localhost:9090/metrics`

### Grafana Dashboards

Pre-built dashboards in `grafana/dashboards/`:

- `irondag-overview.json` - System overview
- `irondag-mining.json` - Mining stats
- `irondag-network.json` - Network stats
- `irondag-transactions.json` - Transaction metrics

```bash
cd grafana
docker-compose up -d
```

Access Grafana at `http://localhost:3000`

---

## Troubleshooting

### Port Already in Use

The node automatically tries alternative ports if the default is occupied.

Check what's using a port:
```bash
# Windows
netstat -ano | findstr :8545

# Linux/macOS
lsof -i :8545
```

### Node Won't Start

1. **Kill zombie processes:**
   ```bash
   # Windows
   taskkill /F /IM node.exe
   
   # Linux/macOS
   pkill -9 node
   ```

2. **Clear data:**
   ```bash
   rm -rf ./data
   ```

3. **Rebuild:**
   ```bash
   cargo clean
   cargo build --release
   ```

### RPC Timeout

The node uses fine-grained locking. If you experience timeouts:

1. Check CPU/memory usage
2. Reduce mining threads
3. Check for network issues

### Build Errors

**Windows MSVC errors:**
- Install Visual Studio Build Tools 2022
- Select "C++ build tools" and "Windows SDK"

**Linux missing dependencies:**
```bash
sudo apt install build-essential libssl-dev pkg-config
```

---

## Directory Structure

After running, the node creates:

```
data/
├── blocks/          # Block storage
├── state/           # Account state
├── contracts/       # Contract code
├── storage/         # Contract storage
└── node.log         # Log file
```

---

## Next Steps

- [API Reference](API-Reference) - JSON-RPC documentation
- [Architecture Overview](Architecture-Overview) - System design
- [Module Reference](Module-Reference) - Module documentation
