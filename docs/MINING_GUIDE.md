# IronDAG Mining Guide

## Overview of BraidCore Mining

IronDAG implements a unique **BraidCore Mining** architecture that runs three parallel proof-of-work streams simultaneously. This design provides:

- **ASIC-friendly** mining (Stream A) for network security
- **CPU/GPU accessible** mining (Stream B) for decentralization
- **High-frequency** transactions (Stream C) for throughput

Each stream operates independently with different algorithms, block times, and reward structures.

---

## Stream A: Blake3 (ASIC-Friendly)

### Algorithm Details

| Parameter | Value |
|-----------|-------|
| Algorithm | Blake3 |
| Target Block Time | 10 seconds |
| Block Reward | 50 IDAG |
| Difficulty Adjustment | Every 100 blocks |
| Hardware | ASICs, high-end CPUs |

### Purpose

Stream A provides the **base layer security** for the IronDAG network:
- ASIC-friendly algorithm ensures dedicated hardware can maximize efficiency
- 10-second blocks provide stable, predictable block production
- Higher rewards attract professional mining operations

### Mining Software

Currently, Stream A is mined internally by the node. External mining software support is planned.

### Expected Performance

| Hardware | Hash Rate | Blocks/Day | Daily Reward |
|----------|-----------|------------|--------------|
| Consumer CPU (8 cores) | ~50 MH/s | ~200 | 10,000 IDAG |
| High-end CPU (32 cores) | ~200 MH/s | ~800 | 40,000 IDAG |
| ASIC (projected) | ~10 GH/s+ | ~40,000 | 2,000,000 IDAG |

---

## Stream B: KHeavyHash (CPU/GPU)

### Algorithm Details

| Parameter | Value |
|-----------|-------|
| Algorithm | KHeavyHash (Kaspa variant) |
| Target Block Time | 5 seconds |
| Block Reward | 25 IDAG |
| Difficulty Adjustment | Every 60 blocks |
| Hardware | CPU, GPU (OpenCL) |

### Purpose

Stream B ensures **decentralized participation**:
- GPU-friendly algorithm allows consumer hardware to compete
- 5-second blocks provide fast confirmation times
- Lower barrier to entry for individual miners

### Mining Backends

The node supports multiple backends for Stream B:

| Backend | Status | Description |
|---------|--------|-------------|
| CPU | ✅ Available | Multi-threaded CPU mining |
| GPU | 🚧 Planned | OpenCL GPU acceleration |
| Auto | ✅ Available | Auto-select best backend |

### CPU Mining

Enable CPU mining (default):
```bash
./node --mining-backend cpu
```

The node automatically uses all available CPU cores. To limit cores:
```bash
# Use 4 cores only
RUST_MIN_THREADS=4 ./node
```

### GPU Mining (Planned)

When available, enable GPU mining:
```bash
./node --mining-backend gpu
```

Requirements:
- OpenCL 2.0+ compatible GPU
- AMD RX 5000 series or newer
- NVIDIA GTX 1000 series or newer

### Expected Performance

| Hardware | Hash Rate | Blocks/Day | Daily Reward |
|----------|-----------|------------|--------------|
| 4-core CPU | ~100 KH/s | ~1,000 | 25,000 IDAG |
| 8-core CPU | ~200 KH/s | ~2,000 | 50,000 IDAG |
| 16-core CPU | ~400 KH/s | ~4,000 | 100,000 IDAG |
| GPU (RX 6800) | ~5 MH/s | ~50,000 | 1,250,000 IDAG |
| GPU (RTX 3080) | ~8 MH/s | ~80,000 | 2,000,000 IDAG |

---

## Stream C: ZK Proofs (High Frequency)

### Algorithm Details

| Parameter | Value |
|-----------|-------|
| Algorithm | ZK-SNARK proofs (Groth16 on BN254) |
| Target Block Time | 1 second |
| Block Reward | Transaction fees only |
| Difficulty Adjustment | Dynamic |
| Hardware | High-core-count CPUs |

### Purpose

Stream C provides **maximum throughput**:
- Ultra-fast blocks for high-frequency transactions
- Fee-based rewards align miner incentives with network usage
- ZK proofs enable privacy-preserving transactions

### Enable Stream C

Stream C is disabled by default due to high CPU usage:

```bash
./node --enable-stream-c
```

### ZK Proving Requirements

To generate ZK proofs, you need proving keys:

```bash
# Generate keys (requires privacy feature)
cargo run --bin zk_setup --features privacy

# Or specify keys directory
./node --zk-keys-dir /path/to/keys
```

### Expected Performance

| Hardware | Proofs/Second | Blocks/Day | Daily Fees |
|----------|---------------|------------|------------|
| 8-core CPU | ~5 | ~40,000 | Variable |
| 16-core CPU | ~10 | ~80,000 | Variable |
| 32-core CPU | ~20 | ~160,000 | Variable |

---

## Hardware Requirements

### Minimum Requirements

For running a node without mining:
- **CPU**: 4 cores
- **RAM**: 8 GB
- **Storage**: 100 GB SSD
- **Network**: 10 Mbps

### BraidCore Mining (All 3 Streams)

| Component | Recommendation |
|-----------|---------------|
| **CPU** | 16+ cores (AMD Ryzen 9, Intel i9) |
| **RAM** | 32 GB DDR4/DDR5 |
| **Storage** | 500 GB NVMe SSD |
| **Network** | 100 Mbps, low latency |
| **GPU** | Optional (for future GPU mining) |

### Single-Stream Mode (Stream A Only)

For resource-constrained environments:
- **CPU**: 4 cores
- **RAM**: 8 GB
- **Storage**: 100 GB SSD

Enable with:
```bash
./node --single-stream
```

---

## Configuration

### Enable/Disable Mining

Disable all mining (RPC-only node):
```bash
./node --disable-mining
```

### Miner Address

Set the address that receives mining rewards:

```bash
./node --miner-address 0xYourAddressHere
```

Requirements:
- 40 hex characters
- Optional 0x prefix

Example:
```bash
./node --miner-address 0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

### Stream Selection

**BraidCore (default):**
```bash
./node
```

**Single-stream (Stream A only):**
```bash
./node --single-stream
```

**With Stream C enabled:**
```bash
./node --enable-stream-c
```

### Mining Backend

```bash
# CPU mining (default)
./node --mining-backend cpu

# GPU mining (when available)
./node --mining-backend gpu

# Auto-select
./node --mining-backend auto
```

### TOML Configuration

```toml
[mining]
miner_address = "0x0101010101010101010101010101010101010101"
enable_stream_a = true
enable_stream_b = true
enable_stream_c = false
mining_backend = "cpu"
```

### Mining Pause During IBD (Initial Block Download)

When a node starts and detects that a peer has significantly more blocks, it enters IBD. During this phase:

- All three BraidCore mining streams (A, B, C) are **automatically paused**
- Blocks are synced from peers without interference from local mining
- Mining **automatically resumes** when sync completes
- The pause is guaranteed by `scopeguard` — mining always resumes even if sync fails

This prevents DAG tip contamination, where locally-mined blocks create divergent DAG tips that become unresolvable orphans on peer nodes.

**Log output during IBD:**
```
⏸️ [MINING] Paused during initial block download
... (sync progress) ...
▶️ [MINING] Resumed after sync complete
```

**Note:** This behavior is automatic and cannot be disabled. It only activates during IBD when the peer has 50+ more blocks than the local node.

---

## Monitoring Mining Performance

### RPC Methods

#### irondag_getMiningStatus

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"irondag_getMiningStatus","params":[],"id":1}'
```

Response:
```json
{
  "jsonrpc": "2.0",
  "result": {
    "enabled": true,
    "streams": {
      "a": { "active": true, "blocks_mined": 1234 },
      "b": { "active": true, "blocks_mined": 5678 },
      "c": { "active": false }
    }
  },
  "id": 1
}
```

#### irondag_getMiningDashboard

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"irondag_getMiningDashboard","params":[3600],"id":1}'
```

Parameters:
- `durationSeconds`: Time window for statistics (default: 3600 = 1 hour)

Response includes:
- Blocks mined per stream
- Hash rates
- Rewards earned
- Efficiency metrics

### Node Logs

Monitor mining activity in node logs:

```bash
# With systemd
sudo journalctl -u irondag-node -f | grep -i mine

# Direct output
RUST_LOG=info ./node 2>&1 | grep -i mine
```

### DAG Statistics

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"irondag_getDagStats","params":[],"id":1}'
```

---

## Rewards and Economics

### Block Rewards

| Stream | Block Time | Reward | Daily Blocks | Daily Emission |
|--------|------------|--------|--------------|----------------|
| A | 10s | 50 IDAG | 8,640 | 432,000 IDAG |
| B | 5s | 25 IDAG | 17,280 | 432,000 IDAG |
| C | 1s | Fees | 86,400 | Variable |

### Total Daily Emission

- **Stream A + B**: ~864,000 IDAG/day
- **Annual emission**: ~315 million IDAG
- **Stream C**: Variable based on network usage

### Halving Schedule

Block rewards halve every **2 years** (approximately):

| Year | Stream A Reward | Stream B Reward | Daily Emission |
|------|-----------------|-----------------|----------------|
| 1-2 | 50 IDAG | 25 IDAG | ~2.59M IDAG |
| 3-4 | 25 IDAG | 12.5 IDAG | ~1.30M IDAG |
| 5-6 | 12.5 IDAG | 6.25 IDAG | ~648K IDAG |

### Fee Market

Transaction fees are distributed to Stream C miners:
- Base fee: 20 gwei per gas unit
- Priority fee: Optional tip for faster inclusion
- Fee burning: Not implemented (unlike EIP-1559)

---

## Pool Mining

### Solo vs Pool Mining

| Aspect | Solo Mining | Pool Mining |
|--------|-------------|-------------|
| Rewards | Irregular, full block reward | Regular, proportional shares |
| Variance | High | Low |
| Minimum Hardware | High | Low |
| Fees | None | 1-3% |
| Current Status | Supported | Planned |

### Planned Pool Features

Future releases will support:
- Stratum protocol compatibility
- PPLNS (Pay Per Last N Shares) payout scheme
- Variable difficulty shares
- Real-time statistics API

---

## Optimization Tips

### CPU Optimization

1. **Disable hyper-threading** for mining-dedicated machines
2. **Use performance governor**: `cpufreq-set -g performance`
3. **Isolate mining cores**: Use `taskset` to dedicate cores
4. **Enable huge pages**: Improves memory access

```bash
# Enable huge pages
sudo sysctl -w vm.nr_hugepages=128

# Set CPU governor
sudo cpufreq-set -g performance

# Isolate cores 0-7 for mining
sudo taskset -c 0-7 ./node
```

### Memory Optimization

1. **Close unnecessary applications**
2. **Disable swap** for mining-only machines
3. **Use fast RAM** (DDR4-3200+ or DDR5)

### Network Optimization

1. **Use wired connection** instead of WiFi
2. **Enable port forwarding** for P2P (port 8080 default)
3. **Use low-latency DNS**

### Storage Optimization

1. **Use NVMe SSD** for data directory
2. **Enable sled high-throughput mode** (default)
3. **Separate data directory** from OS drive

---

## Troubleshooting

### High CPU Usage

**Problem**: Node using 100% CPU

**Solutions**:
1. Enable single-stream mode:
   ```bash
   ./node --single-stream
   ```

2. Disable Stream C:
   ```bash
   # Ensure enable_stream_c = false in config
   ```

3. Limit CPU cores:
   ```bash
   taskset -c 0-3 ./node
   ```

### No Blocks Mined

**Problem**: Mining for hours with no rewards

**Check**:
1. Verify miner address is set:
   ```bash
   curl -X POST http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"eth_getBalance","params":["0xYourAddress","latest"],"id":1}'
   ```

2. Check difficulty vs your hash rate:
   ```bash
   curl -X POST http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"irondag_getDagStats","params":[],"id":1}'
   ```

3. Ensure node is synced:
   ```bash
   curl -X POST http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   ```

### Memory Errors

**Problem**: Out of memory crashes

**Solutions**:
1. Reduce sled cache:
   ```toml
   sled_cache_mb = 128
   ```

2. Enable more aggressive pruning:
   ```toml
   prune_interval_secs = 30
   keep_red_blocks = false
   ```

3. Add more RAM or enable swap

### Network Connection Issues

**Problem**: Low peer count

**Solutions**:
1. Add bootstrap peers:
   ```bash
   ./node --bootstrap-peer 192.168.1.100:8080
   ```

2. Enable port forwarding on router
3. Check firewall rules:
   ```bash
   sudo ufw allow 8080/tcp
   ```

### ZK Proof Failures (Stream C)

**Problem**: Stream C not producing blocks

**Check**:
1. ZK keys are present:
   ```bash
   ls /path/to/zk-keys/
   # Should contain: proving_key.bin, verifying_key.bin
   ```

2. Generate keys if missing:
   ```bash
   cargo run --bin zk_setup --features privacy
   ```

3. Verify feature is enabled:
   ```bash
   cargo build --release --features privacy
   ```

---

## Security Considerations

1. **Secure your private keys** - Never share miner private keys
2. **Use firewall rules** - Only expose necessary ports
3. **Keep software updated** - Apply security patches promptly
4. **Monitor for anomalies** - Watch for unexpected hash rate drops
5. **Backup wallet** - Store miner address private key securely

---

## FAQ

**Q: Can I mine on a laptop?**
A: Yes, but use single-stream mode (`--single-stream`) to prevent overheating.

**Q: How do I know if I'm mining successfully?**
A: Check `irondag_getMiningDashboard` RPC method for block counts and rewards.

**Q: Can I mine to an exchange address?**
A: Yes, but ensure the exchange supports IDAG deposits.

**Q: What's the minimum payout?**
A: There is no minimum - rewards are credited directly to your miner address.

**Q: Can I run multiple miners on one machine?**
A: Yes, but use different data directories and ports for each instance.

**Q: Is GPU mining available?**
A: CPU mining is available now. GPU mining is planned for a future release.

**Q: How do I calculate profitability?**
A: Use the formula:
```
Daily Profit = (Your Hash Rate / Network Hash Rate) × Daily Emission × Price - Electricity Cost
```
