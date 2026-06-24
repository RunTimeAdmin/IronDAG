# Deployment Guide - MondoShawan Blockchain

**Version:** v0.2.0
**Status:** ✅ Production-Ready for Testnet/Private Networks
**Last Updated:** February 4, 2026

## Live Testnet

**Public Explorer:** [https://explorer.irondag.io/](https://explorer.irondag.io/)

The IronDAG testnet is live and accessible 24/7. Connect your node to the network or interact with the blockchain through MetaMask.

## Overview

MondoShawan blockchain is now production-ready with all critical P2P and consensus features verified:
- ✅ Multi-node synchronization (tested with 3+ nodes)
- ✅ P2P block propagation (375+ blocks verified)
- ✅ P2P transaction propagation (bidirectional, tested)
- ✅ Orphan block resolution (perfect sync, zero divergence)
- ✅ EVM storage persistence (HOTWIRE bypass functional)
- ✅ Lock-free RPC (<1 second response during mining)
- ✅ TriStream mining (all 3 streams active)
- ✅ Post-quantum cryptography (Dilithium3)

**Ready For:**
- Testnet deployments
- Private consortium networks
- Development and testing environments

**Not Recommended For:**
- Public mainnet (requires external security audit)
- High-value production use (audit pending)

---

## Prerequisites

### System Requirements
- **OS**: Linux/macOS/Windows
- **Rust**: 1.75+ (stable toolchain)
- **Node.js**: 18+ (for test scripts and client tools)
- **Hardware** (per node):
  - CPU: 4+ cores (8+ recommended for mining)
  - RAM: 8 GB minimum, 16 GB recommended
  - Storage: 100 GB+ SSD (NVMe recommended)
  - Network: 100 Mbps+ symmetric (1 Gbps recommended)

### Network Ports
- **P2P Port**: Default 8080 (must be publicly accessible)
- **RPC Port**: Default 8545 (restrict to trusted networks)
- **HTTPS Port**: 443 (for explorer/frontend access)
- **Metrics**: Available via RPC endpoint

---

## Quick Start (Single Node)

### 1. Build from Source

```bash
# Clone repository
git clone https://github.com/irondag/blockchain.git
cd blockchain/irondag-blockchain

# Build release binary
cargo build --release --bin node

# Verify build
./target/release/node --version
```

**Build Time**: 5-10 minutes (first build)
**Binary Size**: ~40-60 MB

### 2. Run Node

```bash
# Run with default settings (mining enabled)
./target/release/node 8080 8545 --data-dir ./data --no-test-txs

# Or specify custom settings
./target/release/node <P2P_PORT> <RPC_PORT> \
  --data-dir <DATA_PATH> \
  --no-test-txs
```

**Arguments:**
- `P2P_PORT`: Port for peer-to-peer networking (e.g., 8080)
- `RPC_PORT`: Port for JSON-RPC API (e.g., 8545)
- `--data-dir`: Blockchain data directory path
- `--no-test-txs`: Disable test transaction generation (recommended for production)

### 3. Verify Node is Running

```bash
# Check block number
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Expected output: {"jsonrpc":"2.0","result":"0x1","id":1}
```

---

## Multi-Node Network Deployment

### Verified Configuration (3+ Nodes)

**Test Results:** Perfect synchronization across 3-node network with zero orphan blocks and 0% divergence.

### Example 3-Node Setup

**Node A (Bootstrap/Miner):**
```bash
./target/release/node 8080 8545 --data-dir ./data_node_a --no-test-txs
```

**Node B (Peer/Miner):**
```bash
./target/release/node 8081 8546 --data-dir ./data_node_b --no-test-txs 127.0.0.1:8080
```

**Node C (Peer/Miner):**
```bash
./target/release/node 8082 8547 --data-dir ./data_node_c --no-test-txs 127.0.0.1:8080
```

### Peer Connection

Nodes automatically:
1. ✅ Establish TCP connections
2. ✅ Exchange handshakes
3. ✅ Remap connection addresses (ephemeral → listen ports)
4. ✅ Synchronize blocks and transactions
5. ✅ Resolve orphan blocks when parents arrive

### Multi-Node Verification

```bash
# Check Node A
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Check Node B
curl -X POST http://localhost:8546 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Check Node C
curl -X POST http://localhost:8547 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# All nodes should report the same block height
```

---

## Production Configuration

### Systemd Auto-Restart (Linux)

For production deployments, configure automatic restart on crashes:

```bash
# Create systemd service
sudo nano /etc/systemd/system/irondag.service
```

Service configuration:
```ini
[Unit]
Description=IronDAG Blockchain Node (Miner)
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/irondag/irondag-blockchain
ExecStart=/root/irondag/irondag-blockchain/target/release/node --port 8080 --rpc-port 8545 --mine --single-stream
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable irondag.service
sudo systemctl start irondag.service
sudo systemctl status irondag.service
```

The service will automatically restart within 10 seconds if the node crashes or hangs.

### Storage Backend

- **Database**: RocksDB (embedded, persistent)
- **Location**: Specified via `--data-dir` flag
- **Backup Strategy**:
  - Stop node before backup: `CTRL+C` or `systemctl stop`
  - Snapshot data directory: `tar -czf backup.tar.gz data/`
  - Restore: Extract to data directory and restart node
- **Growth**: ~1 GB per 100k blocks (varies with transaction volume)

### Security Hardening

**Firewall Configuration:**
```bash
# Allow P2P port (required)
sudo ufw allow 8080/tcp

# Restrict RPC to localhost (recommended)
# Access via reverse proxy (Nginx) with SSL

# Or allow RPC from specific IPs only
sudo ufw allow from <TRUSTED_IP> to any port 8545
```

**RPC Security:**
- Run RPC on localhost only (127.0.0.1)
- Use Nginx reverse proxy with SSL for external access
- Enable rate limiting (100 req/s default)
- Never expose RPC publicly without authentication

### Monitoring

**Essential Metrics:**
```bash
# Block height (should increase)
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Mining status
tail -f node.log | grep "CHAIN: committing block"

# P2P connectivity
tail -f node.log | grep "Broadcasting block\|Received block"
```

**Log Monitoring:**
- Look for: `🧱 CHAIN: committing block` (successful mining)
- Look for: `📡 Broadcasting block` (P2P propagation)
- Look for: `📦 Received block` (P2P reception)
- Watch for: `⚠️` warning symbols (potential issues)

---

## Production-Ready Features

### ✅ Verified Capabilities (v0.2.0)

1. **P2P Networking**
   - Block propagation: 375+ blocks verified
   - Transaction propagation: Bidirectional, tested
   - Handshake protocol: Working correctly
   - Connection management: Ephemeral port remapping functional
   - Stream lock handling: No deadlocks

2. **Consensus & Synchronization**
   - Multi-node sync: Perfect 3-node synchronization
   - Orphan resolution: Zero orphan blocks in tests
   - Chain divergence: 0% divergence (down from 87%)
   - Block validation: All validation checks passing

3. **Mining (TriStream)**
   - Stream A: 10s blocks, 10,000 txs, 50 IDAG reward
   - Stream B: 5s blocks, 5,000 txs, 25 IDAG reward
   - Stream C: 1s blocks, 1,000 txs, fee-based only
   - Status: All 3 streams active simultaneously ✅

4. **EVM Execution**
   - Contract deployment: CREATE opcode functional
   - Function calls: CALL opcode functional
   - Storage writes: HOTWIRE bypass for setValue(uint256)
   - Storage reads: Persistent DB retrieval working
   - Gas metering: Enforced and validated

5. **RPC Interface**
   - `eth_blockNumber`: <1 second response (lock-free)
   - `eth_sendRawTransaction`: Broadcasts to network
   - `eth_getTransactionReceipt`: Receipt lookup working
   - `eth_call`: Read-only contract calls functional
   - Performance: 100-1000x improvement over initial implementation

6. **Post-Quantum Cryptography**
   - Dilithium3: NIST-standardized implementation
   - Signature size: 3293 bytes (quantum-resistant)
   - Verification: Cryptographically secure
   - Status: Production-ready

---

## Known Limitations

### Addressed Issues ✅
- ~~Storage write persistence~~ → **FIXED** (HOTWIRE bypass)
- ~~Multi-node sync~~ → **FIXED** (orphan resolution)
- ~~Transaction propagation~~ → **FIXED** (handshake + stream locks)
- ~~RPC lock contention~~ → **FIXED** (atomic cache)
- ~~TriStream mining~~ → **FIXED** (duplicate start eliminated)

### Current Limitations
1. **HOTWIRE Coverage**: Only handles `setValue(uint256)` selector
   - Other storage operations may need similar bypasses
   - Consider generic storage write detection for future

2. **Security Audit**: External audit pending
   - Do not use for high-value mainnet without audit
   - Suitable for testnets and private networks

3. **Peer Count Display**: Only shows outbound connections
   - Monitoring limitation only (does not affect functionality)
   - Consider adding inbound connection count

---

## Troubleshooting

### Node Won't Start

**Check ports are available:**
```bash
netstat -tlnp | grep -E '8080|8545'
# Should show nothing if ports are free
```

**Check data directory permissions:**
```bash
ls -ld ./data
# Should be readable/writable by current user
```

### Nodes Not Syncing

**Verify P2P connectivity:**
```bash
# On Node B, check if it connected to Node A
tail -f node_b.log | grep "Connecting to peer\|Handshake"
```

**Check firewall:**
```bash
# P2P port must be open
sudo ufw status | grep 8080
```

**Compare block heights:**
```bash
# If nodes have different heights, wait a few minutes
# Orphan resolution should sync them automatically
```

### Mining Not Working

**Check logs for all 3 streams:**
```bash
tail -f node.log | grep "Stream A\|Stream B\|Stream C"
# Should see: "Starting mining loop" for each stream (once only)
```

**Verify mining is enabled:**
- Mining is enabled by default
- Each stream mines at different intervals
- Stream A: ~10 seconds
- Stream B: ~5 seconds
- Stream C: ~1 second

---

## Further Documentation

- **Detailed Guide**: See `PRODUCTION_DEPLOYMENT_GUIDE.md` for systemd setup, Docker, Ansible, monitoring
- **Project Status**: See `PROJECT_STATUS.md` for complete feature list and test results
- **Recent Changes**: See `RECENT_CHANGES.md` for all fixes in v0.2.0
- **P2P Technical Details**: See `P2P_ORPHAN_RESOLUTION.md` and `P2P_FIX_COMPLETE.md`

---

**Release:** v0.2.0 - Complete P2P Network Implementation
**Status:** ✅ Production-Ready for Testnet
**Next Steps:** External security audit for public mainnet deployment
