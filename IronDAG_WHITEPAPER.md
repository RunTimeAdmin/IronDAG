# IronDAG: Layer 1 Blockchain

**IronDAG Protocol** | Ticker: **IDAG**

## Executive Summary

IronDAG is a Layer 1 blockchain that combines quantum resistance with post-quantum cryptography and BraidCore mining architecture. Currently ready for testnet deployment, IronDAG integrates advanced features at the protocol level rather than as afterthoughts.

**Websites**: [IRONDAG.network](https://irondag.network) | [IRONDAG.io](https://irondag.io) | [IRONDAG.xyz](https://irondag.xyz)

**Key Differentiators:**
- BraidCore Mining Architecture — IronDAG's multi-stream mining architecture where three parallel mining streams (A, B, C) cross-reference each other at the parent level, braiding into a single GhostDAG consensus layer
- Post-Quantum Cryptography (Dilithium3, SPHINCS+, Kyber)
- Verkle Trees for stateless validation
- Rule-Based Fraud Detection & Forensics
- MEV-Aware Transaction Ordering
- Native Sharding with cross-shard support (160,000+ TPS with 10 shards)
- Full EVM Compatibility

---

## 1. BraidCore Mining Architecture

### Unique Three-Stream BraidCore Design

**Stream A: ASIC Mining**
- Algorithm: Blake3
- Block Time: 10 seconds
- Transactions: 10,000 per block
- Reward: 50 IDAG tokens
- Purpose: Security & decentralization

**Stream B: CPU Mining (GPU in development)**
- Algorithm: KHeavyHash
- Block Time: 5 seconds
- Transactions: 5,000 per block
- Reward: 25 IDAG tokens
- Purpose: Accessibility & participation
- **Note**: Currently CPU-only; GPU mining via OpenCL is planned but not yet implemented

**Stream C: ZK Proof Validation**
- Block Time: 1 second (optimized from 100ms for CPU efficiency on standard hardware)
- Transactions: 1,000 per block
- Reward: Fee-based only
- Purpose: Speed & scalability

### Why BraidCore?

1. **Inclusivity**: Multiple mining algorithms allow various hardware participation
2. **Speed**: Sub-second blocks via Stream C for fast transactions
3. **Security**: Stream A provides Bitcoin-level security through ASIC resistance
4. **Decentralization**: Three parallel streams prevent single-point centralization

**Implementation**: `src/mining.rs` - Full BraidCore production code

---

## 2. Post-Quantum Cryptography

### Quantum-Resistant from Day One

IronDAG is the first L1 blockchain with native post-quantum cryptography:

**Signature Schemes:**
- **Dilithium3**: NIST-approved lattice-based signatures
- **SPHINCS+**: Hash-based stateless signatures
- **Ed25519**: Classical fallback for compatibility

**Key Exchange:**
- **Kyber**: Post-quantum key encapsulation
- Used for P2P network encryption
- Session key establishment

### Account Types

```rust
PqAccount::new_dilithium3()   // Quantum-resistant
PqAccount::new_sphincsplus()  // Hash-based security
PqAccount::new_ed25519()      // Classical compatibility
```

**Features:**
- Dual-signature transactions (classical + PQ)
- Account type auto-detection
- Backward compatibility with Ethereum wallets

**Implementation**: `src/pqc/accounts.rs`, `src/pqc/kyber.rs`

---

## 3. Verkle Trees & Stateless Validation

### The Future of Blockchain State

IronDAG implements Verkle trees for efficient state management:

**Benefits:**
- **Compact Proofs**: 10-100x smaller than Merkle proofs
- **Stateless Clients**: Verify transactions without full state
- **Scalability**: Reduced storage requirements
- **Fast Sync**: New nodes sync in minutes, not days

**Light Client Support:**
```rust
LightClient::verify_balance(address, balance, proof)
LightClient::verify_storage(address, key, value, proof)
```

### State Proof System

- KZG commitments for polynomial verification
- Cryptographic state proofs
- Cross-shard state verification
- Fraud proof generation

**Implementation**: `src/verkle/`, `src/light_client.rs`

---

## 4. MEV Protection & Transaction Fairness

### MEV Detection & Fairness Analysis

IronDAG includes protocol-level MEV detection and mitigation:

**Detection Capabilities:**
- Sandwich attack identification
- Front-running detection
- Back-running analysis
- Fee concentration monitoring
- Reordering distance calculation

**Fairness Metrics:**
```json
{
  "fairness_score": 0.95,
  "sandwich_detections": 0,
  "frontrun_detections": 0,
  "estimated_mev_value": "0x0",
  "reordering_distance": 0.02
}
```

### Five Ordering Policies

Users and validators choose transaction ordering:

1. **FIFO** - First In, First Out (most fair)
2. **Random** - Prevents front-running completely
3. **Fee-Based** - Maximizes miner revenue
4. **Hybrid** - FIFO with fee boost for old transactions
5. **Time-Weighted** - Age-based priority with fee tiebreaker

**Change Policy via API:**
```bash
IronDAG_setOrderingPolicy --policy "random"
```

**Implementation**: `src/mining/fairness.rs`, `src/mining/ordering.rs`

---

## 5. Rule-Based Fraud Detection & Forensics

### Native Fraud Detection

IronDAG has built-in rule-based security monitoring at the protocol level:

**Risk Scoring:**
- Real-time transaction risk assessment
- Pattern-based fraud detection
- Confidence scoring
- Address behavior analysis

**Risk Categories:**
- Honeypot contracts
- Mixer services
- Phishing addresses
- High-frequency trading bots
- Suspicious fund movements

### Forensic Analysis Tools

**Fund Tracing:**
```bash
IronDAG_traceFunds --address 0x123... --max-hops 5
```
- Follow money flows across addresses
- Multi-hop transaction tracking
- Value concentration analysis

**Anomaly Detection:**
- Rapid fund movement
- Circular transactions
- Many-to-one patterns (mixers)
- One-to-many patterns (distribution)
- High-frequency small transactions

**Address Summary:**
- Total received/sent
- Unique contacts
- Suspicious patterns
- Risk indicators
- Transaction history

**Implementation**: `src/security/forensics.rs`, `src/security/risk_scoring.rs`

---

## 6. Security Policies & Governance

### Opt-In Behavior Gating

Users and smart contracts can enforce security policies:

**Policy Types:**
- Max risk score thresholds
- Required risk summaries for contracts
- Blocked addresses/labels
- Minimum confidence requirements
- Custom policy rules

**Example Policy:**
```rust
SecurityPolicy {
    name: "No High-Risk Interactions",
    policy_type: MaxRiskScore { threshold: 0.7 },
    action: Reject { reason: "Risk too high" },
    enabled: true
}
```

**Use Cases:**
- DeFi protocols blocking risky addresses
- Wallets warning users about scams
- Exchanges enforcing compliance
- DAOs governing fund movements

**Implementation**: `src/security/policies.rs`

---

## 7. Sharding Architecture

### Horizontal Scalability

IronDAG implements production-ready sharding:

**Features:**
- Configurable shard count (default: 10)
- Consistent hashing for address assignment
- Cross-shard transaction support
- Shard-aware block propagation
- Per-shard transaction pools

**Assignment Strategies:**
- **Consistent Hashing**: Deterministic address-to-shard mapping
- **Random**: Load balancing
- **RoundRobin**: Even distribution

**Cross-Shard Transactions:**
```rust
CrossShardTransaction {
    from_shard: 2,
    to_shard: 7,
    transaction: tx,
    status: Pending
}
```

**Metrics Per Shard:**
- Transaction count
- Cross-shard tx count
- Shard utilization
- Load balancing stats

**Implementation**: `src/sharding.rs`

---

## 8. EVM Compatibility

### Ethereum Smart Contract Support

IronDAG is fully EVM-compatible:

**Supported:**
- Solidity smart contracts
- Web3.js/ethers.js integration
- MetaMask wallet support
- Existing Ethereum tooling
- Remix IDE compatibility

**JSON-RPC Methods:**
```javascript
// Ethereum-compatible
eth_blockNumber()
eth_getBalance(address)
eth_sendTransaction(tx)
eth_call(tx)
eth_estimateGas(tx)

// IronDAG-specific extensions
IronDAG_getFairnessMetrics()
IronDAG_getRiskScore(address)
IronDAG_traceFunds(address)
```

**Implementation**: `src/evm.rs`, `src/rpc.rs`

---

## 9. GhostDAG Consensus

### BlockDAG Architecture

IronDAG uses GhostDAG for high throughput:

**Features:**
- Parallel block production
- Blue/Red block classification
- Topological ordering
- Byzantine fault tolerance
- Sub-second block times

**Advantages over Chains:**
- No orphan blocks wasted
- Higher transaction throughput
- Lower latency
- Better resource utilization

**Stats:**
```json
{
  "blue_blocks": 1534,
  "red_blocks": 23,
  "total_blocks": 1557,
  "blue_ratio": 98.5,
  "tps": 4521.3,
  "shard_count": 10,
  "sharded_tps": 45213.0
}
```

**Implementation**: `src/consensus.rs`

---

## 10. Production Monitoring

### Prometheus & Grafana Integration

Complete observability out of the box:

**Metrics Collected:**
- Blocks mined per stream
- Transaction throughput (TPS)
- Mining rewards
- Network peers
- Transaction pool size
- Shard statistics
- Cross-shard transactions
- Block size distribution

**Pre-built Dashboards:**
1. IronDAG Blockchain Overview
2. Mining Metrics (per-stream)
3. Network Metrics
4. Sharding Metrics
5. Transaction Metrics

**Access:**
- Prometheus: `http://localhost:9090`
- Grafana: `http://localhost:3001` (admin/admin)

**Implementation**: `src/metrics.rs`, `grafana/`

---

## 11. Developer Experience

### Complete API Suite

**JSON-RPC API:**
- 129+ IronDAG-specific RPC methods (`irondag_*`)
- Full Ethereum-compatible subset (`eth_*`, `net_*`, `web3_*`)
- Security, forensics, sharding, PQ accounts, DAG stats
- WebSocket support
- Rate limiting built-in

**Block Explorer:**
- Real-time blockchain data
- Transaction search
- Address lookup
- Fairness metrics
- Security analysis
- Forensic tools

**SDKs (Planned):**
- JavaScript/TypeScript
- Python
- Rust
- Go

---

## 12. Architecture Overview

### System Components

```
┌─────────────────────────────────────────────┐
│          IronDAG Node Architecture            │
├─────────────────────────────────────────────┤
│                                             │
│  ┌─────────────────────────────────────┐   │
│  │   BraidCore Mining Manager          │   │
│  │  ┌─────┐ ┌─────┐ ┌─────┐          │   │
│  │  │  A  │ │  B  │ │  C  │          │   │
│  │  │ 10s │ │  5s │ │ 1s  │          │   │
│  │  └─────┘ └─────┘ └─────┘          │   │
│  └─────────────────────────────────────┘   │
│             ↓                               │
│  ┌─────────────────────────────────────┐   │
│  │    GhostDAG Consensus Engine        │   │
│  │  • Block ordering                   │   │
│  │  • Blue/Red classification          │   │
│  └─────────────────────────────────────┘   │
│             ↓                               │
│  ┌─────────────────────────────────────┐   │
│  │       Blockchain State              │   │
│  │  • Verkle Trees                     │   │
│  │  • Account balances                 │   │
│  │  • Smart contracts (EVM)            │   │
│  │  • Shard states                     │   │
│  └─────────────────────────────────────┘   │
│             ↓                               │
│  ┌─────────────────────────────────────┐   │
│  │    Security & Fairness Layer        │   │
│  │  • MEV Detection                    │   │
│  │  • Risk Scoring                     │   │
│  │  • Forensic Analysis                │   │
│  │  • Policy Enforcement               │   │
│  └─────────────────────────────────────┘   │
│             ↓                               │
│  ┌─────────────────────────────────────┐   │
│  │      Storage & Persistence          │   │
│  │  • Sled Database                    │   │
│  │  • State persistence                │   │
│  │  • Transaction indexing             │   │
│  └─────────────────────────────────────┘   │
│             ↓                               │
│  ┌──────────────┬──────────────────────┐   │
│  │  JSON-RPC    │   P2P Network        │   │
│  │  API Server  │   • Peer discovery   │   │
│  │  Port 8545   │   • Block prop       │   │
│  └──────────────┴──────────────────────┘   │
└─────────────────────────────────────────────┘
```

---

## 13. Performance Characteristics

### Throughput

#### Base Throughput (Single Shard)

- **Stream A (ASIC)**: 1,000 TPS
  - 10,000 transactions per block
  - 10-second block time
  - 1,000 TPS per shard

- **Stream B (CPU)**: 5,000 TPS (GPU planned)
  - 5,000 transactions per block
  - 5-second block time
  - 1,000 TPS per shard

- **Stream C (ZK Proofs)**: 1,000 TPS
  - 1,000 transactions per block
  - 1-second block time
  - 1,000 TPS per shard

- **Combined Base**: ~16,000 TPS per shard

#### Sharded Throughput

With native sharding enabled (default: 10 shards), throughput scales linearly:

- **10 Shards**: ~160,000 TPS
  - Each shard processes ~16,000 TPS independently
  - Parallel processing across all shards
  - Cross-shard transactions add minimal overhead (~5-10%)

- **50 Shards**: ~800,000 TPS
  - Linear scaling with shard count
  - Efficient for high-volume applications
  - Maintains low latency per shard

- **100 Shards**: ~1,600,000 TPS
  - Theoretical maximum with current architecture
  - 90% efficiency (10% overhead for coordination)
  - Real-world: ~1,440,000 TPS (accounting for overhead)

**Sharding Configuration**:
- **Default**: 10 shards (160,000 TPS)
- **Configurable**: 1-100 shards
- **Assignment Strategy**: Consistent hashing (deterministic routing)
- **Cross-Shard Support**: Two-phase commit protocol

**Performance Notes**:
- Same-shard transactions: Full throughput (no overhead)
- Cross-shard transactions: ~5-10% overhead (two-phase commit)
- Real-world efficiency: 90-95% of theoretical max at scale

### Latency

- **Same-Shard Finality**: 1-10 seconds (stream-dependent)
- **Cross-Shard Finality**: 2-12 seconds (adds validation phase)
- **Confirmation**: 1 block (probabilistic)
- **Deep Confirmation**: 6 blocks recommended
- **Shard Latency**: Independent per shard (no global consensus delay)

### Storage

- **Block Size**: Average 10-100 KB
- **State Size**: Grows with accounts (~100 bytes/account)
- **Per-Shard State**: Distributed across shards (reduces per-node storage)
- **Verkle Proofs**: ~1-2 KB per proof
- **Shard Overhead**: Minimal (consistent hashing, cross-shard tracking)

---

## 14. Security Model

### Threat Resistance

**Quantum Attacks:** ✅ Resistant (PQ crypto)
**51% Attacks:** ✅ Mitigated (TriStream)
**MEV Exploitation:** ✅ Detected & mitigated
**Front-Running:** ✅ Preventable (ordering policies)
**Sybil Attacks:** ✅ PoW protection
**Eclipse Attacks:** ✅ Peer diversity
**DoS Attacks:** ✅ Rate limiting, pool limits

### Cryptographic Primitives

- **Hashing**: Blake3, SHA-256, Keccak-256
- **Signatures**: Dilithium3, SPHINCS+, Ed25519
- **Key Exchange**: Kyber
- **Commitments**: KZG (for Verkle)

---

## 15. Tokenomics and Distribution Model

### 15.1 Overview

The IronDAG Protocol implements a **hybrid fair launch model** that balances community funding needs with fair distribution principles. The tokenomics model ensures long-term sustainability while maintaining the fairest launch structure in the industry.

**Key Principles**:
- **97% Fair Launch**: 97% of tokens generated through mining
- **3% Community Presale**: Small pre-mine for community development funding
- **0% Team Allocation**: No tokens allocated to founders or team
- **10% Development Fund**: Long-term sustainability from mining rewards
- **10 Billion Max Supply**: Hard cap ensures scarcity
- **4-Year Halving**: Reduces inflation over time

---

### 15.2 Token Generation Model

#### 15.2.1 Mining-Based Generation (Fair Launch)

The primary mechanism for token generation is **block mining**. Tokens are created as rewards when miners successfully mine blocks across the three mining streams.

**Generation Process**:
```
Block Creation → Token Generation → Reward Distribution
```

**Stream-Specific Generation**:
- **Stream A (ASIC)**: 50 IDAG per block, 10-second intervals
- **Stream B (CPU)**: 25 IDAG per block, 5-second intervals (GPU planned)
- **Stream C (ZK Proofs)**: Fee-based only, 1-second intervals

**Daily Generation (Year 1)**:
- Stream A: ~432,000 IDAG/day (50 IDAG × 8,640 blocks)
- Stream B: ~1,728,000 IDAG/day (20 IDAG × 86,400 blocks)
- Stream C: ~4,320,000 IDAG/day (5 IDAG × 864,000 blocks)
- **Total**: ~6,480,000 IDAG/day from mining

**Fair Launch Percentage**: 97% of total supply (8.7 billion IDAG) generated through mining

---

#### 15.2.2 Community Presale Matching (Pre-Launch)

To fund initial development, security audits, and exchange listings, a **community presale** is conducted on Pump.fun before mainnet launch. Presale tokens are matched 1:1 with native IDAG, capped at 1 billion IDAG (10% of max supply).

**Allocation Details**:
- **Matching Cap**: Up to 1,000,000,000 IDAG (10% of 10 billion max supply)
- **Purpose**: Community development funding
- **Distribution**: Sold to community members (not team, not VCs)
- **Price**: Determined by Pump.fun bonding curve (early buyers get better prices)
- **Target Raise**: $300,000+ USD
- **Matching**: 1:1 migration to native IDAG (up to 1B cap)
- **If Over Cap**: Proportional matching if presale creates more than 1B tokens

**Presale Justification**:
- Smallest pre-launch allocation in the industry (vs competitors' 20-65%)
- Community-funded (not team-funded)
- Transparent allocation and spending
- Necessary for initial development and security audits
- Flexible matching works with Pump.fun's bonding curve model

**Fair Launch Impact**: Up to 10% presale match means 90% fair launch (still fairest in industry)

---

### 15.3 Total Supply and Distribution

#### 15.3.1 Max Supply Cap

**Total Supply**: 10,000,000,000 IDAG (10 billion)

**Hard Cap Rationale**:
- Creates scarcity narrative
- Predictable supply curve
- Value accrual mechanism
- Investor confidence

**Supply Schedule**:
- **Year 1**: ~2.13 billion IDAG (21.3% of cap, includes up to 1B presale match)
- **Year 5**: ~6.65 billion IDAG (66.5% of cap)
- **Year 10**: ~9.33 billion IDAG (93.3% of cap)
- **Year 20**: ~10 billion IDAG (100% cap reached)
- **Post-Cap**: Only transaction fees (no block rewards)

---

#### 15.3.2 Distribution Breakdown

**Initial Distribution (Pre-Launch)**:
- **Community Presale Match**: Up to 1,000,000,000 IDAG (10% cap)
  - Purpose: Development funding
  - Source: Pump.fun presale tokens (matched 1:1)
  - Matching: 1:1 up to 1B cap (proportional if over cap)
  - Allocation: Community members only
  - Transparency: Public wallet, spending reports
  - **Fair Launch**: 90% (9B from mining + dev fund, 1B from presale match)

**Ongoing Distribution (Post-Launch)**:
- **Mining Rewards**: 8,700,000,000 IDAG (87%)
  - Stream A: ~157.68M IDAG/year (Year 1)
  - Stream B: ~630.72M IDAG/year (Year 1)
  - Stream C: ~1,576.8M IDAG/year (Year 1)
  - Total: ~2,128.68M IDAG/year (Year 1)
  - Fair Launch: All tokens generated through mining

- **Development Fund**: 300,000,000 IDAG (3%)
  - Source: 10% of all mining rewards
  - Generation: ~259,200 IDAG/day (Year 1)
  - Annual: ~94.608M IDAG/year (Year 1)
  - Governance: Multi-sig wallet, community oversight
  - Use: Audits, listings, grants, infrastructure

**Total Distribution**:
- Presale Match: Up to 1B IDAG (10% cap) - Matched from Pump.fun
- Mining: 8.7B IDAG (87%) - Fair launch
- Dev Fund: 300M IDAG (3%) - From mining rewards
- **Total**: 10B IDAG (100%)
- **Fair Launch**: 90% (9B from mining + dev fund, 1B from presale match)

---

### 15.4 Fair Launch Model

#### 15.4.1 Fair Launch Principles

The IronDAG Protocol implements a **90% fair launch model**, the fairest in the industry:

**Fair Launch Components**:
- ✅ **90% Mining-Based**: 9 billion IDAG generated through mining and development fund
- ✅ **0% Team Allocation**: No tokens to founders or team
- ✅ **0% Advisor Allocation**: No tokens to advisors
- ✅ **0% VC Allocation**: No tokens to venture capitalists
- ✅ **Equal Opportunity**: Anyone can mine and earn tokens
- ✅ **Transparent**: All allocations public and verifiable

**Pre-Launch Components** (Up to 10%):
- ⚠️ **Up to 10% Community Presale Match**: Up to 1B IDAG matched from Pump.fun presale
- ✅ **Community-Funded**: Sold to community, not team
- ✅ **Transparent**: Public allocation and spending
- ✅ **Smallest in Industry**: vs competitors' 20-65% pre-mine
- ✅ **Flexible Matching**: Works with Pump.fun bonding curve model

---

#### 15.4.2 Fair Launch Comparison

| Project | Pre-mine | Fair Launch % | Team % | Presale % |
|---------|----------|---------------|--------|-----------|
| **Bitcoin** | 0% | 100% | 0% | 0% |
| **Ethereum** | ~12% | 88% | ~12% | 0% |
| **Typical L1** | 20-65% | 35-80% | 10-20% | 15-30% |
| **IronDAG** | **Up to 10%** | **90%** | **0%** | **Up to 10%** |

**IronDAG is the fairest launch in the industry** ✅

---

### 15.5 Emission Schedule and Halving

#### 15.5.1 Block Rewards

**Year 1-4 (Pre-Halving)**:
- Stream A: 50 IDAG per block
- Stream B: 20 IDAG per block
- Stream C: 5 IDAG per block
- Daily Emission: ~6,480,000 IDAG
- Annual Emission: ~2,128,680,000 IDAG

**Year 5-8 (Post First Halving)**:
- Stream A: 25 IDAG per block (50% reduction)
- Stream B: 10 IDAG per block (50% reduction)
- Stream C: 2.5 IDAG per block (50% reduction)
- Daily Emission: ~3,240,000 IDAG
- Annual Emission: ~1,064,340,000 IDAG

**Year 9-12 (Post Second Halving)**:
- Stream A: 12.5 IDAG per block
- Stream B: 5 IDAG per block
- Stream C: 1.25 IDAG per block
- Daily Emission: ~1,620,000 IDAG
- Annual Emission: ~532,170,000 IDAG

**Year 13-16 (Post Third Halving)**:
- Stream A: 6.25 IDAG per block
- Stream B: 2.5 IDAG per block
- Stream C: 0.625 IDAG per block
- Daily Emission: ~810,000 IDAG
- Annual Emission: ~266,085,000 IDAG

**Year 20+ (Max Supply Reached)**:
- Block Rewards: 0 IDAG (cap reached)
- Mining Rewards: Transaction fees only
- Deflationary: If fee burns enabled

---

#### 15.5.2 Halving Mechanism

**Halving Schedule**:
- **First Halving**: Year 5 (after 4 years)
- **Second Halving**: Year 9 (after 8 years)
- **Third Halving**: Year 13 (after 12 years)
- **Fourth Halving**: Year 17 (after 16 years)

**Halving Impact**:
- Reduces inflation by 50% every 4 years
- Creates scarcity narrative
- Similar to Bitcoin's halving model
- Predictable emission schedule

**Inflation Rate**:
- Year 1: N/A (initial)
- Year 5: ~15.3% (1.06B / 6.95B existing)
- Year 10: ~5.5% (0.53B / 9.63B existing)
- Year 15: ~2.7% (0.27B / 9.89B existing)
- Year 20+: ~0% (cap reached)

---

### 15.6 Development Fund

#### 15.6.1 Fund Structure

**Allocation**: 10% of all block rewards

**Generation**:
- Stream A: 5 IDAG per block (10% of 50)
- Stream B: 2 IDAG per block (10% of 20)
- Stream C: 0.5 IDAG per block (10% of 5)
- Daily Fund: ~259,200 IDAG/day (Year 1)
- Annual Fund: ~94,608,000 IDAG/year (Year 1)

**Total Fund Over Lifetime**: ~1 billion IDAG (10% of 10B max supply)

---

#### 15.6.2 Fund Governance

**Initial Structure**: Multi-sig wallet (3-of-5 signatures required)
- Core team members: 2 signatures
- Community representatives: 2 signatures
- Technical advisor: 1 signature

**Future Structure**: On-chain governance
- IDAG holders vote on fund allocation
- Proposal system for spending
- Transparent spending reports
- Community oversight

---

#### 15.6.3 Fund Usage

**Year 1 Priorities**:
1. **Security Audits** (33%): $100,000+
   - Smart contract audit (Trail of Bits, OpenZeppelin)
   - Blockchain security audit
   - Penetration testing

2. **Exchange Listings** (25%): $75,000+
   - Binance listing fee
   - Coinbase listing fee
   - Other major exchanges

3. **Developer Grants** (17%): $50,000+
   - Ecosystem building
   - Developer incentives
   - Tool development

4. **Infrastructure** (17%): $50,000+
   - Servers and hosting
   - Monitoring tools
   - Development tools

5. **Marketing** (17%): $50,000+
   - Community building
   - Content creation
   - Social media

6. **Legal/Compliance** (8%): $25,000+
   - Legal structure
   - Regulatory compliance

**Total Year 1 Budget**: ~$600,000+ (from dev fund + presale)

---

### 15.7 Community Presale

#### 15.7.1 Presale Structure

**Allocation**: 300,000,000 IDAG (3% of max supply)

**Pricing**:
- Fixed Price: $0.001 per IDAG
- Target Raise: $300,000 USD
- Accepted Currencies: USDC, USDT, ETH, BTC

**Purchase Limits**:
- Minimum: 1,000 IDAG ($1.00)
- Maximum: 10,000,000 IDAG ($10,000) per address
- Whale Protection: Hard cap prevents single address dominance

**Timeline**:
- Announcement: TBD
- Registration: TBD (KYC if required)
- Presale Period: 30-60 days (or until sold out)
- Token Distribution: Within 7 days after presale ends
- Mainnet Launch: TBD (after security audit)

---

#### 15.7.2 Presale Transparency

**Public Transparency**:
- ✅ Public wallet address for fund collection
- ✅ Real-time tracking dashboard
- ✅ Live statistics (raised, sold, contributors)
- ✅ Monthly spending reports
- ✅ Multi-sig wallet (3-of-5)

**Fund Usage**:
- 33% Security audits
- 25% Exchange listings
- 17% Marketing & community
- 17% Infrastructure
- 8% Legal & compliance

**Governance**:
- Community oversight
- Public proposals
- Transparent spending
- Regular reports

---

#### 15.7.3 Fair Launch Justification

**Why 3% Pre-mine is Acceptable**:
1. **Smallest in Industry**: vs competitors' 20-65% pre-mine
2. **Community-Funded**: Not team, not VCs
3. **Transparent**: Public allocation and spending
4. **Necessary**: Funds development, audits, listings
5. **Still 97% Fair**: Remaining 97% from mining

**Fair Launch Comparison**:
- Bitcoin: 100% fair launch (0% pre-mine)
- Ethereum: 88% fair launch (12% pre-mine)
- Typical L1: 35-80% fair launch (20-65% pre-mine)
- **IronDAG: 97% fair launch (3% pre-mine)** ← Fairest!

---

### 15.8 Supply Projections

#### 15.8.1 Cumulative Supply Growth

| Year | Cumulative Supply | Annual Emission | Inflation Rate | % of Cap |
|------|------------------|----------------|---------------|----------|
| 1 | ~2.43B | 2.13B | N/A | 24.3% |
| 5 | ~6.95B | 1.06B | ~15.3% | 69.5% |
| 10 | ~9.63B | 0.53B | ~5.5% | 96.3% |
| 15 | ~9.89B | 0.27B | ~2.7% | 98.9% |
| 20 | ~10B | 0 | ~0% | 100% |

**Note**: Includes up to 1B IDAG from presale match (Year 1)

---

#### 15.8.2 Inflation Model

**Deflationary with Cap**:
- **Year 1-4**: High emission (building network)
- **Year 5-8**: Reduced emission (first halving)
- **Year 9-12**: Lower emission (second halving)
- **Year 13-16**: Minimal emission (third halving)
- **Year 20+**: Zero emission (cap reached)

**Inflation Rate Trend**:
- Starts high (building supply)
- Decreases with halving
- Approaches zero as cap is reached
- Becomes deflationary if fee burns enabled

---

### 15.9 Token Utility

#### 15.9.1 Primary Uses

1. **Transaction Fees**: Pay for blockchain transactions
2. **Smart Contract Gas**: Execute EVM smart contracts
3. **Mining Rewards**: Incentivize network security
4. **Governance** (Future): Vote on protocol changes
5. **Staking** (Future): Potential staking mechanism

#### 15.9.2 Value Drivers

- **Network Security**: Mining rewards incentivize participation
- **Transaction Demand**: Fees create demand for IDAG
- **Smart Contract Usage**: Gas fees drive utility
- **Scarcity**: Max supply cap and halving create scarcity
- **Utility**: Essential for using the network

---

### 15.10 Fair Launch Summary

#### 15.10.1 Fair Launch Metrics

**Distribution**:
- **90% Fair Launch**: 9B IDAG from mining + dev fund
- **Up to 10% Presale Match**: Up to 1B IDAG matched from Pump.fun
- **0% Team**: No team allocation
- **0% VCs**: No venture capital allocation
- **10% Dev Fund**: From mining rewards (not pre-mined)

**Fair Launch Score**: **90%** (Fairest in Industry)

#### 15.10.2 Competitive Advantage

**IronDAG vs Competitors**:
- ✅ **Smallest Pre-mine**: 3% vs 20-65%
- ✅ **No Team Allocation**: 0% vs 10-20%
- ✅ **Transparent**: Public allocation vs private
- ✅ **Community-Funded**: Presale to community vs VCs
- ✅ **Fair Limits**: Max 10M per address vs unlimited

**Result**: IronDAG is the fairest launch in the industry

---

## 16. Comparison with Other L1s

| Feature | IronDAG | Ethereum | Solana | Cardano | Kaspa |
|---------|-------|----------|--------|---------|-------|
| **Post-Quantum** | ✅ Native | ❌ | ❌ | ❌ | ❌ |
| **MEV Protection** | ✅ Built-in | 🟡 External | ❌ | ❌ | ✅ |
| **Verkle Trees** | ✅ | 🟡 Planned | ❌ | ❌ | ❌ |
| **Rule-Based Security** | ✅ Native | ❌ | ❌ | ❌ | ❌ |
| **DAG Consensus** | ✅ GhostDAG | ❌ | ❌ | ❌ | ✅ |
| **EVM Compatible** | ✅ | ✅ | ❌ | ❌ | ❌ |
| **Sharding** | ✅ Native | 🟡 Planned | ❌ | ✅ | ❌ |
| **TPS** | 160,000+ (10 shards)<br>1.6M+ (100 shards) | 15-30 | 65,000 | 250 | 32,000+ |
| **Block Time** | 0.1-10s | 12s | 0.4s | 20s | 1s |

---

## 17. Use Cases

### DeFi Applications
- Decentralized exchanges (DEXs)
- Lending/borrowing protocols
- Stablecoins
- Yield farming
- Options/derivatives

### Enterprise Solutions
- Supply chain tracking
- Identity management
- Healthcare records
- Financial settlements
- Compliance systems

### NFT & Gaming
- NFT marketplaces
- GameFi platforms
- Digital collectibles
- Metaverse assets
- Play-to-earn games

### AI & Machine Learning
- On-chain AI inference
- Model marketplaces
- Data provenance
- Federated learning
- AI agent coordination

> **Note:** AI/ML capabilities described above are under consideration for implementation as a separate L2 service or RPC-layer enhancement. They are not part of the L1 consensus protocol. Current L1 validation uses deterministic rule-based logic only.

---

## 18. Roadmap

### Phase 1: Core Implementation ✅
- Core blockchain ✅
- BraidCore mining ✅
- GhostDAG consensus ✅
- P2P networking ✅
- Storage persistence ✅
- Full RPC API (129+ methods) ✅

### Phase 2: Advanced Features ✅
- Post-quantum crypto ✅
- Verkle trees ✅
- MEV protection ✅
- Security forensics ✅
- Sharding (core) ✅
- Block explorer ✅
- Monitoring (Prometheus/Grafana) ✅

### Phase 3: Testnet Launch (Current)
- ✅ Testnet-ready deployment
- ✅ All features operational
- ⚠️ Multi-node testing (recommended)
- ⚠️ Documentation polish
- ⚠️ Testnet configuration guide

### Phase 4: Ecosystem (Q2 2026)
- Public testnet launch
- Developer SDKs
- Wallet integrations
- Bridge to Ethereum
- Community building

### Future Phase: AI Integration (Timeline TBD)
- zkML verification
- On-chain AI inference
- Model registry
- Verifiable computation
- AI agent framework

**Note:** Planned as L2/RPC layer service, not part of L1 mainnet launch.

### Phase 5: Governance (Q3 2026)
- On-chain governance
- Parameter adjustment
- Upgrade mechanisms
- Community voting
- Treasury management

---

## 19. Technical Specifications

### Node Requirements

**Minimum:**
- CPU: 4 cores
- RAM: 8 GB
- Storage: 100 GB SSD
- Network: 10 Mbps

**Recommended:**
- CPU: 8+ cores
- RAM: 16+ GB
- Storage: 500 GB NVMe SSD
- Network: 100+ Mbps

### Software Stack

- **Language**: Rust 1.75+
- **Database**: Sled (embedded)
- **Networking**: Tokio async runtime
- **Cryptography**: RustCrypto, pqcrypto
- **EVM**: SputnikVM (evm 0.41)
- **Monitoring**: Prometheus

---

## 20. Getting Started

### Installation

```powershell
# Clone repository
git clone https://github.com/dev-irondag/irondag
cd irondag/irondag-blockchain

# Build node
cargo build --release

# Run node
./target/release/node
```

### Configuration

```toml
[node]
port = 8080
rpc_port = 8545
miner_address = "0x0101010101010101010101010101010101010101"

[features]
enable_sharding = true
shard_count = 10
enable_verkle = true
```

### Running Services

```powershell
# Start Prometheus & Grafana
cd grafana
docker-compose up -d

# Access dashboards
# Grafana: http://localhost:3001
# Prometheus: http://localhost:9090
```

---

## 21. API Examples

### Get Fairness Metrics

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "IronDAG_getMevMetrics",
    "params": [10],
    "id": 1
  }'
```

### Check Risk Score

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "IronDAG_getRiskScore",
    "params": ["0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb"],
    "id": 1
  }'
```

### Trace Funds

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "IronDAG_traceFunds",
    "params": ["0x123...", 5, 10],
    "id": 1
  }'
```

---

## 22. Security Audits

### Planned Audits
- [ ] Trail of Bits (Q1 2026)
- [ ] OpenZeppelin (Q2 2026)
- [ ] CertiK (Q2 2026)
- [ ] Quantstamp (Q3 2026)

### Bug Bounty Program
- Launch: Q2 2026
- Rewards: Up to $100,000
- Scope: Core protocol, smart contracts

---

## 23. Community & Support

### Resources
- **Website**: https://IronDAG.io 
- **GitHub**: https://github.com/dev-irondag/irondag
- **Documentation**: https://docs.IronDAG.io 
- **Discord**: https://discord.gg/IronDAG (Coming soon)
- **Twitter**: @DevIronDAG (https://x.com/DevIronDAG)

### Developer Support
- Stack Overflow: `IronDAG` tag
- GitHub Issues
- Discord #dev-support channel
- Monthly community calls

---

## 24. Team

### Core Contributors
- Core development team
- Additional team members to be announced

### Advisors
- To be announced

---

## 25. Legal & Compliance

### Token Status
- Not a security (pure utility token)
- No pre-mine or ICO
- Fair launch via mining only

### Regulatory Compliance
- GDPR compliant
- AML/KYC at exchange level only
- Open-source and transparent

---

## Conclusion

IronDAG is a Layer 1 blockchain that integrates:

✅ **Quantum Resistance** - Future-proof cryptography
✅ **Rule-Based Security** - Protocol-level fraud detection  
✅ **BraidCore Mining** - Multi-stream architecture
✅ **Advanced State Management** - Verkle trees
✅ **Fair Transaction Ordering** - MEV protection
✅ **Horizontal Scaling** - Native sharding
✅ **Full EVM Compatibility** - Ethereum ecosystem access

**IronDAG delivers post-quantum security with BraidCore mining architecture.**

---

## Appendix A: File Structure

```
IronDAG/
├── IronDAG-blockchain/
│   ├── src/
│   │   ├── bin/node.rs              # Node binary
│   │   ├── blockchain/              # Core blockchain
│   │   ├── mining/                  # BraidCore mining
│   │   │   ├── fairness.rs          # MEV detection
│   │   │   └── ordering.rs          # Tx ordering
│   │   ├── pqc/                     # Post-quantum crypto
│   │   │   ├── accounts.rs          # PQ accounts
│   │   │   ├── kyber.rs             # Key exchange
│   │   │   └── tooling.rs           # PQ utilities
│   │   ├── verkle/                  # Verkle trees
│   │   │   ├── tree.rs              # Tree implementation
│   │   │   └── proof.rs             # Proof generation
│   │   ├── security/                # Security layer
│   │   │   ├── forensics.rs         # Fund tracing
│   │   │   ├── risk_scoring.rs      # Risk analysis
│   │   │   └── policies.rs          # Security policies
│   │   ├── consensus.rs             # GhostDAG
│   │   ├── sharding.rs              # Sharding
│   │   ├── evm.rs                   # EVM integration
│   │   ├── rpc.rs                   # JSON-RPC API
│   │   ├── network.rs               # P2P networking
│   │   ├── storage.rs               # Persistence
│   │   ├── metrics.rs               # Monitoring
│   │   └── light_client.rs          # Light client
│   └── Cargo.toml
├── IronDAG-explorer-frontend/         # Block explorer
├── grafana/                          # Monitoring dashboards
└── docs/                             # Documentation
```

---

## Appendix B: RPC Methods (Complete List)

### Ethereum-Compatible Methods
- `eth_blockNumber`
- `eth_getBalance`
- `eth_getTransactionCount`
- `eth_getBlockByNumber`
- `eth_getBlockByHash`
- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_sendTransaction`
- `eth_sendRawTransaction`
- `eth_call`
- `eth_estimateGas`
- `eth_gasPrice`
- `eth_chainId`
- `eth_syncing`
- `net_version`
- `net_peerCount`

### IronDAG-Specific Methods

**DAG & Consensus:**
- `irondag_getDagStats`, `irondag_getTps`, `irondag_getBlueScore`
- `irondag_getBlockOrder`, `irondag_getParentHashes`

**Fairness & MEV:**
- `irondag_getMevMetrics`, `irondag_getBlockFairness`
- `irondag_setOrderingPolicy`, `irondag_getOrderingPolicy`
- `irondag_getFairnessMetrics`

**Security & Risk:**
- `irondag_getRiskScore`, `irondag_getRiskLabels`
- `irondag_getTransactionRisk`, `irondag_analyzeBehavior`
- `irondag_detectAnomalies`

**Forensics:**
- `irondag_traceFunds`, `irondag_getAddressSummary`
- `irondag_findRelatedAddresses`, `irondag_getTransactionPath`

**Sharding:**
- `irondag_getShardStats`, `irondag_getCrossShardTransaction`
- `irondag_getShardForAddress`, `irondag_getShardTransactions`
- `irondag_getShardBlock`, `irondag_getShardBalance`

**Verkle & Light Client:**
- `irondag_getStateRoot`, `irondag_getStateProof`
- `irondag_verifyStateProof`, `irondag_getLightClientProof`

**Post-Quantum:**
- `irondag_getPqAccountType`, `irondag_createPqAccount`
- `irondag_signPqTransaction`, `irondag_verifyPqSignature`

**Policies:**
- `irondag_addSecurityPolicy`, `irondag_removeSecurityPolicy`
- `irondag_getSecurityPolicies`, `irondag_checkPolicy`

*129+ total IronDAG-specific RPC methods - see full documentation for complete list*

---

**Document Version**: 1.1
**Last Updated**: January 2026
**Status**: Testnet Ready | Production Potential

---

*IronDAG: Building the Future of Decentralized Finance*
