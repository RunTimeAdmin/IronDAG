# IronDAG Blockchain — Threat Model and Security Analysis

> This document is a genuine security analysis, not marketing material. It catalogs our attack surface, documents known risks including unresolved issues, explains our mitigations, and is honest about what has not been externally verified.
>
> Last updated: April 2026 | Author: David Cooper (CCIE #11567)

---

## 1. Scope and Methodology

### Assessment Type
This threat model is based on **self-assessed code review**. No external security audit has been performed. All findings represent the author's analysis of the current codebase as of April 2026.

### Subsystems Analyzed
1. P2P Networking (QUIC transport, Kyber handshake, gossip protocol)
2. Consensus (GhostDAG implementation, finality mechanism)
3. EVM Execution (SputnikVM integration, state management)
4. Mining (BraidCore multi-stream PoW, difficulty adjustment)
5. Storage (Sled database, state persistence)
6. RPC/API (JSON-RPC surface, authentication)
7. Cryptography (ML-KEM-768, Ed25519, HKDF, AES-GCM)
8. Transaction Pool (mempool management, spam prevention)

### Severity Ratings
- **Critical**: Could cause catastrophic network failure, consensus split, or fund loss
- **High**: Could cause significant disruption or targeted attacks on specific users
- **Medium**: Could cause localized issues or requires specific preconditions
- **Low**: Minor issues, defense-in-depth gaps, or documentation concerns

---

## 2. Attack Surface by Subsystem

### 2.1 P2P Networking

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| Eclipse attacks | Attacker controls all peer connections to isolate a victim node | **IMPLEMENTED**: Subnet diversity (max 10 peers from /16) + global peer cap (MAX_TOTAL_PEERS = 500 with reputation-based eviction) + 70% outbound slot reservation | Medium | Testnet limits are loose; mainnet requires stricter subnet granularity |
| Sybil attacks | Attacker creates many identities to overwhelm network | **IMPLEMENTED**: PoW cost for peer identity, reputation scoring (0-100 composite), exponential backoff bans | Medium | No stake-based Sybil resistance; pure PoW identity cost |
| TOFU first-connection | First contact with a peer is unauthenticated by design (SSH trust-on-first-use model) | **KNOWN RISK — UNCHANGED**: Operator-configured bootnodes reduce exposure window | Medium | Initial connection to unknown peers carries inherent risk |
| Gossip amplification | Malicious peer floods network with invalid messages | **IMPLEMENTED**: 3000 msgs/min per-peer rate limit, Ed25519 signature requirement on all gossip | Low | Rate limits are empirically tuned, not formally verified |
| Clock drift exploitation | Attacker manipulates timestamps to gain mining advantage or cause reordering | **IMPLEMENTED**: Graduated 5/10/15 point penalties at 2/3/4 min thresholds, 60s evaluation window, -20 point cap (commit 72eb70c) | Medium | NTP dependency remains; extreme drift (>5 min) still causes rejection |
| Kyber handshake exhaustion | Attacker opens many connections to exhaust Kyber computation resources | **IMPLEMENTED**: 50-permit semaphore with try_acquire, 10s timeout, plaintext fallback (commit 8916d7a) | Medium | Plaintext fallback window exposes traffic to non-PQC encryption |
| Kyber session key caching | Repeated handshakes with same peer cause computational overhead | **IMPLEMENTED**: 60s TTL, 1000 entry cap, LRU eviction, 90s cleanup interval (commit 8916d7a) | Low | Cache poisoning would require breaking AES-GCM or Ed25519 |
| Plaintext fallback window | During Kyber handshake or semaphore exhaustion, messages use TLS-only | **KNOWN RISK — ACCEPTED**: Window typically <1 second per connection | Low | Traffic is still TLS-encrypted, just not post-quantum |

### 2.2 Consensus (GhostDAG)

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| K=4 parameter choice | Anti-ghosting parameter K affects security/throughput tradeoff | **EMPIRICAL — NOT FORMALLY VERIFIED**: Kaspa standard value, chosen for 3-stream topology | High | Formal proof needed for our specific multi-stream configuration |
| 100-block finality | Finality depth empirically chosen | **EMPIRICAL — NOT FORMALLY VERIFIED**: Based on Kaspa mainnet observation | Medium | Mathematical proof of finality probability not completed |
| Selfish mining | Miner withholds blocks to gain unfair advantage | **STANDARD POW RISK**: Multi-stream architecture may introduce cross-stream vectors | Medium | Game theory analysis for 3-stream selfish mining not performed |
| Timestamp manipulation | Attacker manipulates block timestamps to affect difficulty | **MITIGATED**: ±5 minute replay window, clock drift reputation docking (commit 4e5b265) | Medium | NTP dependency for honest nodes; gaming within 5-min window possible |

### 2.3 EVM Execution

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| StateSnapshot merge bug | "Take maximum" merge logic creates phantom values when parallel batches modify same account | **KNOWN, UNMITIGATED**: Currently NOT in production path (sequential execution only) | Critical | Must fix before enabling parallel execution; documented in code comments |
| Reentrancy | Contract calls back into itself to manipulate state | **DELEGATED**: SputnikVM handles reentrancy protection (industry-standard) | Low | Relies on upstream Rust EVM project correctness |
| Integer overflow | Arithmetic overflow in EVM operations | **MITIGATED**: saturating_add/mul audit completed (commit 6210767) | High | All consensus-critical arithmetic now uses saturating operations |
| State divergence | Nodes disagree on state root after execution | **MITIGATED**: Dual-persist to sled + in-memory, SEC-010 fix (commit 7080860) | Critical | Fixed; monitoring in place to detect any future divergence |
| PUSH0/Shanghai compatibility | Incorrect Shanghai hard fork support | **VERIFIED WORKING**: Config::shanghai() enabled, integration tested | Low | Opcode behavior verified against Ethereum mainnet expectations |
| Block hash manipulation | Block hash opcode returns incorrect values | **MITIGATED**: 256-entry ring buffer, SEC-013 fix | Medium | Historical block hashes now correctly persisted and retrieved |

### 2.4 Mining (BraidCore)

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| Difficulty manipulation | Attacker manipulates timestamps to affect DAA | **MITIGATED**: DAA with 0.25x-4.0x clamping per epoch | Medium | Timestamp gaming within ±5 min window still possible |
| Cross-stream incentives | Game theory for 3-stream mining rewards | **NOT ANALYZED**: No formal documentation of cross-stream game theory | Medium | Potential for strategic mining across streams not modeled |
| Reward overflow | Block reward calculation overflows | **MITIGATED**: Saturating arithmetic throughout reward calculation | Low | Overflow would saturate, not wrap |

### 2.5 Storage (Sled)

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| Database encryption | Data at rest is encrypted | **NOT IMPLEMENTED**: Sled stores data unencrypted at rest | Medium | Physical access to storage exposes all node data |
| Corruption recovery | Database corruption handling | **MANUAL ONLY**: rm -rf data + resync required | Medium | No automated repair tool; full resync is recovery path |
| Storage DoS | Attacker fills disk with large blocks | **MITIGATED**: 10MB block size limit, per-stream tx pool caps | Low | Limits prevent unbounded growth |

### 2.6 RPC/API

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| Authentication bypass | RPC authentication can be disabled | **DOCUMENTED RISK**: --rpc-no-auth flag exists (documented as unsafe) | Medium | Operators can disable auth; not recommended for production |
| Information leakage | Debug endpoints expose internal state | **PARTIAL**: Debug endpoints present, rate limited | Low | No endpoint access control beyond rate limiting |
| Request size DoS | Large RPC requests cause memory exhaustion | **UNMITIGATED**: No explicit per-request size limit enforced | Medium | DoS vector; needs size limits on request body |
| Rate limiting | Request flooding | **MITIGATED**: 1000 req/min per IP, burst 50 | Low | Limits empirically tuned |

### 2.7 Cryptography

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| ML-KEM-768 implementation | Post-quantum key encapsulation | **UPSTREAM**: NIST FIPS 203 reference implementation (ml-kem crate) | Low | Relies on upstream library audit status |
| HKDF domain separation | Key derivation without domain labels | **IMPLEMENTED**: HKDF-SHA256 with domain label "IRONDAG-KYBER-AES256GCM-v1" (commit 8c44c9f) | Low | NIST SP 800-227 compliant derivation |
| Ed25519 signatures | Signature verification | **UPSTREAM**: Standard dalek implementation, strict verification | Low | No signature malleability (strict mode enabled) |
| Key zeroization | Sensitive key material in memory | **PARTIAL**: Session keys zeroed on drop (Zeroizing wrapper). Long-lived Ed25519 identity keys NOT zeroed during runtime | Medium | Runtime identity key exposure on memory dump |
| Platform split | Different Kyber implementations per platform | **ACCEPTED**: ml-kem on Linux, pqcrypto-kyber (liboqs) on Windows MSVC | Low | Both produce interoperable 32-byte shared secrets; tested |

### 2.8 Transaction Pool

| Attack Vector | Description | Mitigation Status | Severity | Residual Risk |
|--------------|-------------|-------------------|----------|---------------|
| Spam | Transaction pool flooding | **MITIGATED**: Per-stream caps (60K/30K/10K), fee-based priority | Low | Caps prevent unbounded growth |
| Front-running/MEV | Transaction ordering manipulation | **NOT MITIGATED**: FIFO within pool, NOT enforced at consensus | High | Miners can reorder transactions; no MEV protection |
| Nonce validation | Invalid nonce handling | **STRICT**: Exact nonce required, gaps rejected | Low | Stricter than Ethereum (which allows gaps) |

---

## 3. Known Unresolved Issues

| Issue | Severity | Subsystem | Status | Remediation Timeline |
|-------|----------|-----------|--------|---------------------|
| StateSnapshot merge bug — phantom values on parallel batch merge | Critical | EVM | Known, unmitigated | Before parallel execution enable |
| Database encryption at rest | Medium | Storage | Not implemented | Post-mainnet hardening |
| GhostDAG formal verification (K=4 for 3-stream topology) | High | Consensus | Not started | Research phase |
| BraidCore game theory analysis | Medium | Mining | Not started | Post-launch research |
| MEV protection / transaction ordering fairness | High | Tx Pool | Not implemented | Future hard fork |
| Long-lived Ed25519 key zeroization | Medium | Cryptography | Not implemented | Requires architectural change |
| RPC request size limits | Medium | RPC | Not implemented | Near-term fix |

---

## 4. External Verification Status

### Audit Status
**NO external security audit has been performed.** All findings in this document are self-assessed based on code review by the author.

### Cryptographic Primitive Reliance
We rely on the correctness of the following upstream libraries:
- **ml-kem**: NIST FIPS 203 reference implementation for ML-KEM-768
- **ed25519-dalek**: Ed25519 signature verification
- **aes-gcm**: AES-256-GCM encryption
- **hkdf**: HKDF-SHA256 key derivation

These libraries have varying levels of external audit coverage. We have not commissioned independent audits of our usage.

### EVM Correctness
SputnikVM EVM correctness is delegated to the upstream Rust EVM project. We have verified:
- PUSH0 opcode behavior via integration test
- Shanghai hard fork configuration
- State transition equivalence with Ethereum mainnet for tested cases

### Testnet Operational Status
Testnet has been operational since April 2026 across 3 VPS instances (6 nodes total), producing blocks with real PoW and processing EVM transactions.

---

## 5. Responsible Disclosure Policy

We welcome security researchers to examine our codebase and report vulnerabilities.

**Contact**: security@protocol11567.com
**PGP Key**: [To be published]

### Disclosure Timeline
- **Day 0**: Researcher reports vulnerability via email
- **Day 1-3**: We acknowledge receipt and begin assessment
- **Day 14**: We provide initial severity assessment to researcher
- **Day 90**: Public disclosure (coordinated with researcher)
- **Exception**: If actively exploited in the wild, disclosure may be accelerated

### Scope
All code in the irondag-blockchain repository is in scope:
- P2P networking and transport security
- Consensus and block validation
- EVM execution and state management
- Cryptographic implementations
- RPC/API endpoints

### Out of Scope
- Social engineering attacks
- Denial of service via volume (we're aware of rate limiting gaps)
- Issues in upstream dependencies (report to upstream maintainers)

### Recognition
Valid critical and high-severity findings will be rewarded from a designated portion of fundraise proceeds. We commit to:
- Public credit (with researcher consent) in our security acknowledgments
- Financial reward proportional to severity (details published after seed round)
- No legal action against good-faith security researchers

We believe honest disclosure builds trust. If you find something, tell us.

---

## 6. Risk Summary Matrix

| Finding | Severity | Subsystem | Status |
|---------|----------|-----------|--------|
| StateSnapshot merge bug | Critical | EVM | Unmitigated |
| GhostDAG formal verification gap | High | Consensus | Not started |
| MEV protection absent | High | Tx Pool | Not implemented |
| Database encryption absent | Medium | Storage | Not implemented |
| Long-lived key zeroization | Medium | Cryptography | Not implemented |
| RPC request size limits absent | Medium | RPC | Not implemented |
| BraidCore game theory | Medium | Mining | Not analyzed |
| TOFU first-connection risk | Medium | P2P | Accepted risk |
| Plaintext fallback window | Low | P2P | Accepted risk |
| Clock drift exploitation | Low | P2P | Mitigated |
| Kyber handshake exhaustion | Low | P2P | Mitigated |
| Kyber session key caching | Low | P2P | Mitigated |
| Eclipse attacks | Low | P2P | Mitigated |
| Sybil attacks | Low | P2P | Mitigated |
| Gossip amplification | Low | P2P | Mitigated |
| Integer overflow | Low | EVM | Mitigated |
| State divergence | Low | EVM | Mitigated |
| Block hash manipulation | Low | EVM | Mitigated |
| PUSH0/Shanghai | Low | EVM | Verified |
| Difficulty manipulation | Low | Mining | Mitigated |
| Reward overflow | Low | Mining | Mitigated |
| Storage DoS | Low | Storage | Mitigated |
| Authentication bypass | Low | RPC | Documented risk |
| Information leakage | Low | RPC | Partial |
| Rate limiting | Low | RPC | Mitigated |
| Spam | Low | Tx Pool | Mitigated |
| Nonce validation | Low | Tx Pool | Mitigated |

---

## Document History

| Date | Change |
|------|--------|
| April 2026 | Initial publication |

---

*This document is a living analysis. As the codebase evolves and issues are resolved, this threat model will be updated to reflect the current security posture.*

