# Network Security Model

IronDAG uses a layered authentication model for P2P communication. No single mechanism provides security — layers compose to create defense in depth.

## 1. Transport Encryption: QUIC + TLS 1.3

- All peer connections use QUIC (RFC 9000) with TLS 1.3
- Each node generates a self-signed ECDSA P-256 certificate at startup
- The P-256 key exists solely for the TLS handshake — it's what QUIC requires for transport encryption
- The node's persistent cryptographic identity is a separate Ed25519 key pair, embedded in the certificate's Subject Alternative Name (SAN) as a URI: `irondag:{hex_encoded_ed25519_pubkey}`
- **Design rationale**: Two key types serve two distinct purposes. P-256 handles the wire (TLS transport). Ed25519 handles trust (node identity). This separation is intentional — TLS mandates specific key types for handshake, while Ed25519 provides the compact, fast signatures needed for per-message authentication at scale.
- Certificate validity: 1 year from node startup
- No Certificate Authority — self-signed certificates are accepted for encryption, not trust. Trust is established at the application layer.
- Custom `InsecureCertVerifier` validates only certificate validity period (not_before/not_after), not CA chain. This is standard for P2P networks with ephemeral self-signed certificates.
- Reference: `src/quic_transport.rs` lines 63-202

## 2. Peer Identity: TOFU (Trust On First Use)

- After QUIC handshake, the node extracts the peer's Ed25519 public key from the certificate SAN via `extract_peer_identity()`
- On first connection, the key is accepted and pinned in a `peer_identities` map (HashMap<SocketAddr, Vec<u8>>)
- On subsequent connections, the key MUST match the pinned value or the connection is flagged as a potential MITM attack with both old and new key fingerprints logged
- **Honest trade-off**: The first connection to any peer is unprotected against man-in-the-middle by design. This is inherent to every TOFU system — SSH has the same limitation. First-connection trust relies on out-of-band verification or operator acceptance. Subsequent connections are cryptographically bound to the original key. This is an explicit architectural decision, not an oversight.
- Reference: `src/quic_transport.rs` lines 318-373, `src/network.rs` lines 1614-1625

## 3. Message Authentication: Ed25519 Signatures

- Every gossip protocol message is wrapped in an `AuthenticatedMessage` containing:
  - The serialized message payload
  - Sender's 32-byte Ed25519 public key
  - 64-byte Ed25519 signature over the payload
  - Timestamp (Unix epoch)
- Verification steps on receipt:
  1. **Key pinning check**: If the peer has a TOFU-pinned key, the message's signing key must match exactly. Mismatch → rejected as potential MITM.
  2. **Signature verification**: Ed25519 signature validated against the message payload.
  3. **Replay protection**: Timestamp must be within ±5 minutes of current time. Messages outside this window are rejected.
- Unsigned, malformed, or timestamp-expired messages are dropped and the peer's invalid message counter is incremented.
- Reference: `src/network.rs` lines 801-810, 1316-1398

## 4. Anti-Sybil and Eclipse Attack Prevention

Multiple independent mechanisms prevent network-level attacks:

### 4.1 Subnet Diversity

- Maximum peers per /16 subnet: configurable constant `MAX_PEERS_PER_SUBNET_16` (currently 10 for testnet, tighten for mainnet)
- Prevents a single operator from filling all peer slots from one network range
- Localhost (127.0.0.0/8) is exempt for local multi-node testing
- Reference: `src/network.rs` lines 41-43, 1545-1558

### 4.2 Outbound Slot Reservation

- 70% of peer slots are reserved for outbound connections (operator-controlled)
- 30% available for inbound connections
- Prevents an attacker from eclipsing a node by flooding inbound connections — the majority of peers are always chosen by the node operator
- Reference: `src/network.rs` constant `OUTBOUND_SLOT_RATIO = 0.7`

### 4.3 Peer Scoring and Ban System

Four independent penalty dimensions, each with its own counter and threshold:

| Dimension | Triggers | Threshold | What It Catches |
|-----------|----------|-----------|-----------------|
| Invalid messages | Ed25519 signature failure, deserialization errors, malformed frames | 5 events | Protocol abuse, corrupted peers |
| Invalid blocks | Consensus validation failure (bad PoW, invalid parent refs, rule violations) | 3 events | Dishonest miners, chain poisoning |
| Invalid transactions | Format errors, bad signatures, invalid nonces, double-spend attempts | 50 events | Spam, fuzzing, wallet bugs |
| Rate limiting | Message volume exceeding 3,000 messages/minute (~50/sec) per peer | Per-window check | DoS, flooding |

**Ban duration uses exponential backoff:**
- 1st offense: 10 minutes
- 2nd offense: 1 hour
- 3rd offense: 6 hours
- 4th+ offenses: 24 hours (cap)

Bans are temporary. On expiry, penalty counters reset but offense count persists — repeat offenders escalate faster. Ban cleanup runs periodically and logs expiration events.

Reference: `src/network.rs` lines 301-335 (PeerScore struct), 370-410 (thresholds and ban duration), 694-779 (penalty logic)

### 4.4 Reputation-Based Peer Selection

Beyond punitive banning, the network actively selects for high-value peers using a composite reputation score. This turns Sybil resistance from a defensive mechanism into an optimization — the network doesn't just exclude bad peers, it preferentially connects to useful ones.

A composite 0-100 score used for peer eviction and selection priority. Components:
- Block validity ratio (0-50 points)
- Delivery volume with diminishing returns (0-20 points)
- Latency tiers: ≤50ms excellent, ≤100ms good, ≤200ms acceptable, ≤500ms poor (0-20 points)
- Connection uptime (0-10 points)
- Novelty ratio: bonus for peers providing first-look blocks, penalty for duplicates (-10 to +10 points)
- Freshness penalty: deduction for peers sending stale blocks significantly behind the tip (-15 to 0 points)

Reference: `src/network.rs` lines 421-506 (reputation calculation)

### 4.5 Global Peer Cap with Reputation-Based Eviction

- Maximum total peers: `MAX_TOTAL_PEERS = 500` (testnet default, configurable for mainnet)
- When the peer count reaches this limit, the node evicts the lowest-reputation peer before accepting a new connection
- Eviction uses the existing composite reputation score (block validity, delivery, latency, uptime, novelty, freshness)
- Works in conjunction with subnet-level limits (`MAX_PEERS_PER_SUBNET_16 = 10`) for defense-in-depth
- This two-layer approach prevents both geographic concentration (subnet limits) and memory exhaustion (global cap)
- Reference: `src/network.rs` lines 46 (constant), peer acceptance logic with `evict_lowest_peer()`

## 5. Post-Quantum Key Exchange (Optional)

- When compiled with the `kyber` Cargo feature, an ML-KEM-768 (NIST FIPS 203) key exchange runs on a dedicated QUIC stream after the TLS handshake
- Protocol: Initiator sends 1184-byte public key → Responder sends 1184-byte public key → Initiator sends 1088-byte ciphertext (encapsulated shared secret) → Responder acknowledges decapsulation
- The 32-byte ML-KEM-768 shared secret is passed through HKDF-SHA256 with the domain separation label `IRONDAG-KYBER-AES256GCM-v1` to derive the AES-256-GCM key. Per NIST SP 800-227 draft guidance, hybrid post-quantum schemes should apply a KDF with domain separation even when the shared secret is already uniform (IND-CCA2). The domain label is versioned to ensure protocol upgrades produce distinct key material, preventing cross-protocol key reuse. This adds defense-in-depth at negligible computational cost.
- Each encrypted message uses a fresh random 96-bit nonce via `Aes256Gcm::generate_nonce()`
- Frame byte `0x02` (MSG_FRAME_KYBER_ENCRYPTED) identifies post-quantum encrypted messages
- Falls back to TLS-only encryption if Kyber session is unavailable for a peer
- Session keys are stored with automatic zeroization on drop (`zeroize::Zeroizing`)
- Platform support: `ml-kem` crate (pure Rust, FIPS 203) on Linux; `pqcrypto-kyber` (liboqs binding) on Windows/MSVC. Pure Rust is preferred for auditability and cross-compilation; the liboqs binding is used on MSVC targets where `ml-kem`'s inline assembly lacks compiler support.
- **Purpose**: Adds quantum-resistant forward secrecy on top of classical TLS 1.3. Even if TLS session keys are compromised by a future quantum computer, the Kyber layer protects message confidentiality independently.
- Reference: `src/pqc/kyber.rs`, `src/pqc/encryption.rs`, `src/network.rs` lines 69-174, 2239-2286, 3099-3129

## 6. Security Model Summary

| Layer | Mechanism | What It Provides |
|-------|-----------|------------------|
| Transport | QUIC + TLS 1.3 (ECDSA P-256) | Wire encryption, connection integrity |
| Identity | Ed25519 in certificate SAN + TOFU | Peer authentication, impersonation prevention |
| Messages | Ed25519 signatures + timestamps | Per-message authentication, replay protection |
| Network | Subnet limits + global cap + outbound reservation + scoring | Eclipse/Sybil attack prevention, reputation-based peer selection (0-100 composite: block validity, delivery, latency, uptime, novelty, freshness) |
| Post-Quantum | ML-KEM-768 + AES-256-GCM (optional) | Quantum-resistant forward secrecy |

**Design philosophy**: Open P2P network — any node can connect without an allowlist. Trust is cryptographic, not administrative. A peer cannot fake messages, impersonate a previously-seen node, or eclipse a target without controlling diverse network ranges. The post-quantum layer provides insurance against future cryptographic threats.

**Known limitations**:
- TOFU first-connection vulnerability (mitigated by operator verification and multi-layer validation)
- No permanent bans (intentional — prevents permanent network fragmentation from transient issues)
- Subnet diversity assumes honest IP allocation (does not protect against BGP hijacking)
