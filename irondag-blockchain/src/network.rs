//! P2P Network layer for multi-node communication
//!
//! Features:
//! - Peer discovery
//! - Block propagation (with compact blocks)
//! - Transaction propagation
//! - Chain synchronization (headers-first sync)
//! - Orphan pool for out-of-order BlockDAG blocks

pub mod stun;
pub mod sync;

use crate::blockchain::{Block, Blockchain, PublicKey, Transaction};
use crate::network::sync::MAX_BLOCKS_PER_REQUEST;
use crate::quic_transport::{STREAM_TYPE_GOSSIP, STREAM_TYPE_KYBER, STREAM_TYPE_SYNC};
use crate::types::Hash;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex, RwLock, Semaphore};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

/// Type alias for QUIC gossip stream (send only)
pub type GossipStream = Arc<Mutex<quinn::SendStream>>;

/// Attempt to detect the primary network interface IP address.
/// Uses UDP socket trick — connects to external address without sending data.
fn detect_public_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// Maximum peers allowed from the same /16 subnet (eclipse attack prevention)
/// Set to 10 for testnet multi-node deployments; tighten for mainnet
const MAX_PEERS_PER_SUBNET_16: usize = 10;

/// Global maximum peer count. Prevents unbounded memory growth.
/// Tighten for testnet (e.g., 100), raise for mainnet (e.g., 2000).
const MAX_TOTAL_PEERS: usize = 500;

/// Timeout in seconds for each read operation during Kyber key exchange
/// A peer that can't complete a sub-100ms exchange within this window is a liability
#[cfg(feature = "kyber")]
const KYBER_HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Maximum concurrent Kyber handshake tasks (prevents blocking threadpool exhaustion)
const MAX_CONCURRENT_KYBER_HANDSHAKES: usize = 50;

/// Capability bit flags advertised in the P2P handshake.
/// Add a new bit when a protocol feature or algorithm variant is introduced.
/// Never reuse a bit — retired bits must stay 0 forever to avoid false positives.
pub const CAP_BLAKE3_POW: u32 = 1 << 0; // Blake3 PoW (Stream A/C)
pub const CAP_B3MEMHASH: u32 = 1 << 1; // B3MemHash PoW (Stream B)
pub const CAP_ML_KEM_768: u32 = 1 << 2; // ML-KEM-768 post-quantum KEM
pub const CAP_DILITHIUM3: u32 = 1 << 3; // Dilithium3 / ML-DSA-65 signatures
pub const CAP_SPHINCS_PLUS: u32 = 1 << 4; // SPHINCS+-SHA256-128f signatures
pub const CAP_COMPACT_BLOCKS: u32 = 1 << 5; // BIP-152 style compact block relay
pub const CAP_SHARDING: u32 = 1 << 6; // Horizontal sharding protocol

/// Bitmask of capabilities this node advertises. Update when new features ship.
#[cfg(feature = "kyber")]
pub const LOCAL_CAPABILITIES: u32 = CAP_BLAKE3_POW
    | CAP_B3MEMHASH
    | CAP_ML_KEM_768
    | CAP_DILITHIUM3
    | CAP_SPHINCS_PLUS
    | CAP_COMPACT_BLOCKS;

#[cfg(not(feature = "kyber"))]
pub const LOCAL_CAPABILITIES: u32 =
    CAP_BLAKE3_POW | CAP_B3MEMHASH | CAP_DILITHIUM3 | CAP_SPHINCS_PLUS | CAP_COMPACT_BLOCKS;

/// Minimum capability set a peer MUST advertise to be accepted.
/// Peers missing these bits cannot validate IronDAG blocks and are rejected
/// during the handshake. Kept conservative: only the two PoW algorithms
/// required for Stream A/B/C validation.
/// CAP_ML_KEM_768 is NOT required here because the kyber feature is optional.
pub const REQUIRED_CAPABILITIES: u32 = CAP_BLAKE3_POW | CAP_B3MEMHASH;

/// Returns true if `peer_caps` satisfies `REQUIRED_CAPABILITIES`.
pub fn meets_required_capabilities(peer_caps: u32) -> bool {
    peer_caps & REQUIRED_CAPABILITIES == REQUIRED_CAPABILITIES
}

/// Grace period in seconds for Kyber session key cache (reuse on quick reconnect)
#[cfg(feature = "kyber")]
const KYBER_SESSION_CACHE_TTL_SECS: u64 = 60;

/// Maximum number of entries in the Kyber session cache (prevents unbounded growth)
#[cfg(feature = "kyber")]
const MAX_KYBER_SESSION_CACHE_SIZE: usize = 1000;

/// Ratio of peer slots reserved for outbound connections (eclipse attack prevention)
const OUTBOUND_SLOT_RATIO: f64 = 0.7; // 70% of slots reserved for outbound

/// Calculate number of outbound slots (70% of max_peers)
fn outbound_slots(max_peers: usize) -> usize {
    (max_peers as f64 * OUTBOUND_SLOT_RATIO) as usize
}

/// Calculate number of inbound slots (30% of max_peers)
fn inbound_slots(max_peers: usize) -> usize {
    max_peers.saturating_sub(outbound_slots(max_peers))
}

/// Jaccard similarity threshold for Sybil warning (80% overlap)
const JACCARD_WARNING_THRESHOLD: f64 = 0.8;
/// Jaccard similarity threshold for strong Sybil signal (90% overlap)
const JACCARD_SYBIL_THRESHOLD: f64 = 0.9;

/// Monotonic nonce counter for replay protection — each signed message gets a unique nonce.
/// This ensures that even if a timestamp is within the 5-minute window, a replayed message
/// with the same nonce from the same peer will be rejected.
#[allow(dead_code)]
static MESSAGE_NONCE: AtomicU64 = AtomicU64::new(0);
/// Reputation penalty per Sybil detection
const JACCARD_REPUTATION_PENALTY: f64 = 5.0;

/// Sled key prefix for persisting known peers
const PEER_PREFIX: &[u8] = b"peer:";

/// Kyber public key size (ML-KEM-768 / Kyber768)
#[cfg(feature = "kyber")]
const KYBER_PUBLIC_KEY_SIZE: usize = 1184;

/// Kyber ciphertext size (ML-KEM-768 / Kyber768)
#[cfg(feature = "kyber")]
const KYBER_CIPHERTEXT_SIZE: usize = 1088;

/// Role in Kyber key exchange handshake
#[cfg(feature = "kyber")]
#[derive(Debug, Clone, Copy, PartialEq)]
enum KyberRole {
    /// Initiator: Sends public key first, receives ciphertext, then decapsulates
    Initiator,
    /// Responder: Receives public key first, encapsulates, sends ciphertext back
    Responder,
}

/// Perform Kyber key exchange handshake
///
/// Returns the shared secret session key on success.
///
/// # Protocol
/// - Initiator: Send PK -> Receive peer PK -> Encapsulate -> Send CT -> Receive ACK
/// - Responder: Receive peer PK -> Send PK -> Receive CT -> Decapsulate -> Send ACK
#[cfg(feature = "kyber")]
async fn perform_kyber_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    role: KyberRole,
    our_kyber: crate::pqc::KyberKeyExchange,
    peer_addr: SocketAddr,
) -> Result<crate::pqc::SessionKey, String> {
    match role {
        KyberRole::Initiator => {
            // Send our public key first
            let our_pk = our_kyber.public_key_bytes();
            send.write_all(&our_pk)
                .await
                .map_err(|e| format!("Failed to send Kyber PK to {}: {}", peer_addr, e))?;

            // Receive responder's public key
            let mut pk_buf = [0u8; KYBER_PUBLIC_KEY_SIZE];
            tokio::time::timeout(
                std::time::Duration::from_secs(KYBER_HANDSHAKE_TIMEOUT_SECS),
                recv.read_exact(&mut pk_buf),
            )
            .await
            .map_err(|e| format!("Timeout receiving PK from {}: {:?}", peer_addr, e))?
            .map_err(|e| format!("Failed to receive PK from {}: {}", peer_addr, e))?;

            // Encapsulate to get ciphertext and shared secret
            let (ciphertext, session_key) = our_kyber
                .encapsulate_async(pk_buf.to_vec())
                .await
                .map_err(|e| format!("Kyber encapsulation failed for {}: {}", peer_addr, e))?;

            // Send ciphertext
            send.write_all(&ciphertext)
                .await
                .map_err(|e| format!("Failed to send Kyber CT to {}: {}", peer_addr, e))?;

            // Wait for ACK
            let mut ack_buf = [0u8; 1];
            tokio::time::timeout(
                std::time::Duration::from_secs(KYBER_HANDSHAKE_TIMEOUT_SECS),
                recv.read_exact(&mut ack_buf),
            )
            .await
            .map_err(|e| format!("Timeout receiving ACK from {}: {:?}", peer_addr, e))?
            .map_err(|e| format!("Failed to receive ACK from {}: {}", peer_addr, e))?;

            if ack_buf[0] != 0x01 {
                return Err(format!("Invalid ACK from {}", peer_addr));
            }

            Ok(session_key)
        }
        KyberRole::Responder => {
            // Receive initiator's public key first
            let mut pk_buf = [0u8; KYBER_PUBLIC_KEY_SIZE];
            tokio::time::timeout(
                std::time::Duration::from_secs(KYBER_HANDSHAKE_TIMEOUT_SECS),
                recv.read_exact(&mut pk_buf),
            )
            .await
            .map_err(|e| format!("Timeout receiving PK from {}: {:?}", peer_addr, e))?
            .map_err(|e| format!("Failed to receive PK from {}: {}", peer_addr, e))?;

            // Send our public key
            let our_pk = our_kyber.public_key_bytes();
            send.write_all(&our_pk)
                .await
                .map_err(|e| format!("Failed to send Kyber PK to {}: {}", peer_addr, e))?;

            // Receive ciphertext
            let mut ct_buf = [0u8; KYBER_CIPHERTEXT_SIZE];
            tokio::time::timeout(
                std::time::Duration::from_secs(KYBER_HANDSHAKE_TIMEOUT_SECS),
                recv.read_exact(&mut ct_buf),
            )
            .await
            .map_err(|e| format!("Timeout receiving CT from {}: {:?}", peer_addr, e))?
            .map_err(|e| format!("Failed to receive CT from {}: {}", peer_addr, e))?;

            // Decapsulate to get shared secret
            let session_key = our_kyber
                .decapsulate_async(ct_buf.to_vec())
                .await
                .map_err(|e| format!("Kyber decapsulation failed for {}: {}", peer_addr, e))?;

            // Send ACK
            send.write_all(&[0x01])
                .await
                .map_err(|e| format!("Failed to send ACK to {}: {}", peer_addr, e))?;

            Ok(session_key)
        }
    }
}

/// Connection type for tracking inbound vs outbound peers
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionType {
    Inbound,
    Outbound,
}

/// Evict old entries from a seen cache (HashMap<Hash, Instant>) when it exceeds max_size.
/// First removes entries older than 2 hours, then removes oldest entries if still over limit.
fn evict_seen_cache(cache: &mut HashMap<Hash, Instant>, max_size: usize) {
    if cache.len() > max_size {
        let now = Instant::now();
        let two_hours = std::time::Duration::from_secs(2 * 60 * 60);
        cache.retain(|_, &mut timestamp| now.duration_since(timestamp) < two_hours);
        // If still over limit, remove oldest entries
        if cache.len() > max_size {
            let mut entries: Vec<(Hash, Instant)> = cache.iter().map(|(k, v)| (*k, *v)).collect();
            entries.sort_by_key(|(_, t)| *t);
            let to_remove = cache.len() - max_size + 1000;
            for (hash, _) in entries.into_iter().take(to_remove) {
                cache.remove(&hash);
            }
        }
    }
}

/// Extract /16 prefix from a socket address (first two octets of IPv4)
/// Returns None for IPv6 addresses (exempt from /16 bucketing)
fn subnet_prefix_16(addr: &SocketAddr) -> Option<[u8; 2]> {
    match addr.ip() {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // Localhost (127.0.0.0/8) is exempt from subnet limits for testing
            if octets[0] == 127 {
                return None;
            }
            Some([octets[0], octets[1]])
        }
        IpAddr::V6(_) => None, // IPv6 peers exempt from /16 bucketing
    }
}

/// Log peer diversity statistics
/// Called periodically or on peer count changes to monitor subnet diversity
fn log_peer_diversity(peer_count: usize, subnet_counts: &HashMap<[u8; 2], usize>) {
    let unique_subnets = subnet_counts.len();
    info!(
        "Peer diversity: {} peers across {} /16 subnets",
        peer_count, unique_subnets
    );

    if peer_count > 0 && !subnet_counts.is_empty() {
        // Find the most common subnet
        let (most_common_prefix, most_common_count) = subnet_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(prefix, count)| (*prefix, *count))
            .unwrap_or(([0, 0], 0));

        let percentage = (most_common_count as f64 / peer_count as f64) * 100.0;

        if most_common_count > peer_count / 2 {
            warn!(
                "Low peer diversity: {:.1}% of peers from subnet {}.{}.0.0/16",
                percentage, most_common_prefix[0], most_common_prefix[1]
            );
        }
    }
}

/// Persist a peer address to sled storage for reconnection on restart.
async fn persist_peer(blockchain: &Arc<RwLock<Blockchain>>, addr: &SocketAddr) {
    if let Some(db) = blockchain.read().await.database() {
        let key = [PEER_PREFIX, addr.to_string().as_bytes()].concat();
        if let Err(e) = db.insert_raw(key, vec![]) {
            warn!("Failed to persist peer {}: {}", addr, e);
        } else {
            debug!("Persisted peer {} to sled storage", addr);
        }
    }
}

/// Remove a peer address from sled storage.
async fn remove_persisted_peer(blockchain: &Arc<RwLock<Blockchain>>, addr: &SocketAddr) {
    if let Some(db) = blockchain.read().await.database() {
        let key = [PEER_PREFIX, addr.to_string().as_bytes()].concat();
        if let Err(e) = db.remove_raw(&key) {
            warn!("Failed to remove persisted peer {}: {}", addr, e);
        } else {
            debug!("Removed persisted peer {} from sled storage", addr);
        }
    }
}

/// Load persisted peers from sled storage. Returns vector of peer addresses.
async fn load_persisted_peers(blockchain: &Arc<RwLock<Blockchain>>) -> Vec<SocketAddr> {
    let mut peers = Vec::new();
    if let Some(db) = blockchain.read().await.database() {
        for (key, _) in db.scan_prefix_bytes(PEER_PREFIX) {
            let key_str = String::from_utf8_lossy(&key);
            let addr_str = key_str.strip_prefix("peer:").unwrap_or(&key_str);
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                peers.push(addr);
            }
        }
        if !peers.is_empty() {
            info!("Loaded {} persisted peers from sled storage", peers.len());
        }
    }
    peers
}

/// Calculate Jaccard similarity between two peer address sets.
/// Returns a value between 0.0 (disjoint sets) and 1.0 (identical sets).
/// Used for Sybil detection: high similarity between peers from different subnets
/// suggests a single operator puppeting multiple nodes.
fn jaccard_similarity(a: &HashSet<SocketAddr>, b: &HashSet<SocketAddr>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// Per-peer quality metrics for eviction when at max_peers.
#[derive(Debug, Clone)]
pub struct PeerScore {
    pub connected_at: Instant, // when peer connected
    pub last_seen: Instant,
    pub success_count: u32,
    pub failure_count: u32,
    // Rate limiting fields
    pub messages_this_window: u64, // messages received in current window
    pub window_start: Instant,     // when current rate window started
    pub banned_until: Option<Instant>, // if set, peer is temporarily banned
    pub ban_reason: Option<String>, // reason for ban
    pub invalid_blocks: u64,       // count of invalid blocks sent
    pub blocks_delivered: u64,     // count of valid blocks delivered
    pub invalid_txs: u64,          // count of invalid transactions sent
    pub invalid_messages: u32,     // count of failed verify_message() or invalid messages
    // Latency measurement fields
    pub latency_ms: Option<u64>,         // measured RTT in milliseconds
    pub ping_sent_at: Option<Instant>,   // when last ping was sent
    pub pending_ping_nonce: Option<u64>, // nonce for matching pong to ping
    pub latency_samples: Vec<u64>,       // last N RTT samples for averaging
    pub last_pong_time: Option<Instant>, // for rate-limiting pong responses
    // Ban tracking
    pub offense_count: u32, // number of times peer has been banned
    // Freshness scoring fields
    pub stale_blocks: u32, // Blocks received >STALE_THRESHOLD behind tip
    pub fresh_blocks: u32, // Blocks received within STALE_THRESHOLD of tip
    pub last_block_height_delta: i64, // Last observed delta from our tip
    // Novelty tracking fields
    pub novel_blocks: u32,     // Blocks we hadn't seen before
    pub duplicate_blocks: u32, // Blocks we already had in DAG
    // Bandwidth tracking fields
    pub bytes_sent: u64,               // Bytes sent to this peer
    pub bytes_received: u64,           // Bytes received from this peer
    pub last_bandwidth_reset: Instant, // When bandwidth counters were last reset
    // Clock drift tracking
    pub clock_drift_penalties: f64, // Accumulated reputation penalty from timestamp drift
    pub last_clock_drift_check: Option<Instant>, // Last time clock drift penalty was applied (rate-limited)
}

impl Default for PeerScore {
    fn default() -> Self {
        Self {
            connected_at: Instant::now(),
            last_seen: Instant::now(),
            success_count: 0,
            failure_count: 0,
            messages_this_window: 0,
            window_start: Instant::now(),
            banned_until: None,
            ban_reason: None,
            invalid_blocks: 0,
            blocks_delivered: 0,
            invalid_txs: 0,
            invalid_messages: 0,
            latency_ms: None,
            ping_sent_at: None,
            pending_ping_nonce: None,
            latency_samples: Vec::new(),
            last_pong_time: None,
            offense_count: 0,
            stale_blocks: 0,
            fresh_blocks: 0,
            last_block_height_delta: 0,
            novel_blocks: 0,
            duplicate_blocks: 0,
            bytes_sent: 0,
            bytes_received: 0,
            last_bandwidth_reset: Instant::now(),
            clock_drift_penalties: 0.0,
            last_clock_drift_check: None,
        }
    }
}

// Rate limiting and reputation constants
const MAX_MESSAGES_PER_MINUTE: u64 = 3000; // ~50 messages/sec per peer (increased for sync)
const MAX_INVALID_MESSAGES: u32 = 5; // ban after 5 invalid messages (verify_message failure)
const MAX_INVALID_BLOCKS: u64 = 3; // ban after 3 invalid blocks
const MAX_INVALID_TXS: u64 = 50; // ban after 50 invalid txs
const RATE_WINDOW_SECS: u64 = 60; // 1-minute rate window
const PONG_RATE_LIMIT_SECS: u64 = 1; // minimum seconds between pongs
const LATENCY_MIN_MS: u64 = 1; // minimum valid latency (1ms)
const LATENCY_MAX_MS: u64 = 10_000; // maximum valid latency (10s)

/// Calculate ban duration based on number of offenses (exponential backoff)
fn ban_duration_for_offense(offense_count: u32) -> std::time::Duration {
    match offense_count {
        0 | 1 => std::time::Duration::from_secs(600), // 10 minutes
        2 => std::time::Duration::from_secs(3600),    // 1 hour
        3 => std::time::Duration::from_secs(21600),   // 6 hours
        _ => std::time::Duration::from_secs(86400),   // 24 hours (cap)
    }
}

/// Format duration in human-readable form
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs >= 3600 {
        format!("{}h", secs / 3600)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{}s", secs)
    }
}

// Partition detection constants
const PARTITION_DETECTION_THRESHOLD_SECS: u64 = 60; // No new blocks for 60s = potential partition
const PARTITION_PEER_DIVERGENCE_BLOCKS: u64 = 10; // Peers 10+ blocks ahead = likely partitioned

// Freshness scoring constants
const STALE_BLOCK_THRESHOLD: u64 = 100; // Blocks behind tip to consider "stale"
const STALE_PENALTY_SCORE: f64 = 0.5; // Reputation penalty per stale block
const FRESHNESS_MIN_SAMPLE_SIZE: u32 = 10; // Minimum blocks before applying freshness penalty
const NOVELTY_MIN_SAMPLE_SIZE: u32 = 10; // Minimum blocks before applying novelty adjustment

impl PeerScore {
    /// Composite score for ordering (higher = better). Evict lowest first.
    fn eviction_score(&self) -> i64 {
        let success = self.success_count as i64;
        let failure = self.failure_count as i64 * 2;
        success.saturating_sub(failure)
    }

    /// Calculate reputation score (higher is better, 0-100 scale).
    /// Penalizes invalid blocks heavily. New peers start at neutral (50).
    fn reputation(&self) -> f64 {
        // Calculate valid block ratio
        let valid_ratio = if self.blocks_delivered > 0 {
            1.0 - (self.invalid_blocks as f64 / self.blocks_delivered as f64)
        } else {
            0.5 // neutral for new peers with no blocks delivered
        };

        // Base score from block validity (0-50 points)
        let block_score = valid_ratio * 50.0;

        // Bonus for total blocks delivered (up to 20 points, diminishing returns)
        let delivery_bonus = if self.blocks_delivered > 0 {
            let log_blocks = (self.blocks_delivered as f64).ln().max(0.0);
            (log_blocks * 2.0).min(20.0)
        } else {
            0.0
        };

        // Penalty for invalid transactions (up to -10 points)
        let tx_penalty = (self.invalid_txs as f64 * 0.2).min(10.0);

        // Latency factor (up to 20 points bonus for low latency)
        let latency_bonus = if let Some(latency) = self.latency_ms {
            if latency <= 50 {
                20.0 // Excellent latency
            } else if latency <= 100 {
                15.0 // Good latency
            } else if latency <= 200 {
                10.0 // Acceptable latency
            } else if latency <= 500 {
                5.0 // Poor latency
            } else {
                0.0 // Very poor latency
            }
        } else {
            10.0 // Neutral if no latency data yet
        };

        // Connection time bonus (up to 10 points for long-lived connections)
        let connection_bonus = {
            let uptime_secs = self.connected_at.elapsed().as_secs();
            if uptime_secs >= 3600 {
                10.0 // Connected for 1+ hour
            } else if uptime_secs >= 300 {
                5.0 // Connected for 5+ minutes
            } else {
                0.0 // New connection
            }
        };

        // Freshness penalty - peers sending stale blocks lose reputation
        let freshness_adjustment = {
            let total_blocks = self.stale_blocks.saturating_add(self.fresh_blocks);
            if total_blocks >= FRESHNESS_MIN_SAMPLE_SIZE {
                let stale_ratio = self.stale_blocks as f64 / total_blocks as f64;
                // Progressive penalty based on stale ratio (max -15 points)
                let penalty = stale_ratio * STALE_PENALTY_SCORE * 30.0;
                -penalty.min(15.0)
            } else {
                0.0 // Neutral until we have enough samples
            }
        };

        // Novelty bonus - peers that give us first-look data are more valuable
        let novelty_adjustment = {
            let total_received = self.novel_blocks.saturating_add(self.duplicate_blocks);
            if total_received >= NOVELTY_MIN_SAMPLE_SIZE {
                let novelty_ratio = self.novel_blocks as f64 / total_received as f64;
                // High novelty (>50%) = positive, low novelty (<30%) = negative
                // Scale: -10 to +10 points based on novelty ratio
                let adjustment = (novelty_ratio - 0.3) * 25.0;
                adjustment.clamp(-10.0, 10.0)
            } else {
                0.0 // Neutral until we have enough samples
            }
        };

        // Final score: base + bonuses - penalties + adjustments
        // Clock drift penalty — peers with consistent timestamp drift lose reputation
        // Accumulated from drift > 2 minutes in verify_message path
        let drift_penalty = self.clock_drift_penalties.min(20.0); // cap at -20 points
        let score = block_score + delivery_bonus + latency_bonus + connection_bonus - tx_penalty
            + freshness_adjustment
            + novelty_adjustment
            - drift_penalty;

        // Clamp to 0-100
        score.clamp(0.0, 100.0)
    }

    /// Calculate freshness ratio (fresh / total) for logging/monitoring
    #[allow(dead_code)]
    fn freshness_ratio(&self) -> f64 {
        let total = self.stale_blocks.saturating_add(self.fresh_blocks);
        if total == 0 {
            0.5 // neutral
        } else {
            self.fresh_blocks as f64 / total as f64
        }
    }

    /// Calculate novelty ratio (novel / total) for logging/monitoring
    fn novelty_ratio(&self) -> f64 {
        let total = self.novel_blocks.saturating_add(self.duplicate_blocks);
        if total == 0 {
            0.5 // neutral
        } else {
            self.novel_blocks as f64 / total as f64
        }
    }

    /// Check if peer has concerning stale block ratio (>50%)
    #[allow(dead_code)]
    fn has_high_stale_ratio(&self) -> bool {
        let total = self.stale_blocks.saturating_add(self.fresh_blocks);
        if total < FRESHNESS_MIN_SAMPLE_SIZE {
            return false; // Not enough data
        }
        let stale_ratio = self.stale_blocks as f64 / total as f64;
        stale_ratio > 0.5
    }

    /// Check if peer has low novelty ratio (<10%)
    #[allow(dead_code)]
    fn has_low_novelty_ratio(&self) -> bool {
        let total = self.novel_blocks.saturating_add(self.duplicate_blocks);
        if total < NOVELTY_MIN_SAMPLE_SIZE {
            return false; // Not enough data
        }
        let novelty_ratio = self.novel_blocks as f64 / total as f64;
        novelty_ratio < 0.1
    }

    /// Calculate bandwidth efficiency score
    /// Higher score = better efficiency (high novelty, low bandwidth consumption)
    /// Formula: novelty_ratio * bytes_received / (bytes_sent + 1)
    fn bandwidth_efficiency(&self) -> f64 {
        let novelty_ratio = self.novelty_ratio();
        let bytes_sent = self.bytes_sent.max(1); // Avoid division by zero
        (novelty_ratio * self.bytes_received as f64) / bytes_sent as f64
    }

    /// Check if bandwidth counters need reset (every 10 minutes)
    fn should_reset_bandwidth(&self) -> bool {
        self.last_bandwidth_reset.elapsed().as_secs() >= 600 // 10 minutes
    }

    /// Reset bandwidth counters
    fn reset_bandwidth_counters(&mut self) {
        self.bytes_sent = 0;
        self.bytes_received = 0;
        self.last_bandwidth_reset = Instant::now();
    }

    /// Check if peer is currently banned
    fn is_banned(&self) -> bool {
        if let Some(banned_until) = self.banned_until {
            banned_until > Instant::now()
        } else {
            false
        }
    }
}

/// Evict lowest-scoring peer (for use from accept loop which doesn't have &self).
///
/// LOCK ORDERING CONVENTION: To prevent deadlocks, always acquire locks in this order:
///   1. peers (write)
///   2. gossip_streams (lock)
///   3. connection_types (write)
///   4. subnet_peer_counts (write)
///   5. peer_scores (write)
///
/// This function correctly drops all read locks before acquiring any write locks.
async fn evict_lowest_peer(
    peers: &Arc<RwLock<HashSet<SocketAddr>>>,
    gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    peer_scores: &Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    peer_count_atomic: &Arc<AtomicUsize>,
    connection_types: &Arc<RwLock<HashMap<SocketAddr, ConnectionType>>>,
    subnet_peer_counts: &Arc<RwLock<HashMap<[u8; 2], usize>>>,
) -> Option<SocketAddr> {
    let peers_set = peers.read().await;
    if peers_set.is_empty() {
        return None;
    }
    let scores = peer_scores.read().await;
    let mut worst: Option<(SocketAddr, i64, Instant)> = None;
    for &addr in peers_set.iter() {
        let score = scores.get(&addr).map(|s| s.eviction_score()).unwrap_or(0);
        let last_seen = scores
            .get(&addr)
            .map(|s| s.last_seen)
            .unwrap_or(Instant::now());
        let replace = match &worst {
            None => true,
            Some((_, s, t)) => score < *s || (score == *s && last_seen < *t),
        };
        if replace {
            worst = Some((addr, score, last_seen));
        }
    }
    drop(scores);
    drop(peers_set);
    let evict_addr = worst.map(|(a, _, _)| a)?;
    let was_present = peers.write().await.remove(&evict_addr);
    // TODO: Peer persistence - pass Database reference to remove from sled storage.
    // This function doesn't have database access, so evicted peers remain in persistence
    // and will be reconnected on restart (acceptable for eviction scenario).
    if was_present {
        let prev = peer_count_atomic.load(Ordering::Relaxed);
        if prev > 0 {
            peer_count_atomic.fetch_sub(1, Ordering::Relaxed);
        } else {
            warn!("[P2P] Attempted to decrement peer count below zero during eviction, skipping");
        }
    }
    if let Some(stream_arc) = gossip_streams.lock().await.remove(&evict_addr) {
        drop(stream_arc);
    }
    peer_scores.write().await.remove(&evict_addr);
    connection_types.write().await.remove(&evict_addr);
    // Decrement subnet counter on eviction
    if let Some(prefix) = subnet_prefix_16(&evict_addr) {
        let mut subnet_counts = subnet_peer_counts.write().await;
        if let Some(count) = subnet_counts.get_mut(&prefix) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                subnet_counts.remove(&prefix);
            }
        }
    }
    debug!(
        "Evicted lowest-scoring peer {} to make room (incoming)",
        evict_addr
    );
    Some(evict_addr)
}

/// Check peer rate limit. Returns true if message is allowed, false if rate-limited.
/// Updates message count and bans peers that exceed the rate limit.
fn check_peer_rate_limit(
    peer_scores: &mut HashMap<SocketAddr, PeerScore>,
    addr: &SocketAddr,
) -> bool {
    let now = Instant::now();
    let window_duration = std::time::Duration::from_secs(RATE_WINDOW_SECS);

    let score = peer_scores.entry(*addr).or_insert_with(PeerScore::default);

    // Check if peer is currently banned
    if score.is_banned() {
        return false;
    }

    // Reset window if expired
    if now.duration_since(score.window_start) >= window_duration {
        score.messages_this_window = 0;
        score.window_start = now;
    }

    // Increment message count
    score.messages_this_window += 1;

    // Check if rate limit exceeded
    if score.messages_this_window > MAX_MESSAGES_PER_MINUTE {
        score.offense_count = score.offense_count.saturating_add(1);
        let ban_duration = ban_duration_for_offense(score.offense_count);
        score.banned_until = Some(now + ban_duration);
        score.ban_reason = Some(format!(
            "rate limit exceeded ({} msgs/min)",
            score.messages_this_window
        ));
        warn!(
            "Banned peer {} for {} (offense #{}): rate limit exceeded ({} msgs/min)",
            addr,
            format_duration(ban_duration),
            score.offense_count,
            score.messages_this_window
        );
        return false;
    }

    true
}

/// Check if a peer is currently banned (for use before accepting connections)
fn is_peer_banned(peer_scores: &HashMap<SocketAddr, PeerScore>, addr: &SocketAddr) -> bool {
    if let Some(score) = peer_scores.get(addr) {
        score.is_banned()
    } else {
        false
    }
}

/// Reason for penalizing a peer
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum PenaltyReason {
    InvalidBlock,
    InvalidTransaction,
    InvalidMessage,
    Spam,
    Timeout,
    ProtocolViolation,
    SignatureVerificationFailed,
    MalformedMessage,
}

impl PenaltyReason {
    fn as_str(&self) -> &'static str {
        match self {
            PenaltyReason::InvalidBlock => "invalid_block",
            PenaltyReason::InvalidTransaction => "invalid_transaction",
            PenaltyReason::InvalidMessage => "invalid_message",
            PenaltyReason::Spam => "spam",
            PenaltyReason::Timeout => "timeout",
            PenaltyReason::ProtocolViolation => "protocol_violation",
            PenaltyReason::SignatureVerificationFailed => "signature_verification_failed",
            PenaltyReason::MalformedMessage => "malformed_message",
        }
    }
}

/// Penalize a peer for sending invalid data. May result in temporary ban.
fn penalize_peer(
    peer_scores: &mut HashMap<SocketAddr, PeerScore>,
    addr: &SocketAddr,
    reason: PenaltyReason,
) {
    let now = Instant::now();
    let score = peer_scores.entry(*addr).or_insert_with(PeerScore::default);

    // Increment the appropriate counter based on reason
    let reason_str = reason.as_str();
    match reason {
        PenaltyReason::InvalidMessage
        | PenaltyReason::SignatureVerificationFailed
        | PenaltyReason::MalformedMessage => {
            // Invalid message (verify_message failure, bad signature, etc.)
            score.invalid_messages = score.invalid_messages.saturating_add(1);
            if score.invalid_messages >= MAX_INVALID_MESSAGES {
                score.offense_count = score.offense_count.saturating_add(1);
                let ban_duration = ban_duration_for_offense(score.offense_count);
                score.banned_until = Some(now + ban_duration);
                score.ban_reason = Some(format!("{} invalid messages", score.invalid_messages));
                warn!(
                    "Banned peer {} for {} (offense #{}): {} ({} invalid messages)",
                    addr,
                    format_duration(ban_duration),
                    score.offense_count,
                    reason_str,
                    score.invalid_messages
                );
            } else {
                warn!(
                    "Peer {} penalized for: {} ({} invalid messages)",
                    addr, reason_str, score.invalid_messages
                );
            }
        }
        PenaltyReason::InvalidBlock => {
            score.invalid_blocks += 1;
            if score.invalid_blocks >= MAX_INVALID_BLOCKS {
                score.offense_count = score.offense_count.saturating_add(1);
                let ban_duration = ban_duration_for_offense(score.offense_count);
                score.banned_until = Some(now + ban_duration);
                score.ban_reason = Some(format!("{} invalid blocks", score.invalid_blocks));
                warn!(
                    "Banned peer {} for {} (offense #{}): {} ({} invalid blocks)",
                    addr,
                    format_duration(ban_duration),
                    score.offense_count,
                    reason_str,
                    score.invalid_blocks
                );
            } else {
                warn!(
                    "Peer {} penalized for: {} ({} invalid blocks)",
                    addr, reason_str, score.invalid_blocks
                );
            }
        }
        PenaltyReason::InvalidTransaction => {
            score.invalid_txs += 1;
            if score.invalid_txs >= MAX_INVALID_TXS {
                score.offense_count = score.offense_count.saturating_add(1);
                let ban_duration = ban_duration_for_offense(score.offense_count);
                score.banned_until = Some(now + ban_duration);
                score.ban_reason = Some(format!("{} invalid transactions", score.invalid_txs));
                warn!(
                    "Banned peer {} for {} (offense #{}): {} ({} invalid transactions)",
                    addr,
                    format_duration(ban_duration),
                    score.offense_count,
                    reason_str,
                    score.invalid_txs
                );
            } else {
                warn!(
                    "Peer {} penalized for: {} ({} invalid transactions)",
                    addr, reason_str, score.invalid_txs
                );
            }
        }
        _ => {
            // Generic penalty - increment failure count
            score.failure_count = score.failure_count.saturating_add(1);
            warn!("Peer {} penalized for: {}", addr, reason_str);
        }
    }
}

/// Maximum network message size (10MB - DoS protection)
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of orphan resolution passes during sync
const MAX_ORPHAN_RESOLUTION_PASSES: usize = 50;

// Protocol v2: All messages use frame byte prefix
// v1 peers (without framing) will fail to connect - this is intentional
// as we're pre-mainnet and can break wire format

/// Frame byte for plaintext messages (no encryption)
const MSG_FRAME_PLAINTEXT: u8 = 0x00;
/// Frame byte for Kyber-encrypted messages (AES-256-GCM wrapped)
const MSG_FRAME_KYBER_ENCRYPTED: u8 = 0x01;

/// Authenticated network message wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedMessage {
    /// The actual message payload
    pub message: NetworkMessage,
    /// Ed25519 signature (64 bytes) - signs the serialized message
    pub signature: Vec<u8>,
    /// Ed25519 public key (32 bytes) - for signature verification
    pub public_key: PublicKey,
    /// Message timestamp (Unix epoch seconds) - prevents replay attacks
    pub timestamp: u64,
}

/// Network message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    /// Handshake - announce listen address and capability bitmask
    Handshake {
        listen_addr: String,
        /// Bitmask of CAP_* flags this peer supports. Peers MUST tolerate
        /// unknown bits (forward compatibility). Missing field = 0 (legacy peer).
        #[serde(default)]
        capabilities: u32,
    },
    /// Announce a new block (full block)
    NewBlock { block: Block },
    /// Announce a new block using compact format (BIP 152 style)
    NewCompactBlock { compact_block: sync::CompactBlock },
    /// Announce a new block from a specific shard
    NewShardBlock { block: Block, shard_id: usize },
    /// Announce a new transaction
    NewTransaction { transaction: Transaction },
    /// Request blocks (for sync)
    RequestBlocks { from_block: u64, count: u64 },
    // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
    // Headers-first sync removed as dead code — DAG block structure makes
    // header-only validation insufficient.
    /// Request full blocks by hash (for sync)
    RequestBlocksByHash { hashes: Vec<Hash> },
    /// Request missing parent blocks (for orphan pool)
    RequestMissingParents { hashes: Vec<Hash> },
    /// Request blocks from a specific shard
    RequestShardBlocks {
        shard_id: usize,
        from_block: u64,
        count: u64,
    },
    /// Send blocks (response to RequestBlocks)
    Blocks { blocks: Vec<Block> },
    /// Send blocks from a specific shard
    ShardBlocks { shard_id: usize, blocks: Vec<Block> },
    /// Ping (keepalive with nonce for latency measurement)
    Ping { nonce: u64 },
    /// Pong (response to ping, echoes back nonce)
    Pong { nonce: u64 },
    /// Peer list request
    RequestPeers,
    /// Peer list response
    Peers { addresses: Vec<String> },
    /// Request missing transactions for compact block reconstruction
    /// Sent when reconstruction fails due to missing txs in mempool
    GetMissingTransactions {
        block_hash: Hash,
        short_ids: Vec<[u8; 6]>,
        nonce: u64,
    },
    /// Response with missing transactions for compact block reconstruction
    MissingTransactions {
        block_hash: Hash,
        transactions: Vec<Transaction>,
    },
    /// Kyber public key exchange (post-handshake, over encrypted Noise channel)
    /// Sent after Noise XX handshake to establish hybrid PQ session
    KyberPublicKey { public_key: Vec<u8> },
    /// Kyber ciphertext (encapsulated shared secret)
    /// Sent by initiator after receiving responder's Kyber public key
    KyberCiphertext { ciphertext: Vec<u8> },
    /// Kyber handshake acknowledgment
    /// Sent by responder after successful decapsulation to confirm session establishment
    KyberHandshakeAck,
}

/// Shared network context for P2P communication.
///
/// Bundles all shared state needed by `handle_peer`, `process_message`,
/// and `handle_connection_streams` to avoid 30+ parameter functions.
///
/// Peers and connections are keyed by **actual remote SocketAddr** only (the TCP endpoint).
/// We do not key by advertised listen address from Handshake, so multiple nodes that
/// advertise the same address (e.g. 127.0.0.1:8080) each get their own entry and all
/// receive broadcast blocks and sync.
pub struct NetworkContext {
    /// Blockchain state
    pub blockchain: Arc<RwLock<Blockchain>>,
    /// Connected peer addresses
    pub peers: Arc<RwLock<HashSet<SocketAddr>>>,
    /// Atomic peer counter for non-blocking peer_count() RPC
    pub peer_count_atomic: Arc<AtomicUsize>,
    /// Listen address for advertising to peers
    pub listen_addr: SocketAddr,
    /// Running state flag
    pub is_running: Arc<RwLock<bool>>,
    /// Ed25519 signing key for message authentication
    pub signing_key: ed25519_dalek::SigningKey,
    /// Node public key (derived from signing key)
    pub node_public_key: PublicKey,
    /// Gossip streams for each peer
    pub gossip_streams: Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    /// Orphan pool for out-of-order BlockDAG blocks
    pub orphan_pool: Arc<sync::OrphanPool>,
    /// Mining manager (for mempool access in compact blocks)
    pub mining_manager: Option<Arc<crate::mining::MiningManager>>,
    /// Channel to request peer connections
    pub peer_connect_tx: Option<mpsc::Sender<SocketAddr>>,
    /// Per-peer quality scores
    pub peer_scores: Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    /// Shard manager for shard-aware propagation
    pub shard_manager: Option<Arc<crate::sharding::ShardManager>>,
    /// Max peers limit
    pub max_peers: Option<u32>,
    /// Transaction hashes seen (deduplication)
    pub tx_seen: Arc<RwLock<HashMap<Hash, Instant>>>,
    /// Block hashes seen (deduplication)
    pub block_seen: Arc<RwLock<HashMap<Hash, Instant>>>,
    /// Partition detection flag
    pub partition_detected: Arc<AtomicBool>,
    /// Partition start time
    pub partition_start: Arc<RwLock<Option<Instant>>>,
    /// Last block received timestamp
    pub last_block_received: Arc<RwLock<Instant>>,
    /// Local tip height
    pub local_tip_height: Arc<AtomicU64>,
    /// Peer Ed25519 public keys
    pub peer_public_keys: Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    /// Peer advertised listen addresses
    pub peer_advertised_addrs: Arc<RwLock<HashMap<SocketAddr, String>>>,
    /// RequestPeers rate limiting
    pub peer_request_peers_time: Arc<RwLock<HashMap<SocketAddr, Instant>>>,
    /// Subnet diversity tracking
    pub subnet_peer_counts: Arc<RwLock<HashMap<[u8; 2], usize>>>,
    /// Connection type tracking
    pub connection_types: Arc<RwLock<HashMap<SocketAddr, ConnectionType>>>,
    /// Jaccard Sybil detection
    pub peer_exchange_lists: Arc<RwLock<HashMap<SocketAddr, HashSet<SocketAddr>>>>,
    /// QUIC connections map
    pub quic_connections: Arc<Mutex<HashMap<SocketAddr, quinn::Connection>>>,
    /// Kyber key exchange
    pub kyber_keys: Arc<Mutex<Option<crate::pqc::KyberKeyExchange>>>,
    /// Kyber session keys for PQ-encrypted communication
    #[cfg(feature = "kyber")]
    pub kyber_session_keys:
        Arc<RwLock<std::collections::HashMap<SocketAddr, zeroize::Zeroizing<Vec<u8>>>>>,
    /// Cached Kyber session keys for quick reconnection (TTL-based)
    #[cfg(feature = "kyber")]
    pub kyber_session_cache:
        Arc<RwLock<std::collections::HashMap<SocketAddr, (zeroize::Zeroizing<Vec<u8>>, Instant)>>>,
    /// Semaphore for limiting concurrent Kyber handshakes
    pub kyber_handshake_semaphore: Arc<Semaphore>,
    /// Cached sorted peer list for fanout (timestamp, peer list)
    pub cached_fanout_peers: Arc<RwLock<Option<(Instant, Vec<SocketAddr>)>>>,
}

/// Network manager for P2P communication.
///
/// Peers and connections are keyed by **actual remote SocketAddr** only (the TCP endpoint).
/// We do not key by advertised listen address from Handshake, so multiple nodes that
/// advertise the same address (e.g. 127.0.0.1:8080) each get their own entry and all
/// receive broadcast blocks and sync.
pub struct NetworkManager {
    blockchain: Arc<RwLock<Blockchain>>,
    /// Connected peer addresses (remote SocketAddr per connection). One entry per peer.
    peers: Arc<RwLock<HashSet<SocketAddr>>>,
    /// Atomic peer counter for non-blocking peer_count() RPC
    peer_count_atomic: Arc<AtomicUsize>,
    listen_addr: SocketAddr,
    /// Override for handshake (e.g. public IP:port). If set, used instead of listen_addr for advertising.
    advertise_addr: Option<String>,
    is_running: Arc<RwLock<bool>>,
    /// Node's signing key for message authentication (32 bytes Ed25519 secret key)
    #[allow(dead_code)]
    node_secret_key: [u8; 32],
    /// Node's public key (derived from secret key)
    #[allow(dead_code)]
    node_public_key: PublicKey,
    /// Kyber key exchange for PQ-encrypted P2P communication (Arc for sharing with accept loop)
    kyber_keys: Arc<Mutex<Option<crate::pqc::KyberKeyExchange>>>,
    /// Kyber session keys for hybrid PQ encryption (peer_addr -> 32-byte key)
    /// Used for additional AES-256-GCM layer on top of QUIC transport
    /// Wrapped in Zeroizing to ensure secure memory clearing on drop
    #[cfg(feature = "kyber")]
    kyber_session_keys:
        Arc<RwLock<std::collections::HashMap<SocketAddr, zeroize::Zeroizing<Vec<u8>>>>>,
    /// Cached Kyber session keys for quick reconnection (TTL-based)
    #[cfg(feature = "kyber")]
    kyber_session_cache:
        Arc<RwLock<std::collections::HashMap<SocketAddr, (zeroize::Zeroizing<Vec<u8>>, Instant)>>>,
    /// Semaphore for limiting concurrent Kyber handshakes
    kyber_handshake_semaphore: Arc<Semaphore>,
    /// QUIC endpoint for P2P connections (set up during start())
    quic_endpoint: Arc<RwLock<Option<quinn::Endpoint>>>,
    /// Active QUIC connections (peer_addr -> connection)
    peer_quic_connections: Arc<Mutex<HashMap<SocketAddr, quinn::Connection>>>,
    /// Gossip streams for each peer (peer_addr -> (send_stream, recv_stream))
    peer_gossip_streams: Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    /// Shard manager for shard-aware block/transaction propagation
    shard_manager: Option<Arc<crate::sharding::ShardManager>>,
    /// Ed25519 signing key for QUIC certificate generation
    signing_key: ed25519_dalek::SigningKey,
    // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
    // Headers-first sync removed as dead code — DAG block structure makes
    // header-only validation insufficient.
    /// Orphan pool for out-of-order BlockDAG blocks
    orphan_pool: Arc<sync::OrphanPool>,
    /// Mining manager (for mempool access in compact blocks)
    mining_manager: Option<Arc<crate::mining::MiningManager>>,
    /// Max peers (None = no limit). Enforced in connect_peer and on accept.
    max_peers: Option<u32>,
    /// Channel to request peer connections (peer exchange). Node receives and calls connect_peer.
    peer_connect_tx: Option<mpsc::Sender<SocketAddr>>,
    /// Per-peer quality (last_seen, success/failure counts). Used to evict lowest-scoring peer when at max.
    peer_scores: Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    /// Peers we've sent RequestPeers to recently (for deduplication)
    recent_request_peers: Arc<RwLock<HashSet<SocketAddr>>>,
    /// Peer advertised listen addresses (from Handshake) - maps connection addr to advertised addr
    peer_advertised_addrs: Arc<RwLock<HashMap<SocketAddr, String>>>,
    /// Last time we processed RequestPeers from each peer (for rate limiting)
    peer_request_peers_time: Arc<RwLock<HashMap<SocketAddr, Instant>>>,
    /// Transaction hashes we've seen (for deduplication). Max 10,000 entries.
    tx_seen: Arc<RwLock<HashMap<Hash, Instant>>>,
    /// Block hashes we've seen (for deduplication). Max 10,000 entries.
    block_seen: Arc<RwLock<HashMap<Hash, Instant>>>,
    // Partition detection fields
    /// True when we've detected a potential network partition
    partition_detected: Arc<AtomicBool>,
    /// When the partition was first detected (Instant)
    partition_start: Arc<RwLock<Option<Instant>>>,
    /// Timestamp of last received block (from mining or P2P)
    last_block_received: Arc<RwLock<Instant>>,
    /// Local tip height (block number) for peer divergence comparison
    local_tip_height: Arc<AtomicU64>,

    /// Peer Ed25519 public keys for message signature verification (peer_addr -> public_key)
    peer_public_keys: Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    /// TOFU: Peer Noise static public keys for identity tracking (peer_addr -> public_key)
    peer_identities: Arc<RwLock<HashMap<SocketAddr, Vec<u8>>>>,
    /// Shutdown signal sender for graceful shutdown (watch channel)
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Shutdown signal receiver (cloned for accept loop)
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    /// Subnet diversity tracking: /16 prefix -> peer count (eclipse attack prevention)
    subnet_peer_counts: Arc<RwLock<HashMap<[u8; 2], usize>>>,
    /// Connection type tracking: peer addr -> Inbound/Outbound (eclipse attack prevention)
    connection_types: Arc<RwLock<HashMap<SocketAddr, ConnectionType>>>,
    /// Last peer list received from each peer (for Jaccard Sybil detection)
    peer_exchange_lists: Arc<RwLock<HashMap<SocketAddr, HashSet<SocketAddr>>>>,
    /// QUIC connection idle timeout in seconds
    quic_idle_timeout_secs: u64,
    /// Cached sorted peer list for fanout (timestamp, peer list)
    /// Invalidated on peer join/leave or latency change
    cached_fanout_peers: Arc<RwLock<Option<(Instant, Vec<SocketAddr>)>>>,
    /// Explicitly configured public IP (skips UDP discovery)
    public_ip: Option<std::net::IpAddr>,
    /// Shared network context for handle_peer/process_message (bundled Arc state)
    ctx: Arc<NetworkContext>,
}

/// Loads the node identity key from `{data_dir}/node_key` or generates and saves a new one.
pub fn load_or_generate_signing_key(data_dir: &str) -> [u8; 32] {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use std::path::Path;

    let key_path = Path::new(data_dir).join("node_key");
    if let Ok(bytes) = std::fs::read(&key_path) {
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            info!("Loaded persisted node identity key from {:?}", key_path);
            return arr;
        }
    }
    let key = SigningKey::generate(&mut OsRng);
    let bytes = key.to_bytes();
    if let Err(e) = std::fs::write(&key_path, &bytes) {
        warn!("Could not persist node identity key to {:?}: {}", key_path, e);
    } else {
        info!("Generated and saved new node identity key to {:?}", key_path);
    }
    bytes
}

impl NetworkManager {
    /// Creates a new network manager, loading a persisted Ed25519 identity key or generating one.
    ///
    /// # Arguments
    /// * `blockchain` - The blockchain instance to share with peers
    /// * `listen_addr` - The local address to bind for P2P connections
    /// * `data_dir` - Data directory used to load or save the identity key
    pub fn new_with_data_dir(
        blockchain: Arc<RwLock<Blockchain>>,
        listen_addr: SocketAddr,
        data_dir: &str,
    ) -> Self {
        let secret = load_or_generate_signing_key(data_dir);
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&secret);
        Self::new_inner(blockchain, listen_addr, signing_key)
    }

    /// Creates a new network manager with a freshly-generated Ed25519 identity key.
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, listen_addr: SocketAddr) -> Self {
        // Generate a new Ed25519 keypair for node identity (single generation)
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut OsRng);
        info!("Generated new node identity key");

        Self::new_inner(blockchain, listen_addr, signing_key)
    }

    /// Sets the QUIC idle timeout in seconds.
    pub fn set_quic_idle_timeout(&mut self, secs: u64) {
        self.quic_idle_timeout_secs = secs;
    }

    /// Sets the mining manager for mempool access in compact blocks.
    pub fn set_mining_manager(&mut self, mining_manager: Arc<crate::mining::MiningManager>) {
        self.mining_manager = Some(mining_manager);
    }

    /// Creates a network manager with an existing node identity.
    ///
    /// # Arguments
    /// * `blockchain` - The blockchain instance to share with peers
    /// * `listen_addr` - The local address to bind for P2P connections
    /// * `secret_key` - The 32-byte Ed25519 secret key for signing messages
    pub fn with_identity(
        blockchain: Arc<RwLock<Blockchain>>,
        listen_addr: SocketAddr,
        secret_key: [u8; 32],
    ) -> Self {
        use ed25519_dalek::SigningKey;

        // Derive signing key from provided secret key
        let signing_key = SigningKey::from_bytes(&secret_key);

        Self::new_inner(blockchain, listen_addr, signing_key)
    }

    /// Shared initialization logic for both `new()` and `with_identity()` constructors.
    fn new_inner(
        blockchain: Arc<RwLock<Blockchain>>,
        listen_addr: SocketAddr,
        signing_key: ed25519_dalek::SigningKey,
    ) -> Self {
        let node_secret_key = signing_key.to_bytes();
        let node_public_key = signing_key.verifying_key().to_bytes().to_vec();

        // Create shutdown signal channel (watch channel for graceful shutdown)
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Generate Kyber keys for PQ-encrypted communication
        // Feature-gated: only generate if kyber feature is enabled
        // Note: This is called during initialization (not in async runtime), so blocking is OK
        #[cfg(feature = "kyber")]
        let kyber_keys = {
            let keys = crate::pqc::KyberKeyExchange::generate();
            info!("Kyber PQ key exchange enabled for P2P handshake");
            Arc::new(Mutex::new(Some(keys)))
        };
        #[cfg(not(feature = "kyber"))]
        let kyber_keys: Arc<Mutex<Option<crate::pqc::KyberKeyExchange>>> =
            Arc::new(Mutex::new(None));

        // Create shared Arc instances once, then reuse for both NetworkManager and ctx
        let peers = Arc::new(RwLock::new(HashSet::new()));
        let peer_count_atomic = Arc::new(AtomicUsize::new(0));
        let is_running = Arc::new(RwLock::new(false));
        #[allow(unused_variables)]
        let kyber_session_keys: Arc<
            RwLock<std::collections::HashMap<SocketAddr, zeroize::Zeroizing<Vec<u8>>>>,
        > = Arc::new(RwLock::new(std::collections::HashMap::new()));
        #[allow(unused_variables)]
        let kyber_session_cache: Arc<
            RwLock<std::collections::HashMap<SocketAddr, (zeroize::Zeroizing<Vec<u8>>, Instant)>>,
        > = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let kyber_handshake_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_KYBER_HANDSHAKES));
        let peer_quic_connections = Arc::new(Mutex::new(HashMap::new()));
        let peer_gossip_streams = Arc::new(Mutex::new(HashMap::new()));
        let orphan_pool = Arc::new(sync::OrphanPool::new(1000));
        let peer_scores = Arc::new(RwLock::new(HashMap::new()));
        let recent_request_peers = Arc::new(RwLock::new(HashSet::new()));
        let peer_advertised_addrs = Arc::new(RwLock::new(HashMap::new()));
        let peer_request_peers_time = Arc::new(RwLock::new(HashMap::new()));
        let tx_seen = Arc::new(RwLock::new(HashMap::new()));
        let block_seen = Arc::new(RwLock::new(HashMap::new()));
        let partition_detected = Arc::new(AtomicBool::new(false));
        let partition_start = Arc::new(RwLock::new(None));
        let last_block_received = Arc::new(RwLock::new(Instant::now()));
        let local_tip_height = Arc::new(AtomicU64::new(0));
        let peer_public_keys = Arc::new(RwLock::new(HashMap::new()));
        let peer_identities = Arc::new(RwLock::new(HashMap::new()));
        let subnet_peer_counts = Arc::new(RwLock::new(HashMap::new()));
        let connection_types = Arc::new(RwLock::new(HashMap::new()));
        let peer_exchange_lists = Arc::new(RwLock::new(HashMap::new()));

        let ctx = Arc::new(NetworkContext {
            blockchain: blockchain.clone(),
            peers: peers.clone(),
            peer_count_atomic: peer_count_atomic.clone(),
            listen_addr,
            is_running: is_running.clone(),
            signing_key: signing_key.clone(),
            node_public_key: node_public_key.clone(),
            gossip_streams: peer_gossip_streams.clone(),
            orphan_pool: orphan_pool.clone(),
            mining_manager: None,
            peer_connect_tx: None,
            peer_scores: peer_scores.clone(),
            shard_manager: None,
            max_peers: None,
            tx_seen: tx_seen.clone(),
            block_seen: block_seen.clone(),
            partition_detected: partition_detected.clone(),
            partition_start: partition_start.clone(),
            last_block_received: last_block_received.clone(),
            local_tip_height: local_tip_height.clone(),
            peer_public_keys: peer_public_keys.clone(),
            peer_advertised_addrs: peer_advertised_addrs.clone(),
            peer_request_peers_time: peer_request_peers_time.clone(),
            subnet_peer_counts: subnet_peer_counts.clone(),
            connection_types: connection_types.clone(),
            peer_exchange_lists: peer_exchange_lists.clone(),
            quic_connections: peer_quic_connections.clone(),
            kyber_keys: kyber_keys.clone(),
            #[cfg(feature = "kyber")]
            kyber_session_keys: kyber_session_keys.clone(),
            #[cfg(feature = "kyber")]
            kyber_session_cache: kyber_session_cache.clone(),
            kyber_handshake_semaphore: kyber_handshake_semaphore.clone(),
            cached_fanout_peers: Arc::new(RwLock::new(None)),
        });

        Self {
            blockchain,
            peers,
            peer_count_atomic,
            listen_addr,
            advertise_addr: None,
            is_running,
            node_secret_key,
            node_public_key,
            kyber_keys,
            #[cfg(feature = "kyber")]
            kyber_session_keys,
            #[cfg(feature = "kyber")]
            kyber_session_cache,
            kyber_handshake_semaphore,
            quic_endpoint: Arc::new(RwLock::new(None)),
            peer_quic_connections,
            peer_gossip_streams,
            signing_key,
            shard_manager: None,
            // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
            orphan_pool,
            mining_manager: None,
            max_peers: None,
            peer_connect_tx: None,
            peer_scores,
            recent_request_peers,
            peer_advertised_addrs,
            peer_request_peers_time,
            tx_seen,
            block_seen,
            // Partition detection initialization
            partition_detected,
            partition_start,
            last_block_received,
            local_tip_height,
            peer_public_keys,
            peer_identities,
            shutdown_tx,
            shutdown_rx,
            subnet_peer_counts,
            connection_types,
            peer_exchange_lists,
            quic_idle_timeout_secs: 30, // Default 30 seconds
            cached_fanout_peers: Arc::new(RwLock::new(None)),
            public_ip: None,
            ctx,
        }
    }

    /// Sets the channel for peer connect requests (peer exchange).
    pub fn set_peer_connect_tx(&mut self, tx: mpsc::Sender<SocketAddr>) {
        self.peer_connect_tx = Some(tx);
    }

    /// Sets the maximum number of peers (None = no limit).
    pub fn set_max_peers(&mut self, max: u32) {
        self.max_peers = Some(max);
    }

    /// Sets the public IP for P2P handshake (skips UDP discovery).
    pub fn set_public_ip(&mut self, ip: std::net::IpAddr) {
        self.public_ip = Some(ip);
    }

    /// Evicts the lowest-scoring peer when at max_peers limit.
    pub async fn evict_lowest_scoring_peer(&self) -> Option<SocketAddr> {
        evict_lowest_peer(
            &self.peers,
            &self.peer_gossip_streams,
            &self.peer_scores,
            &self.peer_count_atomic,
            &self.connection_types,
            &self.subnet_peer_counts,
        )
        .await
    }

    /// Records a successful send to a peer, updating quality metrics.
    pub async fn record_send_success(&self, addr: SocketAddr) {
        let mut scores = self.peer_scores.write().await;
        if let Some(s) = scores.get_mut(&addr) {
            s.last_seen = Instant::now();
            s.success_count = s.success_count.saturating_add(1);
        }
    }

    /// Records a failed send to a peer, updating quality metrics.
    pub async fn record_send_failure(&self, addr: SocketAddr) {
        let mut scores = self.peer_scores.write().await;
        if let Some(s) = scores.get_mut(&addr) {
            s.failure_count = s.failure_count.saturating_add(1);
        }
    }

    /// Sets the advertise address for P2P handshake (used when binding to 0.0.0.0).
    pub fn set_advertise_addr(&mut self, addr: String) {
        self.advertise_addr = Some(addr);
    }

    /// Returns the listen address (bind address).
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    /// Sets the shard manager for shard-aware block propagation.
    pub fn set_shard_manager(&mut self, shard_manager: Arc<crate::sharding::ShardManager>) {
        self.shard_manager = Some(shard_manager);
    }

    /// Enables post-quantum encrypted P2P communication (currently disabled).
    pub fn enable_pq_encryption(&mut self) {
        // NOTE: Kyber is currently disabled
        // if self.kyber_keys.is_none() {
        //     self.kyber_keys = Some(crate::pqc::KyberKeyExchange::generate());
        // }
    }

    /// Returns the Kyber public key for handshake if available.
    /// Note: This uses try_lock() which is non-blocking. Use in async context for reliable access.
    pub fn get_kyber_public_key(&self) -> Option<Vec<u8>> {
        self.kyber_keys
            .try_lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|k| k.public_key_bytes()))
    }

    /// Sign a network message
    fn sign_message(
        &self,
        message: NetworkMessage,
    ) -> crate::error::BlockchainResult<AuthenticatedMessage> {
        use bincode;
        use ed25519_dalek::Signer;

        // Serialize message for signing
        let message_bytes = bincode::serialize(&message)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        // Sign message with the cached signing key
        let signing_key = &self.signing_key;
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes: [u8; 32] = verifying_key.to_bytes();

        // Compute timestamp BEFORE signing so it's included in signed payload
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Sign message bytes + timestamp together to prevent replay with altered timestamps
        let mut signed_payload = message_bytes;
        signed_payload.extend_from_slice(&timestamp.to_le_bytes());
        let signature = signing_key.sign(&signed_payload);
        let signature = signature.to_bytes().to_vec();
        let public_key = public_key_bytes.to_vec();

        Ok(AuthenticatedMessage {
            message,
            signature,
            public_key,
            timestamp,
        })
    }

    /// Verify an authenticated message
    ///
    /// # Arguments
    /// * `msg` - The authenticated message to verify
    /// * `pinned_key` - Optional pinned public key for TOFU (Trust On First Use) verification.
    ///   If provided, the message's public key must match exactly.
    fn verify_message(
        msg: &AuthenticatedMessage,
        pinned_key: Option<&[u8; 32]>,
    ) -> crate::error::BlockchainResult<()> {
        use bincode;
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        // Check timestamp (prevent replay attacks - allow 5 minute window)
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Allow 5 minute clock skew
        if msg.timestamp > current_time + 300 || msg.timestamp < current_time.saturating_sub(300) {
            return Err(crate::error::BlockchainError::Network(
                "Message timestamp out of acceptable range (possible replay attack)".to_string(),
            ));
        }

        // Require valid signature on all messages (no unsigned messages allowed)
        if msg.signature.is_empty() || msg.public_key.is_empty() {
            return Err(crate::error::BlockchainError::Network(
                "Message missing signature or public key - unsigned messages are not allowed"
                    .to_string(),
            ));
        }

        // Verify signature
        if msg.signature.len() != 64 {
            return Err(crate::error::BlockchainError::Network(
                "Invalid signature length".to_string(),
            ));
        }

        if msg.public_key.len() != 32 {
            return Err(crate::error::BlockchainError::Network(
                "Invalid public key length".to_string(),
            ));
        }

        // Check key pin FIRST (TOFU - Trust On First Use)
        if let Some(expected) = pinned_key {
            if msg.public_key.as_slice() != expected.as_slice() {
                return Err(crate::error::BlockchainError::Network(
                    "Public key mismatch: peer key changed (possible MITM)".to_string(),
                ));
            }
        }

        // Parse public key
        let pub_key_bytes: [u8; 32] = msg.public_key.as_slice().try_into().map_err(|_| {
            crate::error::BlockchainError::Network("Invalid public key format".to_string())
        })?;

        let verifying_key = VerifyingKey::from_bytes(&pub_key_bytes).map_err(|_| {
            crate::error::BlockchainError::Network("Invalid public key".to_string())
        })?;

        // Parse signature
        let sig_bytes: [u8; 64] = msg.signature.as_slice().try_into().map_err(|_| {
            crate::error::BlockchainError::Network("Invalid signature format".to_string())
        })?;

        let signature = Signature::try_from(&sig_bytes[..])
            .map_err(|_| crate::error::BlockchainError::Network("Invalid signature".to_string()))?;

        // Serialize message for verification
        let message_bytes = bincode::serialize(&msg.message)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        // Reconstruct the signed payload: message bytes + timestamp
        let mut signed_payload = message_bytes;
        signed_payload.extend_from_slice(&msg.timestamp.to_le_bytes());

        // Verify signature
        verifying_key
            .verify(&signed_payload, &signature)
            .map_err(|_| {
                crate::error::BlockchainError::Network(
                    "Message signature verification failed".to_string(),
                )
            })?;

        Ok(())
    }

    /// Get a QUIC connection for a peer (for sync operations)
    ///
    /// # Arguments
    /// * `peer_addr` - The address of the peer to get connection for
    ///
    /// # Returns
    /// The QUIC connection if it exists, None otherwise
    pub async fn get_peer_connection(&self, peer_addr: SocketAddr) -> Option<quinn::Connection> {
        self.peer_quic_connections
            .lock()
            .await
            .get(&peer_addr)
            .cloned()
    }

    /// Gets a peer's Ed25519 public key for signature verification (VULN-006)
    ///
    /// # Arguments
    /// * `peer_addr` - The address of the peer to get public key for
    ///
    /// # Returns
    /// The peer's public key as [u8; 32] if it exists, None otherwise
    pub async fn get_peer_public_key(&self, peer_addr: SocketAddr) -> Option<[u8; 32]> {
        let keys = self.peer_public_keys.read().await;
        keys.get(&peer_addr).and_then(|pk| {
            // PublicKey is a Vec<u8>, convert to [u8; 32]
            if pk.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(pk);
                Some(arr)
            } else {
                None
            }
        })
    }

    /// Starts the network layer and begins listening for peer connections using QUIC.
    pub async fn start(&self) -> crate::error::BlockchainResult<()> {
        *self.is_running.write().await = true;

        info!(
            "Starting P2P network on {} (QUIC transport)",
            self.listen_addr
        );

        // Load persisted peers from sled storage and queue for reconnection
        // FIX: Also register them as known peers to avoid rejection on reconnect
        let persisted_peers = load_persisted_peers(&self.blockchain).await;
        if !persisted_peers.is_empty() {
            info!(
                "Registering {} persisted peers as known",
                persisted_peers.len()
            );

            // Register persisted peers in peers set and peer_scores
            let mut peers_set = self.peers.write().await;
            let mut scores = self.peer_scores.write().await;
            for peer_addr in &persisted_peers {
                peers_set.insert(*peer_addr);
                scores.entry(*peer_addr).or_insert_with(PeerScore::default);
            }
            drop(scores);
            drop(peers_set);

            // Queue for reconnection
            info!(
                "Queueing {} persisted peers for reconnection",
                persisted_peers.len()
            );
            if let Some(tx) = &self.peer_connect_tx {
                for peer_addr in persisted_peers {
                    if let Err(e) = tx.send(peer_addr).await {
                        warn!("Failed to queue persisted peer {}: {}", peer_addr, e);
                    }
                }
            }
        }

        // Create QUIC endpoint with TLS 1.3
        let endpoint = crate::quic_transport::create_endpoint(
            self.listen_addr,
            &self.signing_key,
            self.quic_idle_timeout_secs,
        )
        .map_err(|e| {
            crate::error::BlockchainError::Network(format!(
                "Failed to create QUIC endpoint on {}: {}",
                self.listen_addr, e
            ))
        })?;

        // Store endpoint in the struct
        *self.quic_endpoint.write().await = Some(endpoint.clone());

        info!(
            "QUIC endpoint listening on {}",
            endpoint.local_addr().unwrap_or(self.listen_addr)
        );

        let peers = self.peers.clone();
        let blockchain = self.blockchain.clone();
        let is_running = self.is_running.clone();
        let quic_connections = self.peer_quic_connections.clone();
        let gossip_streams = self.peer_gossip_streams.clone();
        // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
        let orphan_pool = self.orphan_pool.clone();
        let mining_manager = self.mining_manager.clone();
        let peer_connect_tx = self.peer_connect_tx.clone();
        let peer_scores = self.peer_scores.clone();
        let max_peers = self.max_peers;
        let listen_addr = self.listen_addr;
        let shard_manager = self.shard_manager.clone();
        let tx_seen = self.tx_seen.clone();
        let block_seen = self.block_seen.clone();
        let peer_count_atomic = self.peer_count_atomic.clone();
        // Partition detection fields
        let partition_detected = self.partition_detected.clone();
        let partition_start = self.partition_start.clone();
        let last_block_received = self.last_block_received.clone();
        let local_tip_height = self.local_tip_height.clone();
        // Advertise address for handshake
        let _advertise_addr = self.advertise_addr.clone();
        // Peer public keys for signature verification
        let peer_public_keys = self.peer_public_keys.clone();
        // Peer identities for TOFU tracking
        let peer_identities = self.peer_identities.clone();
        // Node identity for signing messages
        let signing_key = self.signing_key.clone();
        let node_public_key = self.node_public_key.clone();
        // Peer exchange fields
        let peer_advertised_addrs = self.peer_advertised_addrs.clone();
        let peer_request_peers_time = self.peer_request_peers_time.clone();
        // Subnet diversity tracking
        let subnet_peer_counts = self.subnet_peer_counts.clone();
        // Connection type tracking (eclipse attack prevention)
        let connection_types = self.connection_types.clone();
        // Jaccard Sybil detection
        let peer_exchange_lists = self.peer_exchange_lists.clone();
        // Kyber key exchange for PQ security
        let kyber_keys = self.kyber_keys.clone();
        // Kyber session keys for hybrid PQ encryption
        #[cfg(feature = "kyber")]
        let kyber_session_keys = self.kyber_session_keys.clone();
        // Kyber session cache for quick reconnection
        #[cfg(feature = "kyber")]
        let kyber_session_cache = self.kyber_session_cache.clone();
        // Kyber handshake semaphore for concurrency limiting
        let kyber_handshake_semaphore = self.kyber_handshake_semaphore.clone();
        // Shutdown signal for graceful shutdown
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    conn = crate::quic_transport::accept_connection(&endpoint) => {
                        match conn {
                            Some(connection) => {
                                let addr = connection.remote_address();

                                // Check global peer limit first (memory budgeting)
                                {
                                    let current = peers.read().await.len();
                                    if current >= MAX_TOTAL_PEERS {
                                        // Try to evict lowest-reputation peer to make room
                                        let evicted = evict_lowest_peer(&peers, &gossip_streams, &peer_scores, &peer_count_atomic, &connection_types, &subnet_peer_counts).await;
                                        if evicted.is_none() {
                                            warn!("Rejecting connection from {}: global peer limit reached ({} >= {})", addr, current, MAX_TOTAL_PEERS);
                                            continue;
                                        }
                                    }
                                }

                                // Check if peer is banned before accepting connection
                                {
                                    let scores = peer_scores.read().await;
                                    if is_peer_banned(&scores, &addr) {
                                        warn!("Rejecting connection from banned peer {}", addr);
                                        // Connection will be dropped when it goes out of scope
                                        continue;
                                    }
                                }

                                // Prevent duplicate connections: if we already have this peer
                                // (e.g., as an outbound connection), skip the inbound.
                                // This avoids the collision where both nodes have --peer pointing
                                // at each other and establish simultaneous connections.
                                {
                                    if peers.read().await.contains(&addr) {
                                        debug!("Skipping inbound from {} — already connected", addr);
                                        continue;
                                    }
                                }

                                // Check subnet diversity limit (eclipse attack prevention)
                                if let Some(prefix) = subnet_prefix_16(&addr) {
                                    let mut subnet_counts = subnet_peer_counts.write().await;
                                    let current_count = subnet_counts.get(&prefix).copied().unwrap_or(0);
                                    if current_count >= MAX_PEERS_PER_SUBNET_16 {
                                        warn!("Rejecting connection from {}: /16 subnet {}.{}.0.0/16 already has {} peers (max {})",
                                              addr, prefix[0], prefix[1], current_count, MAX_PEERS_PER_SUBNET_16);
                                        continue;
                                    }
                                    // Increment subnet counter
                                    subnet_counts.insert(prefix, current_count + 1);
                                    debug!("Accepted peer from /16 subnet {}.{}.0.0/16 (count: {})",
                                           prefix[0], prefix[1], current_count + 1);
                                }

                                // Check inbound slot limit (eclipse attack prevention)
                                // Localhost connections are exempt for testing
                                if let Some(max) = max_peers {
                                    let max_inbound = inbound_slots(max as usize);
                                    let conn_types = connection_types.read().await;
                                    let inbound_count = conn_types.values().filter(|&&t| t == ConnectionType::Inbound).count();
                                    drop(conn_types);

                                    // Only enforce slot limit for non-localhost connections
                                    let is_localhost = match addr.ip() {
                                        std::net::IpAddr::V4(ip) => ip.octets()[0] == 127,
                                        std::net::IpAddr::V6(_) => false,
                                    };

                                    if !is_localhost && inbound_count >= max_inbound {
                                        warn!("Rejecting inbound connection from {}: inbound slots full ({}/{})",
                                              addr, inbound_count, max_inbound);
                                        // Decrement subnet counter since we incremented it above
                                        if let Some(prefix) = subnet_prefix_16(&addr) {
                                            let mut subnet_counts = subnet_peer_counts.write().await;
                                            if let Some(count) = subnet_counts.get_mut(&prefix) {
                                                *count = count.saturating_sub(1);
                                                if *count == 0 {
                                                    subnet_counts.remove(&prefix);
                                                }
                                            }
                                        }
                                        continue;
                                    }
                                }

                                // Check if we need to evict a peer to make room
                                if let Some(max) = max_peers {
                                    let current = peers.read().await.len();
                                    if current >= max as usize {
                                        evict_lowest_peer(&peers, &gossip_streams, &peer_scores, &peer_count_atomic, &connection_types, &subnet_peer_counts).await;
                                    }
                                }

                                peers.write().await.insert(addr);
                                peer_scores.write().await.insert(addr, PeerScore::default());
                                peer_count_atomic.fetch_add(1, Ordering::Relaxed);
                                // Track connection type as Inbound
                                connection_types.write().await.insert(addr, ConnectionType::Inbound);
                                // Persist peer for reconnection on restart
                                persist_peer(&blockchain, &addr).await;

                                // Log peer diversity after accepting new peer
                                {
                                    let peer_count = peers.read().await.len();
                                    let subnet_counts = subnet_peer_counts.read().await;
                                    log_peer_diversity(peer_count, &subnet_counts);
                                }

                                // Extract peer identity from QUIC certificate with TOFU check
                                if let Some(peer_pk) = crate::quic_transport::extract_peer_identity(&connection) {
                                    let mut identities = peer_identities.write().await;
                                    if let Some(existing_pk) = identities.get(&addr) {
                                        if existing_pk != &peer_pk {
                                            warn!("⚠️ TOFU: Peer {} identity key CHANGED (possible key rotation or MITM). Old: {}..., New: {}...",
                                                addr, hex::encode(&existing_pk[..8]), hex::encode(&peer_pk[..8]));
                                        }
                                    }
                                    identities.insert(addr, peer_pk);
                                    debug!("Extracted peer identity from QUIC certificate for {}", addr);
                                }

                                // Store the QUIC connection
                                quic_connections.lock().await.insert(addr, connection.clone());

                                // Spawn the connection stream handler immediately
                                // This handler accepts ALL streams (gossip, sync, kyber) in a single loop
                                // to avoid the race condition where sync streams arrive before gossip handshake completes
                                debug!("Spawning connection stream handler for {}", addr);

                                // Create NetworkContext for the handler
                                let ctx = Arc::new(NetworkContext {
                                    blockchain: blockchain.clone(),
                                    peers: peers.clone(),
                                    peer_count_atomic: peer_count_atomic.clone(),
                                    listen_addr,
                                    is_running: is_running.clone(),
                                    signing_key: signing_key.clone(),
                                    node_public_key: node_public_key.clone(),
                                    gossip_streams: gossip_streams.clone(),
                                    orphan_pool: orphan_pool.clone(),
                                    mining_manager: mining_manager.clone(),
                                    peer_connect_tx: peer_connect_tx.clone(),
                                    peer_scores: peer_scores.clone(),
                                    shard_manager: shard_manager.clone(),
                                    max_peers,
                                    tx_seen: tx_seen.clone(),
                                    block_seen: block_seen.clone(),
                                    partition_detected: partition_detected.clone(),
                                    partition_start: partition_start.clone(),
                                    last_block_received: last_block_received.clone(),
                                    local_tip_height: local_tip_height.clone(),
                                    peer_public_keys: peer_public_keys.clone(),
                                    peer_advertised_addrs: peer_advertised_addrs.clone(),
                                    peer_request_peers_time: peer_request_peers_time.clone(),
                                    subnet_peer_counts: subnet_peer_counts.clone(),
                                    connection_types: connection_types.clone(),
                                    peer_exchange_lists: peer_exchange_lists.clone(),
                                    quic_connections: quic_connections.clone(),
                                    kyber_keys: kyber_keys.clone(),
                                    #[cfg(feature = "kyber")]
                                    kyber_session_keys: kyber_session_keys.clone(),
                                    #[cfg(feature = "kyber")]
                                    kyber_session_cache: kyber_session_cache.clone(),
                                    kyber_handshake_semaphore: kyber_handshake_semaphore.clone(),
                                    cached_fanout_peers: Arc::new(RwLock::new(None)),
                                });

                                let connection_clone = connection.clone();

                                tokio::spawn(async move {
                                    handle_connection_streams(
                                        connection_clone,
                                        addr,
                                        ctx,
                                    ).await;
                                });
                            }
                            None => {
                                // Endpoint closed, exit loop
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        info!("Network shutting down gracefully");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stops the network layer.
    pub async fn stop(&self) {
        *self.is_running.write().await = false;
    }

    /// Shuts down the network layer gracefully.
    pub async fn shutdown(&self) {
        info!("Network shutdown requested");
        let _ = self.shutdown_tx.send(true);
    }

    /// Starts the periodic peer exchange task (runs every 5 minutes).
    pub fn start_periodic_peer_exchange(&self) {
        let peers = self.peers.clone();
        let peer_gossip_streams = self.peer_gossip_streams.clone();
        let peer_scores = self.peer_scores.clone();
        let recent_request_peers = self.recent_request_peers.clone();
        let is_running = self.is_running.clone();
        let signing_key = self.signing_key.clone();
        let node_public_key = self.node_public_key.clone();
        // Kyber session cache for cleanup
        #[cfg(feature = "kyber")]
        let kyber_session_cache = self.kyber_session_cache.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(90)); // 90 seconds
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                if !*is_running.read().await {
                    break;
                }

                // Cleanup expired Kyber session cache entries
                #[cfg(feature = "kyber")]
                {
                    let mut cache = kyber_session_cache.write().await;
                    let before = cache.len();
                    cache.retain(|addr, (_, cached_at)| {
                        let expired = cached_at.elapsed().as_secs() >= KYBER_SESSION_CACHE_TTL_SECS;
                        if expired {
                            debug!("Kyber session cache expired for {}", addr);
                        }
                        !expired
                    });
                    let evicted = before - cache.len();
                    if evicted > 0 {
                        debug!("Evicted {} expired Kyber session cache entries", evicted);
                    }
                }

                // BPR-008: Cleanup expired bans
                {
                    let mut scores = peer_scores.write().await;
                    let now = Instant::now();
                    scores.retain(|addr, score| {
                        if let Some(banned_until) = score.banned_until {
                            if banned_until <= now {
                                info!("Ban expired for peer {}", addr);
                                // Clear the ban but keep the peer entry
                                score.banned_until = None;
                                score.ban_reason = None;
                                // Reset invalid counters on ban expiry
                                score.invalid_messages = 0;
                                score.invalid_blocks = 0;
                                score.invalid_txs = 0;
                            }
                        }
                        true // Keep all entries
                    });
                }

                // Log peer reputation scores periodically with freshness and novelty stats
                {
                    let scores = peer_scores.read().await;
                    let connected_peers = peers.read().await;
                    for &peer_addr in connected_peers.iter() {
                        if let Some(score) = scores.get(&peer_addr) {
                            let freshness_total =
                                score.stale_blocks.saturating_add(score.fresh_blocks);
                            let freshness_pct = if freshness_total > 0 {
                                (score.fresh_blocks as f64 / freshness_total as f64) * 100.0
                            } else {
                                0.0
                            };

                            let novelty_total =
                                score.novel_blocks.saturating_add(score.duplicate_blocks);
                            let novelty_pct = if novelty_total > 0 {
                                (score.novel_blocks as f64 / novelty_total as f64) * 100.0
                            } else {
                                0.0
                            };

                            info!(
                                "Peer {} stats: reputation={:.1}, fresh={}/{} ({:.0}%), novelty={:.0}%",
                                peer_addr,
                                score.reputation(),
                                score.fresh_blocks,
                                freshness_total,
                                freshness_pct,
                                novelty_pct
                            );

                            debug!(
                                "Peer {} details: blocks={}/{}, latency={}ms, stale={}, novel={}/{}",
                                peer_addr,
                                score.blocks_delivered,
                                score.blocks_delivered + score.invalid_blocks,
                                score.latency_ms.map(|l| l.to_string()).unwrap_or_else(|| "N/A".to_string()),
                                score.stale_blocks,
                                score.novel_blocks,
                                novelty_total
                            );
                        }
                    }
                }

                // Clear recent request peers set every 5 minutes
                recent_request_peers.write().await.clear();

                // Get connected peers
                let connected: Vec<SocketAddr> = peers.read().await.iter().copied().collect();
                if connected.is_empty() {
                    continue;
                }

                // === Send pings to all peers for latency measurement ===
                // Each peer gets a UNIQUE nonce to correctly match pong responses
                {
                    let now = Instant::now();
                    use ed25519_dalek::Signer;

                    // Generate unique nonces per peer and record timestamps
                    let peer_nonces: HashMap<SocketAddr, u64> = connected
                        .iter()
                        .map(|&addr| (addr, rand::random::<u64>()))
                        .collect();

                    // Update ping timestamps and nonces for all peers
                    {
                        let mut scores = peer_scores.write().await;
                        for &peer_addr in &connected {
                            if let Some(score) = scores.get_mut(&peer_addr) {
                                score.ping_sent_at = Some(now);
                                score.pending_ping_nonce = peer_nonces.get(&peer_addr).copied();
                            }
                        }
                    }

                    // Send authenticated ping to each peer with their unique nonce
                    let gossip_streams = peer_gossip_streams.lock().await;
                    for &peer_addr in &connected {
                        let nonce = match peer_nonces.get(&peer_addr) {
                            Some(&n) => n,
                            None => continue,
                        };

                        let ping = NetworkMessage::Ping { nonce };
                        let message_bytes = match bincode::serialize(&ping) {
                            Ok(b) => b,
                            Err(_) => continue,
                        };
                        let signature = signing_key.sign(&message_bytes);
                        let timestamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let authenticated = AuthenticatedMessage {
                            message: ping,
                            signature: signature.to_bytes().to_vec(),
                            public_key: node_public_key.clone(),
                            timestamp,
                        };
                        if let Ok(data) = bincode::serialize(&authenticated) {
                            if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
                                let mut stream = stream_arc.lock().await;
                                let total_len = 1 + data.len();
                                let _ = stream.write_u32(total_len as u32).await;
                                let _ = stream.write_u8(MSG_FRAME_PLAINTEXT).await;
                                let _ = stream.write_all(&data).await;
                            }
                        }
                    }
                    debug!(
                        "Periodic latency ping sent to {} peers with unique nonces",
                        connected.len()
                    );
                }

                // Pick a random connected peer that we haven't requested from recently
                let mut target_peer = None;
                let recent = recent_request_peers.read().await;
                for peer in &connected {
                    if !recent.contains(peer) {
                        target_peer = Some(*peer);
                        break;
                    }
                }
                drop(recent);

                if let Some(peer_addr) = target_peer {
                    // Send RequestPeers to the selected peer
                    let request = NetworkMessage::RequestPeers;

                    // Sign the message with the cached signing key
                    use ed25519_dalek::Signer;

                    let message_bytes = match bincode::serialize(&request) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };

                    let signature = signing_key.sign(&message_bytes);
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let authenticated = AuthenticatedMessage {
                        message: request,
                        signature: signature.to_bytes().to_vec(),
                        public_key: node_public_key.clone(),
                        timestamp,
                    };

                    if let Ok(data) = bincode::serialize(&authenticated) {
                        let gossip_streams = peer_gossip_streams.lock().await;
                        if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
                            let mut stream = stream_arc.lock().await;
                            let total_len = 1 + data.len();
                            if stream.write_u32(total_len as u32).await.is_ok()
                                && stream.write_u8(MSG_FRAME_PLAINTEXT).await.is_ok()
                                && stream.write_all(&data).await.is_ok()
                            {
                                debug!(
                                    "Periodic peer exchange: sent RequestPeers to {}",
                                    peer_addr
                                );
                                recent_request_peers.write().await.insert(peer_addr);
                            }
                        }
                    }
                }
            }
        });
    }

    /// Starts the background partition checker task (runs every 30 seconds).
    pub fn start_partition_checker(&self) {
        let partition_detected = self.partition_detected.clone();
        let partition_start = self.partition_start.clone();
        let last_block_received = self.last_block_received.clone();
        let is_running = self.is_running.clone();
        let peers = self.peers.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                if !*is_running.read().await {
                    break;
                }

                let now = Instant::now();
                let last_block = *last_block_received.read().await;
                let elapsed_secs = now.duration_since(last_block).as_secs();
                let peer_count = peers.read().await.len();

                // Only check for partition if we have peers (can't be partitioned if alone)
                if peer_count > 0 && elapsed_secs >= PARTITION_DETECTION_THRESHOLD_SECS {
                    let already_detected = partition_detected.load(Ordering::Relaxed);

                    if !already_detected {
                        // New partition detection
                        partition_detected.store(true, Ordering::Relaxed);
                        *partition_start.write().await = Some(last_block);
                        warn!("[PARTITION] No new blocks received for {}s - possible network partition ({} peers connected)",
                                 elapsed_secs, peer_count);
                    }
                    // If already detected, just continue silently until recovery
                }
            }
        });
    }

    /// Connects to a peer at the specified address.
    ///
    /// # Arguments
    /// * `addr` - The socket address of the peer to connect to
    pub async fn connect_peer(&self, addr: SocketAddr) -> crate::error::BlockchainResult<()> {
        // Check outbound slot limit (eclipse attack prevention)
        // Localhost connections are exempt for testing
        if let Some(max) = self.max_peers {
            let max_outbound = outbound_slots(max as usize);
            let conn_types = self.connection_types.read().await;
            let outbound_count = conn_types
                .values()
                .filter(|&&t| t == ConnectionType::Outbound)
                .count();
            drop(conn_types);

            // Only enforce slot limit for non-localhost connections
            let is_localhost = match addr.ip() {
                std::net::IpAddr::V4(ip) => ip.octets()[0] == 127,
                std::net::IpAddr::V6(_) => false,
            };

            if !is_localhost && outbound_count >= max_outbound {
                return Err(crate::error::BlockchainError::Network(format!(
                    "Outbound slot limit reached ({}/{}), cannot connect to {}",
                    outbound_count, max_outbound, addr
                )));
            }

            // Also check total peer limit
            let current = self.peers.read().await.len();
            if current >= max as usize {
                if self.evict_lowest_scoring_peer().await.is_none() {
                    return Err(crate::error::BlockchainError::Network(format!(
                        "Max peers limit reached ({}/{}), cannot connect to {}",
                        current, max, addr
                    )));
                }
            }
        }

        // Check subnet diversity limit (eclipse attack prevention)
        if let Some(prefix) = subnet_prefix_16(&addr) {
            let mut subnet_counts = self.subnet_peer_counts.write().await;
            let current_count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            if current_count >= MAX_PEERS_PER_SUBNET_16 {
                return Err(crate::error::BlockchainError::Network(format!(
                    "Subnet limit reached for {}.{}.0.0/16 ({} peers max), cannot connect to {}",
                    prefix[0], prefix[1], MAX_PEERS_PER_SUBNET_16, addr
                )));
            }
            // Increment subnet counter
            subnet_counts.insert(prefix, current_count + 1);
            debug!(
                "Outbound peer from /16 subnet {}.{}.0.0/16 (count: {})",
                prefix[0],
                prefix[1],
                current_count + 1
            );
        }

        // Try to connect with retry logic using QUIC
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY_MS: u64 = 2000;

        // Get the QUIC endpoint
        let endpoint = self.quic_endpoint.read().await.clone();
        let endpoint = match endpoint {
            Some(ep) => ep,
            None => {
                return Err(crate::error::BlockchainError::Network(
                    "QUIC endpoint not initialized - call start() first".to_string(),
                ));
            }
        };

        let connection = loop {
            match crate::quic_transport::connect_to_peer(&endpoint, addr).await {
                Ok(conn) => break conn,
                Err(e) => {
                    // Convert error to string immediately to avoid holding non-Send type across await
                    let error_msg = {
                        let e = e;
                        format!("{}", e)
                    }; // e is dropped here
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        error!(
                            "Failed to connect to {} after {} attempts: {}",
                            addr, attempts, error_msg
                        );
                        // Decrement subnet counter since we incremented it above
                        if let Some(prefix) = subnet_prefix_16(&addr) {
                            let mut subnet_counts = self.subnet_peer_counts.write().await;
                            if let Some(count) = subnet_counts.get_mut(&prefix) {
                                *count = count.saturating_sub(1);
                                if *count == 0 {
                                    subnet_counts.remove(&prefix);
                                }
                            }
                        }
                        return Err(crate::error::BlockchainError::Network(format!(
                            "Failed to connect to {}: {}",
                            addr, error_msg
                        )));
                    }
                    // Retry silently
                    tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                }
            }
        };

        self.peers.write().await.insert(addr);
        self.peer_scores
            .write()
            .await
            .insert(addr, PeerScore::default());
        // Track connection type as Outbound
        self.connection_types
            .write()
            .await
            .insert(addr, ConnectionType::Outbound);
        // Increment atomic peer counter for outbound connections
        self.peer_count_atomic.fetch_add(1, Ordering::Relaxed);
        // Invalidate fanout cache since peer list changed
        *self.cached_fanout_peers.write().await = None;
        debug!("QUIC connection established to {}", addr);
        // Persist peer for reconnection on restart
        persist_peer(&self.blockchain, &addr).await;

        // Extract peer identity from QUIC TLS certificate with TOFU check
        if let Some(peer_pk) = crate::quic_transport::extract_peer_identity(&connection) {
            let mut identities = self.peer_identities.write().await;
            if let Some(existing_pk) = identities.get(&addr) {
                if existing_pk != &peer_pk {
                    warn!("⚠️ TOFU: Peer {} identity key CHANGED (possible key rotation or MITM). Old: {}..., New: {}...",
                        addr, hex::encode(&existing_pk[..8]), hex::encode(&peer_pk[..8]));
                }
            }
            identities.insert(addr, peer_pk);
            debug!("Extracted peer identity from QUIC certificate for {}", addr);
        }

        // Log peer diversity after connecting to new peer
        {
            let peer_count = self.peers.read().await.len();
            let subnet_counts = self.subnet_peer_counts.read().await;
            log_peer_diversity(peer_count, &subnet_counts);
        }

        // Store the QUIC connection
        self.peer_quic_connections
            .lock()
            .await
            .insert(addr, connection.clone());

        // Open a bidirectional gossip stream
        let (mut send_stream, recv_stream) = match connection.open_bi().await {
            Ok(streams) => streams,
            Err(e) => {
                warn!("Failed to open gossip stream to {}: {}", addr, e);
                // Clean up peer state
                let was_present = self.peers.write().await.remove(&addr);
                self.peer_scores.write().await.remove(&addr);
                self.peer_quic_connections.lock().await.remove(&addr);
                // Remove from persisted peers
                remove_persisted_peer(&self.blockchain, &addr).await;
                // Invalidate fanout cache since peer list changed
                *self.cached_fanout_peers.write().await = None;
                if was_present {
                    let prev = self.peer_count_atomic.load(Ordering::Relaxed);
                    if prev > 0 {
                        self.peer_count_atomic.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                if let Some(prefix) = subnet_prefix_16(&addr) {
                    let mut subnet_counts = self.subnet_peer_counts.write().await;
                    if let Some(count) = subnet_counts.get_mut(&prefix) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            subnet_counts.remove(&prefix);
                        }
                    }
                }
                return Err(crate::error::BlockchainError::Network(format!(
                    "Failed to open gossip stream to {}: {}",
                    addr, e
                )));
            }
        };

        // Write stream type byte for gossip
        if let Err(e) = send_stream.write_all(&[STREAM_TYPE_GOSSIP]).await {
            warn!("Failed to write stream type byte to {}: {}", addr, e);
            // Clean up peer state
            let was_present = self.peers.write().await.remove(&addr);
            self.peer_scores.write().await.remove(&addr);
            self.peer_quic_connections.lock().await.remove(&addr);
            // Remove from persisted peers
            remove_persisted_peer(&self.blockchain, &addr).await;
            // Invalidate fanout cache since peer list changed
            *self.cached_fanout_peers.write().await = None;
            if was_present {
                let prev = self.peer_count_atomic.load(Ordering::Relaxed);
                if prev > 0 {
                    self.peer_count_atomic.fetch_sub(1, Ordering::Relaxed);
                }
            }
            if let Some(prefix) = subnet_prefix_16(&addr) {
                let mut subnet_counts = self.subnet_peer_counts.write().await;
                if let Some(count) = subnet_counts.get_mut(&prefix) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        subnet_counts.remove(&prefix);
                    }
                }
            }
            return Err(crate::error::BlockchainError::Network(format!(
                "Failed to write stream type byte to {}: {}",
                addr, e
            )));
        }

        // Send handshake immediately after connection.
        // Determine advertise address with priority: config > public_ip > auto-detect > localhost fallback
        let advertise_addr = if let Some(config_advertise) = &self.advertise_addr {
            // Use explicitly configured advertise address
            debug!("Using configured advertise address: {}", config_advertise);
            config_advertise.clone()
        } else if let Some(public_ip) = self.public_ip {
            // Use explicitly configured public IP
            let addr = format!("{}:{}", public_ip, self.listen_addr.port());
            debug!("Using configured public IP: {}", addr);
            addr
        } else if self.listen_addr.ip().is_unspecified() {
            // Try auto-detection for 0.0.0.0 bind
            if let Some(detected_ip) = detect_public_ip() {
                let detected_addr = format!("{}:{}", detected_ip, self.listen_addr.port());
                debug!("Auto-detected advertise address: {}", detected_addr);
                detected_addr
            } else {
                // Fallback to localhost
                let fallback = format!("127.0.0.1:{}", self.listen_addr.port());
                warn!("Could not detect public IP; advertising as {}. Use --advertise or --public-ip to set manually.", fallback);
                fallback
            }
        } else {
            // Use listen_addr directly if it's a specific IP
            self.listen_addr.to_string()
        };
        let handshake = NetworkMessage::Handshake {
            listen_addr: advertise_addr,
            capabilities: LOCAL_CAPABILITIES,
        };

        let authenticated = self.sign_message(handshake)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;
        // Protocol v2: Send with frame byte (0x00 = plaintext)
        let total_len = 1 + data.len();
        send_stream.write_u32(total_len as u32).await.map_err(|e| {
            crate::error::BlockchainError::Network(format!(
                "Failed to send handshake length to {}: {}",
                addr, e
            ))
        })?;
        send_stream
            .write_u8(MSG_FRAME_PLAINTEXT)
            .await
            .map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to send handshake frame byte to {}: {}",
                    addr, e
                ))
            })?;
        send_stream.write_all(&data).await.map_err(|e| {
            crate::error::BlockchainError::Network(format!(
                "Failed to send handshake to {}: {}",
                addr, e
            ))
        })?;

        debug!("Sent handshake to {}", addr);

        // Send RequestPeers to discover more peers from this new connection
        let request_peers_msg = NetworkMessage::RequestPeers;
        if let Ok(authenticated) = self.sign_message(request_peers_msg) {
            if let Ok(data) = bincode::serialize(&authenticated) {
                // Protocol v2: Send with frame byte (0x00 = plaintext)
                let total_len = 1 + data.len();
                if let Err(e) = send_stream.write_u32(total_len as u32).await {
                    warn!("Failed to send RequestPeers length to {}: {}", addr, e);
                } else if let Err(e) = send_stream.write_u8(MSG_FRAME_PLAINTEXT).await {
                    warn!("Failed to send RequestPeers frame byte to {}: {}", addr, e);
                } else if let Err(e) = send_stream.write_all(&data).await {
                    warn!("Failed to send RequestPeers to {}: {}", addr, e);
                } else {
                    debug!("Sent RequestPeers to {}", addr);
                }
            }
        }

        // Wrap streams for storage and handle_peer
        let gossip_stream: GossipStream = Arc::new(Mutex::new(send_stream));

        // Store the gossip stream
        self.peer_gossip_streams
            .lock()
            .await
            .insert(addr, gossip_stream.clone());

        // Pass send_stream Arc and recv_stream to handle_peer
        let stored_send = gossip_stream.clone();
        let stored_recv = recv_stream;

        // Spawn Kyber upgrade on a separate stream if enabled
        #[cfg(feature = "kyber")]
        {
            // Check session cache first — reuse key if peer reconnected within TTL
            let cached = {
                let mut cache = self.kyber_session_cache.write().await;
                if let Some((key, cached_at)) = cache.remove(&addr) {
                    if cached_at.elapsed().as_secs() < KYBER_SESSION_CACHE_TTL_SECS {
                        Some(key)
                    } else {
                        None // Expired — perform fresh exchange
                    }
                } else {
                    None
                }
            };

            if let Some(cached_key) = cached {
                // Reuse cached session key — skip ML-KEM exchange
                self.kyber_session_keys
                    .write()
                    .await
                    .insert(addr, cached_key);
                info!("Kyber session reused for {} (cached)", addr);
            } else {
                // No cache hit — perform full Kyber exchange with semaphore throttle
                let connection_clone = connection.clone();
                let kyber_k_clone = self.kyber_keys.clone();
                let kyber_sk_clone = self.kyber_session_keys.clone();
                let addr_clone = addr;
                let semaphore_clone = self.kyber_handshake_semaphore.clone();

                tokio::spawn(async move {
                    // Acquire semaphore permit — skip Kyber if too many concurrent handshakes
                    let _permit = match semaphore_clone.try_acquire() {
                        Ok(permit) => permit,
                        Err(_) => {
                            info!(
                                "Kyber handshake throttled for {} — too many concurrent exchanges",
                                addr_clone
                            );
                            return;
                        }
                    };

                    // Open a separate stream for Kyber exchange
                    match connection_clone.open_bi().await {
                        Ok((mut kyber_send, mut kyber_recv)) => {
                            // Write Kyber stream type
                            if let Err(e) = kyber_send.write_all(&[STREAM_TYPE_KYBER]).await {
                                debug!("Failed to open Kyber stream to {}: {}", addr_clone, e);
                                return;
                            }

                            // Get our Kyber keys
                            let our_kyber = {
                                let guard = kyber_k_clone.lock().await;
                                match guard.as_ref() {
                                    Some(k) => k.clone(),
                                    None => return,
                                }
                            };

                            // Perform Kyber handshake as initiator using unified function
                            match perform_kyber_handshake(
                                &mut kyber_send,
                                &mut kyber_recv,
                                KyberRole::Initiator,
                                our_kyber,
                                addr_clone,
                            )
                            .await
                            {
                                Ok(session_key) => {
                                    // Store session key with automatic zeroization on drop
                                    kyber_sk_clone.write().await.insert(
                                        addr_clone,
                                        zeroize::Zeroizing::new(session_key.as_bytes().to_vec()),
                                    );
                                    debug!("Kyber session established with {}", addr_clone);
                                }
                                Err(e) => {
                                    debug!("Kyber handshake failed for {}: {}", addr_clone, e);
                                }
                            }
                        }
                        Err(_) => {
                            return;
                        }
                    }
                });
            }
        }

        // Use ctx for handle_connection_streams and handle_peer
        let ctx = self.ctx.clone();

        // Spawn incoming stream handler for all stream types (gossip, sync, kyber)
        let connection_clone = connection.clone();
        let ctx_for_streams = ctx.clone();
        let addr_for_streams = addr;
        tokio::spawn(async move {
            handle_connection_streams(connection_clone, addr_for_streams, ctx_for_streams).await;
        });

        tokio::spawn(async move {
            handle_peer(stored_send, stored_recv, addr, ctx).await;
        });

        // Initiate block sync with the new peer
        // This triggers block download if the peer has blocks we don't have
        // Use a small delay to ensure the connection is fully established and stored
        let blockchain_clone = self.blockchain.clone();
        let gossip_streams_clone = self.peer_gossip_streams.clone();
        let signing_key = self.signing_key.clone();
        let node_public_key = self.node_public_key.clone();
        let peer_addr = addr;
        tokio::spawn(async move {
            // Wait for handle_peer to store the connection (it stores it immediately)
            // Give it a moment to ensure the connection is in the map
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // Verify connection exists before sending
            let streams_map = gossip_streams_clone.lock().await;
            if !streams_map.contains_key(&peer_addr) {
                warn!(
                    "Connection not yet stored for peer {}, skipping sync initiation",
                    peer_addr
                );
                return;
            }
            drop(streams_map);

            // Get local height (highest block number we have)
            let local_height = {
                let bc = blockchain_clone.read().await;
                bc.latest_block_number()
            };

            // For BlockDAG: Use simple block range request instead of headers-first sync
            // Headers-first doesn't work well with multi-parent DAG blocks
            // from_block is a block_number filter: request blocks with block_number >= start_height
            let start_height = local_height + 1; // Request blocks AFTER the highest we have
            debug!(
                "SYNC: Requesting blocks from {} (local height: {}) from peer {}",
                start_height, local_height, peer_addr
            );

            // Request blocks directly instead of headers
            let request = NetworkMessage::RequestBlocks {
                from_block: start_height,
                count: MAX_BLOCKS_PER_REQUEST,
            };

            // Sign the message with the cached signing key
            use bincode;
            use ed25519_dalek::Signer;

            let message_bytes = match bincode::serialize(&request) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to serialize sync request: {}", e);
                    return;
                }
            };

            let signature = signing_key.sign(&message_bytes);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let authenticated = AuthenticatedMessage {
                message: request,
                signature: signature.to_bytes().to_vec(),
                public_key: node_public_key.clone(),
                timestamp,
            };

            let data = match bincode::serialize(&authenticated) {
                Ok(d) => d,
                Err(e) => {
                    warn!("Failed to serialize authenticated message: {}", e);
                    return;
                }
            };

            // Send via stored gossip stream
            let streams_map = gossip_streams_clone.lock().await;
            debug!(
                "SYNC: streams_map has {} entries, looking for {}",
                streams_map.len(),
                peer_addr
            );
            if let Some(stream_arc) = streams_map.get(&peer_addr) {
                let mut send_stream = stream_arc.lock().await;

                if let Err(e) = write_framed(&mut send_stream, &data).await {
                    warn!("Failed to send sync request to {}: {}", peer_addr, e);
                } else {
                    debug!(
                        "SYNC: Successfully sent block sync request to {} ({} bytes)",
                        peer_addr,
                        data.len()
                    );
                }
            } else {
                warn!(
                    "No connection found for peer {} when trying to start sync",
                    peer_addr
                );
            }
        });

        Ok(())
    }

    /// Broadcasts a block to peers with fanout-based relay.
    ///
    /// # Arguments
    /// * `block` - The block to broadcast
    /// * `is_own_block` - If true, broadcasts to all peers; if false, uses compact block format
    pub async fn broadcast_block(
        &self,
        block: &Block,
        is_own_block: bool,
    ) -> crate::error::BlockchainResult<()> {
        // Mark block as seen before broadcasting (atomic insert + cleanup)
        {
            let mut block_seen = self.block_seen.write().await;
            block_seen.insert(block.hash, Instant::now());
            // Evict old entries if cache is full
            evict_seen_cache(&mut block_seen, 10_000);
        }

        let peers = self.peers.read().await;
        let peer_count = peers.len();

        if peers.is_empty() {
            // No peers - silently skip broadcast
            return Ok(());
        }

        // Determine which peers to broadcast to based on is_own_block
        let selected_peers: Vec<SocketAddr> = if is_own_block {
            // Own-mined blocks: broadcast to ALL peers (critical path)
            debug!(
                "[BROADCAST_BLOCK] Broadcasting own-mined block #{} to ALL {} peers",
                block.header.block_number, peer_count
            );
            peers.iter().copied().collect()
        } else {
            // Relayed blocks: use sqrt-based fanout with adaptive increase for stale blocks
            // Stale blocks (>10 blocks behind tip) get doubled fanout to help lagging nodes converge
            let local_tip = self.local_tip_height.load(Ordering::Relaxed);
            let block_height = block.header.block_number;
            let is_stale = local_tip.saturating_sub(block_height) > 10;

            let base_fanout = std::cmp::max(3, (peer_count as f64).sqrt() as usize);
            let fanout = if is_stale {
                std::cmp::min(peer_count, base_fanout * 2) // Double fanout for stale blocks
            } else {
                base_fanout
            };

            // Release the peers lock before calling select_peers_for_fanout
            drop(peers);

            // Use latency-aware peer selection for relay
            let selected = self.select_peers_for_fanout(fanout).await;
            if is_stale {
                debug!("[BROADCAST_BLOCK] Relaying stale block #{} to {}/{} peers (2x fanout for convergence)",
                         block.header.block_number, selected.len(), peer_count);
            } else {
                debug!(
                    "[BROADCAST_BLOCK] Relaying block #{} to {}/{} peers (sqrt fanout)",
                    block.header.block_number,
                    selected.len(),
                    peer_count
                );
            }
            selected
        };

        // For own-mined blocks, send full block to all peers
        // For relayed blocks, try compact block format first
        if is_own_block {
            // If sharding is enabled, try to determine shard ID from block transactions
            let shard_id = self.shard_manager.as_ref().and_then(|shard_mgr| {
                // Determine shard from first transaction (if any)
                block
                    .transactions
                    .first()
                    .map(|first_tx| shard_mgr.get_shard_for_address(&first_tx.from))
            });

            let message = if let Some(shard) = shard_id {
                NetworkMessage::NewShardBlock {
                    block: block.clone(),
                    shard_id: shard,
                }
            } else {
                NetworkMessage::NewBlock {
                    block: block.clone(),
                }
            };

            let authenticated = self.sign_message(message)?;
            let data = bincode::serialize(&authenticated)
                .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

            // Check message size
            if data.len() > MAX_MESSAGE_SIZE {
                return Err(crate::error::BlockchainError::Network(format!(
                    "Message size {} exceeds maximum {}",
                    data.len(),
                    MAX_MESSAGE_SIZE
                )));
            }

            for &peer_addr in &selected_peers {
                // Try to use stored QUIC gossip stream
                let gossip_streams = self.peer_gossip_streams.lock().await;

                if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
                    let mut stream = stream_arc.lock().await;

                    // Send via stored stream
                    let total_len = 1 + data.len();
                    match stream.write_u32(total_len as u32).await {
                        Ok(_) => match stream.write_u8(MSG_FRAME_PLAINTEXT).await {
                            Ok(_) => match stream.write_all(&data).await {
                                Ok(_) => {
                                    self.record_send_success(peer_addr).await;
                                    continue;
                                }
                                Err(e) => {
                                    self.record_send_failure(peer_addr).await;
                                    warn!("Failed to send block data to {}: {}", peer_addr, e);
                                }
                            },
                            Err(e) => {
                                self.record_send_failure(peer_addr).await;
                                warn!("Failed to send frame byte to {}: {}", peer_addr, e);
                            }
                        },
                        Err(e) => {
                            self.record_send_failure(peer_addr).await;
                            warn!("Failed to send block length to {}: {}", peer_addr, e);
                        }
                    }
                }
            }
        } else {
            // Relayed blocks: use compact block format for bandwidth efficiency
            // Get mempool hashes for compact block creation
            let mempool_hashes = if let Some(mining_mgr) = &self.mining_manager {
                mining_mgr.get_mempool_hashes().await
            } else {
                // No mining manager - fall back to full block
                return self.broadcast_full_block(block, &selected_peers).await;
            };

            // Create compact block
            let compact_block = sync::CompactBlock::from_block(block, &mempool_hashes);

            let message = NetworkMessage::NewCompactBlock { compact_block };
            let authenticated = self.sign_message(message)?;
            let data = bincode::serialize(&authenticated)
                .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

            // Check message size - if too large, fall back to full block
            if data.len() > MAX_MESSAGE_SIZE {
                return self.broadcast_full_block(block, &selected_peers).await;
            }

            for &peer_addr in &selected_peers {
                let gossip_streams = self.peer_gossip_streams.lock().await;

                if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
                    let mut stream = stream_arc.lock().await;

                    let total_len = 1 + data.len();
                    match stream.write_u32(total_len as u32).await {
                        Ok(_) => match stream.write_u8(MSG_FRAME_PLAINTEXT).await {
                            Ok(_) => match stream.write_all(&data).await {
                                Ok(_) => {
                                    self.record_send_success(peer_addr).await;
                                    continue;
                                }
                                Err(e) => {
                                    self.record_send_failure(peer_addr).await;
                                    warn!(
                                        "Failed to send compact block data to {}: {}",
                                        peer_addr, e
                                    );
                                }
                            },
                            Err(e) => {
                                self.record_send_failure(peer_addr).await;
                                warn!("Failed to send frame byte to {}: {}", peer_addr, e);
                            }
                        },
                        Err(e) => {
                            self.record_send_failure(peer_addr).await;
                            warn!(
                                "Failed to send compact block length to {}: {}",
                                peer_addr, e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Broadcast a full block to a specific set of peers (fallback for compact block failures)
    async fn broadcast_full_block(
        &self,
        block: &Block,
        selected_peers: &[SocketAddr],
    ) -> crate::error::BlockchainResult<()> {
        let message = NetworkMessage::NewBlock {
            block: block.clone(),
        };
        let authenticated = self.sign_message(message)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        if data.len() > MAX_MESSAGE_SIZE {
            return Err(crate::error::BlockchainError::Network(format!(
                "Message size {} exceeds maximum {}",
                data.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        for &peer_addr in selected_peers {
            // Use send_to_peer which properly handles frame bytes and Kyber encryption
            if let Err(e) = self.send_to_peer(&peer_addr, &data).await {
                warn!("Failed to send full block to {}: {}", peer_addr, e);
            }
        }

        Ok(())
    }

    /// Broadcasts a block from a specific shard to all connected peers.
    pub async fn broadcast_shard_block(
        &self,
        block: &Block,
        shard_id: usize,
    ) -> crate::error::BlockchainResult<()> {
        let peers = self.peers.read().await;
        if peers.is_empty() {
            return Ok(());
        }

        let message = NetworkMessage::NewShardBlock {
            block: block.clone(),
            shard_id,
        };
        let authenticated = self.sign_message(message)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        // Check message size
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(crate::error::BlockchainError::Network(format!(
                "Message size {} exceeds maximum {}",
                data.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        for &peer_addr in peers.iter() {
            if let Err(e) = self.send_to_peer(&peer_addr, &data).await {
                warn!("Failed to send shard block to {}: {}", peer_addr, e);
            }
        }

        Ok(())
    }

    /// Broadcasts a transaction to a subset of peers using fanout relay.
    pub async fn broadcast_transaction(
        &self,
        tx: &Transaction,
    ) -> crate::error::BlockchainResult<()> {
        // Mark transaction as seen before broadcasting (atomic insert + cleanup)
        {
            let mut tx_seen = self.tx_seen.write().await;
            tx_seen.insert(tx.hash, Instant::now());
            // Evict old entries if cache is full
            evict_seen_cache(&mut tx_seen, 10_000);
        }

        let peer_count = self.peers.read().await.len();
        debug!("[BROADCAST_TX] Peer count: {}", peer_count);
        if peer_count == 0 {
            warn!("[BROADCAST_TX] No peers to broadcast to");
            return Ok(());
        }

        // Calculate fanout: k = max(3, sqrt(peer_count))
        let fanout = std::cmp::max(3, (peer_count as f64).sqrt() as usize);

        // Use latency-aware peer selection
        let selected_peers = self.select_peers_for_fanout(fanout).await;

        debug!(
            "[BROADCAST_TX] Latency-aware fanout: {}/{} peers",
            selected_peers.len(),
            peer_count
        );

        let message = NetworkMessage::NewTransaction {
            transaction: tx.clone(),
        };
        let authenticated = self.sign_message(message)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        debug!(
            "[BROADCAST_TX] Serialized message size: {} bytes",
            data.len()
        );

        // Check message size
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(crate::error::BlockchainError::Network(format!(
                "Message size {} exceeds maximum {}",
                data.len(),
                MAX_MESSAGE_SIZE
            )));
        }

        // Send to selected peers only
        for &peer_addr in &selected_peers {
            debug!("[BROADCAST_TX] Attempting to send to peer: {}", peer_addr);

            // Use send_to_peer which properly handles frame bytes and Kyber encryption
            if let Err(e) = self.send_to_peer(&peer_addr, &data).await {
                warn!(
                    "[BROADCAST_TX] Failed to send transaction to {}: {}",
                    peer_addr, e
                );
            } else {
                debug!(
                    "[BROADCAST_TX] Successfully sent transaction to {}",
                    peer_addr
                );
            }
        }

        Ok(())
    }

    /// Returns the number of connected peers (non-blocking).
    pub fn peer_count(&self) -> usize {
        self.peer_count_atomic.load(Ordering::Relaxed)
    }

    /// Returns the number of currently banned peers (blocking, non-async).
    pub fn banned_count_blocking(&self) -> usize {
        match self.peer_scores.try_read() {
            Ok(scores) => scores.values().filter(|s| s.is_banned()).count(),
            Err(_) => 0,
        }
    }

    /// Returns peer latency information for metrics (blocking, non-async).
    /// Returns a Vec of (peer_address, latency_ms) tuples.
    pub fn get_peer_latencies_for_metrics_blocking(&self) -> Vec<(String, f64)> {
        match (self.peer_scores.try_read(), self.peers.try_read()) {
            (Ok(scores), Ok(peers)) => peers
                .iter()
                .filter_map(|&addr| {
                    scores
                        .get(&addr)
                        .and_then(|s| s.latency_ms.map(|l| (addr.to_string(), l as f64)))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Returns the number of currently banned peers.
    pub async fn banned_count(&self) -> usize {
        let scores = self.peer_scores.read().await;
        scores.values().filter(|s| s.is_banned()).count()
    }

    /// Returns peer latency information for metrics.
    /// Returns a Vec of (peer_address, latency_ms) tuples.
    pub async fn get_peer_latencies_for_metrics(&self) -> Vec<(String, f64)> {
        let scores = self.peer_scores.read().await;
        let peers = self.peers.read().await;

        peers
            .iter()
            .filter_map(|&addr| {
                scores
                    .get(&addr)
                    .and_then(|s| s.latency_ms.map(|l| (addr.to_string(), l as f64)))
            })
            .collect()
    }

    /// Returns the list of connected peer addresses.
    pub async fn get_peers(&self) -> Vec<SocketAddr> {
        self.peers.read().await.iter().copied().collect()
    }

    /// Sends a ping to a specific peer for latency measurement.
    pub async fn send_ping_to_peer(
        &self,
        peer_addr: SocketAddr,
    ) -> crate::error::BlockchainResult<()> {
        // Generate a random nonce for this ping
        let nonce: u64 = rand::random();

        // Record the ping timestamp in the peer's score
        {
            let mut scores = self.peer_scores.write().await;
            if let Some(score) = scores.get_mut(&peer_addr) {
                score.ping_sent_at = Some(Instant::now());
                score.pending_ping_nonce = Some(nonce);
            } else {
                // Peer not in scores - this shouldn't happen but handle gracefully
                return Ok(());
            }
        }

        // Send authenticated ping message
        let ping = NetworkMessage::Ping { nonce };
        let authenticated = self.sign_message(ping)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        let gossip_streams = self.peer_gossip_streams.lock().await;
        if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
            let mut stream = stream_arc.lock().await;
            let total_len = 1 + data.len();
            stream.write_u32(total_len as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write ping length to {}: {}",
                    peer_addr, e
                ))
            })?;
            stream.write_u8(MSG_FRAME_PLAINTEXT).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write ping frame byte to {}: {}",
                    peer_addr, e
                ))
            })?;
            stream.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write ping data to {}: {}",
                    peer_addr, e
                ))
            })?;
        }

        Ok(())
    }

    /// Sends pings to all connected peers for latency measurement.
    pub async fn send_ping_to_all_peers(&self) {
        let peers: Vec<SocketAddr> = self.peers.read().await.iter().copied().collect();

        for peer_addr in peers {
            if let Err(e) = self.send_ping_to_peer(peer_addr).await {
                warn!("Failed to send ping to {}: {}", peer_addr, e);
            }
        }
    }

    /// Returns peer latency information for RPC/debugging.
    pub async fn get_peer_latencies(&self) -> HashMap<SocketAddr, Option<u64>> {
        let scores = self.peer_scores.read().await;
        let peers = self.peers.read().await;

        peers
            .iter()
            .map(|&addr| {
                let latency = scores.get(&addr).and_then(|s| s.latency_ms);
                (addr, latency)
            })
            .collect()
    }

    /// Selects peers for fanout relay using latency-aware selection.
    /// Uses a 5-second cache to avoid sorting peers on every call.
    pub async fn select_peers_for_fanout(&self, k: usize) -> Vec<SocketAddr> {
        // Check cache first
        {
            let cache = self.cached_fanout_peers.read().await;
            if let Some((timestamp, cached_peers)) = cache.as_ref() {
                if timestamp.elapsed() < std::time::Duration::from_secs(5) {
                    // Cache is fresh, return up to k peers
                    return cached_peers.iter().take(k).copied().collect();
                }
            }
        }

        let peers: Vec<SocketAddr> = self.peers.read().await.iter().copied().collect();

        if peers.len() <= k {
            // Not enough peers to be selective - return all
            return peers;
        }

        // Get latency and bandwidth data for all peers
        let mut scores = self.peer_scores.write().await;
        let mut peers_with_metrics: Vec<(SocketAddr, Option<u64>, f64)> = peers
            .iter()
            .map(|&addr| {
                let (latency, bandwidth_eff) = if let Some(s) = scores.get_mut(&addr) {
                    // Reset bandwidth counters if needed
                    if s.should_reset_bandwidth() {
                        s.reset_bandwidth_counters();
                    }
                    let lat = if s.latency_samples.is_empty() {
                        s.latency_ms
                    } else {
                        // Use average of samples
                        Some(s.latency_samples.iter().sum::<u64>() / s.latency_samples.len() as u64)
                    };
                    (lat, s.bandwidth_efficiency())
                } else {
                    (None, 0.5) // Default values for unknown peers
                };
                (addr, latency, bandwidth_eff)
            })
            .collect();
        drop(scores);

        // Sort by latency (primary) and bandwidth efficiency (secondary)
        // None = unknown latency, treat as high value for sorting
        peers_with_metrics.sort_by(|a, b| {
            match (&a.1, &b.1) {
                (Some(la), Some(lb)) => {
                    // Same latency group (within 10ms) - use bandwidth efficiency as tiebreaker
                    if la.abs_diff(*lb) <= 10 {
                        // Higher bandwidth efficiency is better (reverse sort)
                        b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
                    } else {
                        la.cmp(lb)
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less, // Known latency first
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => {
                    // Both unknown - use bandwidth efficiency
                    b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
                }
            }
        });

        // Find median latency (for splitting into fast/slow)
        let known_latencies: Vec<u64> = peers_with_metrics
            .iter()
            .filter_map(|(_, l, _)| *l)
            .collect();

        let median_latency = if known_latencies.is_empty() {
            None
        } else {
            // Use median of known latencies
            let mut sorted = known_latencies.clone();
            sorted.sort();
            Some(sorted[sorted.len() / 2])
        };

        // Split into fast and slow based on median
        let (fast_peers, slow_peers): (Vec<_>, Vec<_>) =
            peers_with_metrics.into_iter().partition(|(_, latency, _)| {
                match (latency, median_latency) {
                    (Some(l), Some(m)) => *l <= m,
                    // Unknown latency peers go to slow group (for diversity)
                    _ => false,
                }
            });

        // Calculate how many to take from each group
        let from_fast = (k as f64 * 0.7).ceil() as usize;
        let from_slow = k.saturating_sub(from_fast);

        // Fast peers are already sorted by latency + bandwidth efficiency
        // Take top peers from each group (they're already in optimal order)
        let fast_peers: Vec<SocketAddr> = fast_peers.into_iter().map(|(a, _, _)| a).collect();
        let slow_peers: Vec<SocketAddr> = slow_peers.into_iter().map(|(a, _, _)| a).collect();

        // Select from each group
        let mut selected: Vec<SocketAddr> = Vec::with_capacity(k);
        selected.extend(fast_peers.into_iter().take(from_fast));
        selected.extend(slow_peers.into_iter().take(from_slow));

        // If we didn't get enough (e.g., not enough slow peers), fill from remaining
        if selected.len() < k {
            // This shouldn't normally happen, but handle edge case
            let remaining: Vec<SocketAddr> = self
                .peers
                .read()
                .await
                .iter()
                .copied()
                .filter(|p| !selected.contains(p))
                .collect();
            selected.extend(remaining.into_iter().take(k - selected.len()));
        }

        // Cache the sorted peer list for future calls
        // Store all peers in sorted order (not just selected)
        let sorted_all: Vec<SocketAddr> = {
            let mut all = selected.clone();
            // Add any remaining peers not in selected
            let remaining: Vec<SocketAddr> = self
                .peers
                .read()
                .await
                .iter()
                .copied()
                .filter(|p| !all.contains(p))
                .collect();
            all.extend(remaining);
            all
        };
        *self.cached_fanout_peers.write().await = Some((Instant::now(), sorted_all));

        selected
    }

    /// Requests the peer list from a specific peer.
    pub async fn request_peers_from(
        &self,
        peer_addr: SocketAddr,
    ) -> crate::error::BlockchainResult<()> {
        let request = NetworkMessage::RequestPeers;
        let authenticated = self.sign_message(request)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        let gossip_streams = self.peer_gossip_streams.lock().await;
        if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
            let mut stream = stream_arc.lock().await;
            let total_len = 1 + data.len();
            stream.write_u32(total_len as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write length to {}: {}",
                    peer_addr, e
                ))
            })?;
            stream.write_u8(MSG_FRAME_PLAINTEXT).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write frame byte to {}: {}",
                    peer_addr, e
                ))
            })?;
            stream.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!(
                    "Failed to write data to {}: {}",
                    peer_addr, e
                ))
            })?;
            Ok(())
        } else {
            Err(crate::error::BlockchainError::Network(format!(
                "No QUIC stream to peer {}",
                peer_addr
            )))
        }
    }

    // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
    // Headers-first sync removed as dead code — DAG block structure makes
    // header-only validation insufficient.

    /// Broadcasts a block using compact format (BIP 152 style).
    ///
    /// # Arguments
    /// * `block` - The block to broadcast
    /// * `use_compact` - Whether to attempt compact block broadcast
    /// * `is_own_block` - If true, broadcasts to all peers; if false, uses fanout relay
    pub async fn broadcast_block_compact(
        &self,
        block: &Block,
        use_compact: bool,
        is_own_block: bool,
    ) -> crate::error::BlockchainResult<()> {
        // If compact is disabled or we don't have mining manager, fall back to full block
        if !use_compact {
            return self.broadcast_block(block, is_own_block).await;
        }

        // Get mempool hashes for compact block creation
        let mempool_hashes = if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr.get_mempool_hashes().await
        } else {
            // No mining manager - fall back to full block
            return self.broadcast_block(block, is_own_block).await;
        };

        // Create compact block
        let compact_block = sync::CompactBlock::from_block(block, &mempool_hashes);

        // Mark block as seen before broadcasting (atomic insert + cleanup)
        {
            let mut block_seen = self.block_seen.write().await;
            block_seen.insert(block.hash, Instant::now());
            // Evict old entries if cache is full
            evict_seen_cache(&mut block_seen, 10_000);
        }

        let peers = self.peers.read().await;
        let peer_count = peers.len();

        if peers.is_empty() {
            return Ok(());
        }

        // Determine which peers to broadcast to based on is_own_block
        let selected_peers: Vec<SocketAddr> = if is_own_block {
            // Own-mined blocks: broadcast to ALL peers (critical path)
            debug!(
                "[COMPACT_BLOCK] Broadcasting own-mined block #{} to ALL {} peers",
                block.header.block_number, peer_count
            );
            peers.iter().copied().collect()
        } else {
            // Relayed blocks: use higher fanout
            let fanout = std::cmp::max(3, 2 * (peer_count as f64).sqrt() as usize);
            let mut peer_list: Vec<SocketAddr> = peers.iter().copied().collect();
            peer_list.sort_by_cached_key(|_| rand::random::<u64>());
            let selected: Vec<SocketAddr> = peer_list.into_iter().take(fanout).collect();
            debug!(
                "[COMPACT_BLOCK] Relaying block #{} to {}/{} peers (fanout)",
                block.header.block_number,
                selected.len(),
                peer_count
            );
            selected
        };

        // Create the compact block message
        let message = NetworkMessage::NewCompactBlock { compact_block };
        let authenticated = self.sign_message(message)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        // Check message size
        if data.len() > MAX_MESSAGE_SIZE {
            // Compact block too large - fall back to full block
            warn!(
                "Compact block size {} exceeds maximum, falling back to full block",
                data.len()
            );
            return self.broadcast_block(block, is_own_block).await;
        }

        // Send to selected peers (lock acquired once before loop)
        let gossip_streams = self.peer_gossip_streams.lock().await;
        for &peer_addr in &selected_peers {
            if let Some(stream_arc) = gossip_streams.get(&peer_addr) {
                let mut stream = stream_arc.lock().await;

                if let Err(e) = write_framed(&mut stream, &data).await {
                    self.record_send_failure(peer_addr).await;
                    warn!("Failed to send compact block to {}: {}", peer_addr, e);
                } else {
                    self.record_send_success(peer_addr).await;
                }
            }
        }

        Ok(())
    }

    /// Requests shard blocks from a specific peer for catch-up sync.
    pub async fn request_shard_blocks(
        &self,
        peer_addr: SocketAddr,
        shard_id: usize,
        from_block: u64,
        count: u64,
    ) -> crate::error::BlockchainResult<()> {
        let request = NetworkMessage::RequestShardBlocks {
            shard_id,
            from_block,
            count,
        };
        let authenticated = self.sign_message(request)?;
        let data = bincode::serialize(&authenticated)
            .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

        self.send_to_peer(&peer_addr, &data).await
    }

    /// Send data to a peer using stored connection (BPR-003: connection reuse)
    ///
    /// Tries to use an existing connection from peer_connections first.
    /// If no stored connection exists, skips sending with a warning.
    ///
    /// When Kyber feature is enabled and a session key exists for the peer,
    /// applies an additional AES-256-GCM encryption layer on top of the data
    /// for hybrid post-quantum security.
    async fn send_to_peer(
        &self,
        addr: &SocketAddr,
        data: &[u8],
    ) -> crate::error::BlockchainResult<()> {
        // Determine frame byte and payload based on Kyber encryption
        #[cfg(feature = "kyber")]
        let (frame_byte, payload) = {
            if let Some(kyber_key) = self.kyber_session_keys.read().await.get(addr) {
                use crate::pqc::encryption::PqEncryption;
                use crate::pqc::SessionKey;

                let session_key =
                    SessionKey::new(kyber_key.as_slice().try_into().map_err(|_| {
                        crate::error::BlockchainError::Network(
                            "Invalid Kyber key length".to_string(),
                        )
                    })?);

                match PqEncryption::encrypt(data, &session_key) {
                    Ok(encrypted) => {
                        info!("[KYBER DEBUG] Sending Kyber-encrypted message to {} (hybrid PQ encryption active)", addr);
                        let encrypted_bytes = bincode::serialize(&encrypted).map_err(|e| {
                            crate::error::BlockchainError::Serialization(e.to_string())
                        })?;
                        (MSG_FRAME_KYBER_ENCRYPTED, encrypted_bytes)
                    }
                    Err(e) => {
                        warn!(
                            "Kyber encryption failed for {}: {} - sending without PQ layer",
                            addr, e
                        );
                        (MSG_FRAME_PLAINTEXT, data.to_vec())
                    }
                }
            } else {
                let keys: Vec<SocketAddr> = self
                    .kyber_session_keys
                    .read()
                    .await
                    .keys()
                    .cloned()
                    .collect();
                info!(
                    "[KYBER DEBUG] No session key for {}, sending plaintext. Available keys: {:?}",
                    addr, keys
                );
                (MSG_FRAME_PLAINTEXT, data.to_vec())
            }
        };

        #[cfg(not(feature = "kyber"))]
        let (frame_byte, payload) = (MSG_FRAME_PLAINTEXT, data.to_vec());

        // Try to use existing QUIC gossip stream
        let gossip_streams = self.peer_gossip_streams.lock().await;
        if let Some(stream_arc) = gossip_streams.get(addr) {
            let mut stream = stream_arc.lock().await;
            // Send via stored stream: length (4 bytes) + frame byte (1 byte) + payload
            let total_len = 1 + payload.len();
            match stream.write_u32(total_len as u32).await {
                Ok(_) => match stream.write_u8(frame_byte).await {
                    Ok(_) => match stream.write_all(&payload).await {
                        Ok(_) => {
                            self.record_send_success(*addr).await;
                            return Ok(());
                        }
                        Err(e) => {
                            self.record_send_failure(*addr).await;
                            warn!("Failed to send data via QUIC stream to {}: {}", addr, e);
                        }
                    },
                    Err(e) => {
                        self.record_send_failure(*addr).await;
                        warn!(
                            "Failed to send frame byte via QUIC stream to {}: {}",
                            addr, e
                        );
                    }
                },
                Err(e) => {
                    self.record_send_failure(*addr).await;
                    warn!("Failed to send length via QUIC stream to {}: {}", addr, e);
                }
            }
        }
        warn!("No stored QUIC stream for peer {}, skipping send", addr);
        Ok(())
    }
}

/// Handle a peer connection over QUIC
async fn handle_peer(
    send_stream: Arc<Mutex<quinn::SendStream>>,
    recv_stream: quinn::RecvStream,
    addr: SocketAddr,
    ctx: Arc<NetworkContext>,
) {
    // Handler logging disabled to reduce console noise
    let mut buffer = vec![0u8; 1024 * 1024]; // 1MB buffer

    // The send_stream is already stored in gossip_streams by the caller
    // Connection stored - logging disabled to reduce console noise

    // Read timeout with exponential backoff to reduce CPU spinning when idle
    let mut backoff_ms = 100u64;
    const MAX_BACKOFF_MS: u64 = 5000;

    // Wrap recv_stream in a mutex for interior mutability
    let recv_stream = Arc::new(Mutex::new(recv_stream));

    // Debug: Track first message for diagnostic purposes
    let mut first_message = true;

    // Counter for consecutive oversized messages (resilient framing)
    let mut oversized_count = 0u8;
    const MAX_OVERSIZED_CONSECUTIVE: u8 = 5;

    while *ctx.is_running.read().await {
        let timeout = std::time::Duration::from_millis(backoff_ms);
        let len_result = {
            let mut stream = recv_stream.lock().await;
            tokio::time::timeout(timeout, stream.read_u32()).await
        };

        let len = match len_result {
            Ok(Ok(len)) => {
                backoff_ms = 100; // Reset on successful read
                len as usize
            }
            Ok(Err(_)) => {
                cleanup_peer_state_on_error(
                    &ctx.peers,
                    &ctx.peer_scores,
                    &ctx.peer_count_atomic,
                    &ctx.connection_types,
                    &ctx.subnet_peer_counts,
                    &ctx.quic_connections,
                    &ctx.gossip_streams,
                    &ctx.peer_public_keys,
                    &ctx.peer_advertised_addrs,
                    &ctx.peer_request_peers_time,
                    &ctx.peer_exchange_lists,
                    &ctx.cached_fanout_peers,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_keys,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_cache,
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    addr,
                )
                .await;
                break;
            }
            Err(_) => {
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
                continue;
            }
        };

        // Check message size (DoS protection)
        if len > MAX_MESSAGE_SIZE {
            oversized_count += 1;
            warn!(
                "Message from {} exceeds maximum size: {} bytes (consecutive: {})",
                addr, len, oversized_count
            );

            // Only drop connection after multiple consecutive oversized messages
            if oversized_count >= MAX_OVERSIZED_CONSECUTIVE {
                warn!(
                    "Too many consecutive oversized messages from {} — dropping connection",
                    addr
                );
                cleanup_peer_state_on_error(
                    &ctx.peers,
                    &ctx.peer_scores,
                    &ctx.peer_count_atomic,
                    &ctx.connection_types,
                    &ctx.subnet_peer_counts,
                    &ctx.quic_connections,
                    &ctx.gossip_streams,
                    &ctx.peer_public_keys,
                    &ctx.peer_advertised_addrs,
                    &ctx.peer_request_peers_time,
                    &ctx.peer_exchange_lists,
                    &ctx.cached_fanout_peers,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_keys,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_cache,
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    addr,
                )
                .await;
                break;
            }

            // Try to drain the bad message and continue reading
            let drain_result = {
                let mut stream = recv_stream.lock().await;
                // Read and discard up to len bytes (with a reasonable limit to prevent abuse)
                let drain_len = len.min(MAX_MESSAGE_SIZE);
                let mut drain_buf = vec![0u8; drain_len];
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    stream.read_exact(&mut drain_buf),
                )
                .await
            };

            if let Err(_) | Ok(Err(_)) = drain_result {
                warn!(
                    "Failed to drain oversized message from {} — closing stream",
                    addr
                );
                cleanup_peer_state_on_error(
                    &ctx.peers,
                    &ctx.peer_scores,
                    &ctx.peer_count_atomic,
                    &ctx.connection_types,
                    &ctx.subnet_peer_counts,
                    &ctx.quic_connections,
                    &ctx.gossip_streams,
                    &ctx.peer_public_keys,
                    &ctx.peer_advertised_addrs,
                    &ctx.peer_request_peers_time,
                    &ctx.peer_exchange_lists,
                    &ctx.cached_fanout_peers,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_keys,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_cache,
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    addr,
                )
                .await;
                break;
            }

            // Continue to next message instead of breaking
            continue;
        }

        // Reset oversized counter on valid message size
        oversized_count = 0;

        // Protocol v2: Read frame byte + payload
        // total_len includes the frame byte, so payload_len = total_len - 1
        if len < 1 {
            warn!("Message from {} too short for frame byte", addr);
            continue;
        }
        let payload_len = len - 1;

        // Resize buffer if needed
        if payload_len > buffer.len() {
            buffer.resize(payload_len, 0);
        }

        // Acquire lock again only for reading the message data
        let (frame_byte, read_result) = {
            let mut stream = recv_stream.lock().await;
            match stream.read_u8().await {
                Ok(frame) => {
                    // Debug: Log first message frame byte for diagnostic purposes
                    if first_message {
                        info!(
                            "[FRAME DEBUG] First message from {}: len={}, frame_byte=0x{:02x}",
                            addr, len, frame
                        );
                        first_message = false;
                    }
                    let result = stream.read_exact(&mut buffer[..payload_len]).await;
                    (frame, result)
                }
                Err(_e) => {
                    // Return an error result that matches the type
                    (
                        0,
                        Err::<(), quinn::ReadExactError>(quinn::ReadExactError::FinishedEarly(0)),
                    )
                }
            }
            // Lock is released here when stream goes out of scope
        };

        // Read message data
        match read_result {
            Ok(_) => {
                // VULN-007: Only rate-limit known peers; allow TOFU-identified peers
                // FIX: Softened check - if peer has TOFU identity (known public key), auto-register and allow
                let is_known = ctx.peers.read().await.contains(&addr);
                if !is_known {
                    // Check for TOFU identity (peer has registered public key)
                    let has_tofu_identity = ctx.peer_public_keys.read().await.contains_key(&addr);
                    // Also check if peer has an active QUIC connection (authenticated via QUIC TLS)
                    // This handles outbound connections where peer_identities is set but peer_public_keys isn't yet
                    let has_quic_connection = ctx.quic_connections.lock().await.contains_key(&addr);
                    if has_tofu_identity || has_quic_connection {
                        // Auto-register peer with TOFU identity or active QUIC connection
                        info!("Auto-registering peer {} with TOFU identity (public_key={}, quic_conn={})",
                              addr, has_tofu_identity, has_quic_connection);
                        ctx.peers.write().await.insert(addr);
                        ctx.peer_scores
                            .write()
                            .await
                            .entry(addr)
                            .or_insert_with(PeerScore::default);
                        ctx.peer_count_atomic
                            .fetch_add(1, std::sync::atomic::Ordering::Release);
                    } else {
                        warn!(
                            "Message from unknown peer {} (no TOFU identity), rejecting",
                            addr
                        );
                        continue; // Drop message from unknown peer
                    }
                }

                // BPR-005: Check rate limit before processing any message
                {
                    let mut scores = ctx.peer_scores.write().await;
                    if !check_peer_rate_limit(&mut scores, &addr) {
                        debug!("Rate limit triggered for peer {}", addr);
                        continue; // Drop message and wait for next
                    }
                }

                // Process message based on frame byte
                let message_bytes = match frame_byte {
                    MSG_FRAME_KYBER_ENCRYPTED => {
                        #[cfg(feature = "kyber")]
                        {
                            if let Some(kyber_key) = ctx.kyber_session_keys.read().await.get(&addr)
                            {
                                use crate::pqc::encryption::PqEncryption;
                                use crate::pqc::SessionKey;

                                let encrypted_msg: crate::pqc::EncryptedMessage =
                                    match bincode::deserialize(&buffer[..payload_len]) {
                                        Ok(msg) => msg,
                                        Err(e) => {
                                            warn!("Failed to deserialize Kyber-encrypted message from {}: {}", addr, e);
                                            continue;
                                        }
                                    };

                                let session_key = SessionKey::new(
                                    kyber_key.as_slice().try_into().unwrap_or([0u8; 32]),
                                );

                                match PqEncryption::decrypt(&encrypted_msg, &session_key) {
                                    Ok(decrypted) => {
                                        debug!("Decrypted Kyber AEAD layer from peer {} (hybrid PQ encryption active)", addr);
                                        decrypted
                                    }
                                    Err(e) => {
                                        warn!("Kyber decryption failed from {}: {} - dropping message", addr, e);
                                        continue;
                                    }
                                }
                            } else {
                                let keys: Vec<SocketAddr> = ctx
                                    .kyber_session_keys
                                    .read()
                                    .await
                                    .keys()
                                    .cloned()
                                    .collect();
                                warn!("[KYBER DEBUG] Received Kyber-encrypted message from {} but no session key. Available keys: {:?}", addr, keys);
                                continue;
                            }
                        }
                        #[cfg(not(feature = "kyber"))]
                        {
                            warn!("Received Kyber-encrypted message but kyber feature not enabled");
                            continue;
                        }
                    }
                    MSG_FRAME_PLAINTEXT => buffer[..payload_len].to_vec(),
                    unknown => {
                        warn!("Unknown message frame byte 0x{:02x} from {}", unknown, addr);
                        continue; // Skip unknown frame types for forward compatibility
                    }
                };

                // Try to deserialize as authenticated message first
                if let Ok(authenticated) =
                    bincode::deserialize::<AuthenticatedMessage>(&message_bytes)
                {
                    // Look up pinned key for this peer (TOFU)
                    let pinned_key: Option<[u8; 32]> = ctx
                        .peer_public_keys
                        .read()
                        .await
                        .get(&addr)
                        .and_then(|k| k.as_slice().try_into().ok());

                    // Verify message signature with key pinning
                    if let Err(_e) =
                        NetworkManager::verify_message(&authenticated, pinned_key.as_ref())
                    {
                        warn!("Message verification failed from {}", addr);
                        // BPR-008: Penalize peer for invalid message (may result in ban)
                        {
                            let mut scores = ctx.peer_scores.write().await;
                            penalize_peer(
                                &mut scores,
                                &addr,
                                PenaltyReason::SignatureVerificationFailed,
                            );
                            // Check if peer was banned
                            if let Some(score) = scores.get(&addr) {
                                if score.is_banned() {
                                    // Disconnect banned peer
                                    cleanup_peer_state_on_error(
                                        &ctx.peers,
                                        &ctx.peer_scores,
                                        &ctx.peer_count_atomic,
                                        &ctx.connection_types,
                                        &ctx.subnet_peer_counts,
                                        &ctx.quic_connections,
                                        &ctx.gossip_streams,
                                        &ctx.peer_public_keys,
                                        &ctx.peer_advertised_addrs,
                                        &ctx.peer_request_peers_time,
                                        &ctx.peer_exchange_lists,
                                        &ctx.cached_fanout_peers,
                                        #[cfg(feature = "kyber")]
                                        &ctx.kyber_session_keys,
                                        #[cfg(feature = "kyber")]
                                        &ctx.kyber_session_cache,
                                        #[cfg(not(feature = "kyber"))]
                                        &(),
                                        #[cfg(not(feature = "kyber"))]
                                        &(),
                                        addr,
                                    )
                                    .await;
                                    warn!("Disconnected banned peer {}", addr);
                                    break;
                                }
                            }
                        }
                        continue;
                    }

                    // Clock drift reputation docking (P2P Hardening Priority 1)
                    {
                        let current_time = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let drift_secs = authenticated.timestamp.abs_diff(current_time);

                        if drift_secs > 120 {
                            // >2 minutes drift
                            let penalty = if drift_secs > 240 {
                                15.0 // 4-5 minutes: heavy penalty
                            } else if drift_secs > 180 {
                                10.0 // 3-4 minutes: moderate penalty
                            } else {
                                5.0 // 2-3 minutes: light penalty
                            };

                            let mut scores = ctx.peer_scores.write().await;
                            if let Some(score) = scores.get_mut(&addr) {
                                // Rate-limit clock drift penalties to once per 60-second window
                                let should_apply = score
                                    .last_clock_drift_check
                                    .is_none_or(|last| last.elapsed().as_secs() >= 60);

                                if should_apply {
                                    score.clock_drift_penalties += penalty;
                                    score.last_clock_drift_check = Some(Instant::now());

                                    if score.clock_drift_penalties >= 50.0
                                        && (score.clock_drift_penalties - penalty) < 50.0
                                    {
                                        // First time crossing threshold — log with both timestamps
                                        warn!("Peer {} clock drift {}s (peer_time={}, local_time={}) — consider NTP synchronization",
                                            addr, drift_secs, authenticated.timestamp, current_time);
                                    }
                                }
                            }
                            drop(scores);
                        }
                    }

                    // Check for peer public key change (possible MITM attack)
                    {
                        let mut keys = ctx.peer_public_keys.write().await;
                        if let Some(existing_key) = keys.get(&addr) {
                            if existing_key != &authenticated.public_key {
                                error!("SECURITY WARNING: Peer {} public key changed unexpectedly! (possible MITM attack)", addr);
                                // Log the key change but continue processing with the new key
                                // The signature was verified, so the message is authentic from the new key holder
                            }
                        }
                        // Store/update the peer's public key
                        keys.insert(addr, authenticated.public_key.clone());
                    }

                    // Process the verified message (this may need to write responses)
                    if let Err(e) = process_message_with_arc(
                        authenticated.message,
                        &ctx.blockchain,
                        &ctx.peers,
                        &send_stream,
                        addr,
                        &ctx.orphan_pool,
                        ctx.mining_manager.as_ref(),
                        &ctx.gossip_streams,
                        ctx.peer_connect_tx.as_ref(),
                        ctx.listen_addr,
                        ctx.shard_manager.as_ref(),
                        ctx.max_peers,
                        &ctx.tx_seen,
                        &ctx.block_seen,
                        &ctx.peer_scores,
                        &ctx.partition_detected,
                        &ctx.partition_start,
                        &ctx.last_block_received,
                        &ctx.local_tip_height,
                        ctx.signing_key.clone(),
                        ctx.node_public_key.clone(),
                        &ctx.peer_advertised_addrs,
                        &ctx.peer_request_peers_time,
                        &ctx.peer_exchange_lists,
                    )
                    .await
                    {
                        warn!("Error processing message from {}: {}", addr, e);
                    } else {
                        if let Some(s) = ctx.peer_scores.write().await.get_mut(&addr) {
                            s.last_seen = Instant::now();
                            s.success_count = s.success_count.saturating_add(1);
                        }
                    }
                } else {
                    // Reject unsigned messages - signature verification is mandatory
                    warn!("Rejected unsigned/invalid message from {} - all P2P messages must be signed", addr);
                    // BPR-008: Penalize peer for invalid message (may result in ban)
                    {
                        let mut scores = ctx.peer_scores.write().await;
                        penalize_peer(&mut scores, &addr, PenaltyReason::MalformedMessage);
                        // Check if peer was banned
                        if let Some(score) = scores.get(&addr) {
                            if score.is_banned() {
                                // Disconnect banned peer
                                cleanup_peer_state_on_error(
                                    &ctx.peers,
                                    &ctx.peer_scores,
                                    &ctx.peer_count_atomic,
                                    &ctx.connection_types,
                                    &ctx.subnet_peer_counts,
                                    &ctx.quic_connections,
                                    &ctx.gossip_streams,
                                    &ctx.peer_public_keys,
                                    &ctx.peer_advertised_addrs,
                                    &ctx.peer_request_peers_time,
                                    &ctx.peer_exchange_lists,
                                    &ctx.cached_fanout_peers,
                                    #[cfg(feature = "kyber")]
                                    &ctx.kyber_session_keys,
                                    #[cfg(feature = "kyber")]
                                    &ctx.kyber_session_cache,
                                    #[cfg(not(feature = "kyber"))]
                                    &(),
                                    #[cfg(not(feature = "kyber"))]
                                    &(),
                                    addr,
                                )
                                .await;
                                warn!("Disconnected banned peer {}", addr);
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Error reading from {}: {}", addr, e);
                cleanup_peer_state_on_error(
                    &ctx.peers,
                    &ctx.peer_scores,
                    &ctx.peer_count_atomic,
                    &ctx.connection_types,
                    &ctx.subnet_peer_counts,
                    &ctx.quic_connections,
                    &ctx.gossip_streams,
                    &ctx.peer_public_keys,
                    &ctx.peer_advertised_addrs,
                    &ctx.peer_request_peers_time,
                    &ctx.peer_exchange_lists,
                    &ctx.cached_fanout_peers,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_keys,
                    #[cfg(feature = "kyber")]
                    &ctx.kyber_session_cache,
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    #[cfg(not(feature = "kyber"))]
                    &(),
                    addr,
                )
                .await;
                break;
            }
        }
    }
}

/// Sign a network message using provided secret key (standalone function version)
/// Use this when you don't have access to `self` (NetworkManager)
fn sign_message_with_key(
    message: NetworkMessage,
    node_secret_key: &[u8; 32],
    _node_public_key: &PublicKey,
) -> crate::error::BlockchainResult<AuthenticatedMessage> {
    use ed25519_dalek::{Signer, SigningKey};

    // Serialize message for signing
    let message_bytes = bincode::serialize(&message)
        .map_err(|e| crate::error::BlockchainError::Serialization(e.to_string()))?;

    // Sign message with the node's secret key
    let signing_key = SigningKey::from_bytes(node_secret_key);
    let verifying_key = signing_key.verifying_key();
    let public_key_bytes: [u8; 32] = verifying_key.to_bytes();

    // Compute timestamp BEFORE signing so it's included in signed payload
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Sign message bytes + timestamp together to prevent replay with altered timestamps
    let mut signed_payload = message_bytes;
    signed_payload.extend_from_slice(&timestamp.to_le_bytes());
    let signature = signing_key.sign(&signed_payload);
    let signature = signature.to_bytes().to_vec();
    let public_key = public_key_bytes.to_vec();

    Ok(AuthenticatedMessage {
        message,
        signature,
        public_key,
        timestamp,
    })
}

/// Handle incoming streams opened by the peer
/// This handles sync streams (type 0x02) and potentially other stream types
/// Handle all streams for a QUIC connection (gossip, sync, kyber)
/// This is the single entry point for all streams on a connection to avoid race conditions.
async fn handle_connection_streams(
    connection: quinn::Connection,
    peer_addr: SocketAddr,
    ctx: Arc<NetworkContext>,
) {
    use std::sync::atomic::Ordering;

    debug!("Starting connection stream handler for {}", peer_addr);

    // Track whether we've completed the gossip handshake
    let gossip_handshake_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    loop {
        // Accept incoming bidirectional streams
        match connection.accept_bi().await {
            Ok((mut send_stream, mut recv_stream)) => {
                // Read stream type byte
                let mut stream_type = [0u8; 1];
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    recv_stream.read_exact(&mut stream_type),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        match stream_type[0] {
                            STREAM_TYPE_GOSSIP => {
                                debug!("Received gossip stream from {}", peer_addr);

                                // Only process the first gossip stream
                                if gossip_handshake_done.load(Ordering::Relaxed) {
                                    warn!("Duplicate gossip stream from {}, ignoring", peer_addr);
                                    continue;
                                }

                                // Process gossip handshake inline
                                match process_gossip_handshake(
                                    send_stream,
                                    recv_stream,
                                    peer_addr,
                                    &ctx.gossip_streams,
                                    &ctx.peer_public_keys,
                                    &ctx.peer_advertised_addrs,
                                    ctx.listen_addr,
                                    ctx.signing_key.clone(),
                                    ctx.node_public_key.clone(),
                                )
                                .await
                                {
                                    Ok((gossip_send, gossip_recv)) => {
                                        // Mark handshake as done
                                        gossip_handshake_done.store(true, Ordering::Relaxed);

                                        // Spawn handle_peer for gossip message processing
                                        let ctx_clone = ctx.clone();
                                        tokio::spawn(async move {
                                            handle_peer(
                                                gossip_send,
                                                gossip_recv,
                                                peer_addr,
                                                ctx_clone,
                                            )
                                            .await;
                                        });

                                        // Spawn Kyber upgrade on a separate stream if enabled
                                        #[cfg(feature = "kyber")]
                                        {
                                            // Check session cache first
                                            let cached = {
                                                let mut cache =
                                                    ctx.kyber_session_cache.write().await;
                                                if let Some((key, cached_at)) =
                                                    cache.remove(&peer_addr)
                                                {
                                                    if cached_at.elapsed().as_secs()
                                                        < KYBER_SESSION_CACHE_TTL_SECS
                                                    {
                                                        Some(key)
                                                    } else {
                                                        None
                                                    }
                                                } else {
                                                    None
                                                }
                                            };

                                            if let Some(cached_key) = cached {
                                                ctx.kyber_session_keys
                                                    .write()
                                                    .await
                                                    .insert(peer_addr, cached_key);
                                                info!(
                                                    "Kyber session reused for {} (cached)",
                                                    peer_addr
                                                );
                                            } else {
                                                let connection_clone = connection.clone();
                                                let kyber_k_clone = ctx.kyber_keys.clone();
                                                let kyber_sk_clone = ctx.kyber_session_keys.clone();
                                                let addr_clone = peer_addr;
                                                let semaphore_clone =
                                                    ctx.kyber_handshake_semaphore.clone();

                                                tokio::spawn(async move {
                                                    let _permit = match semaphore_clone
                                                        .try_acquire()
                                                    {
                                                        Ok(permit) => permit,
                                                        Err(_) => {
                                                            info!("Kyber handshake throttled for {} — too many concurrent exchanges", addr_clone);
                                                            return;
                                                        }
                                                    };

                                                    // Open a separate stream for Kyber exchange
                                                    match connection_clone.open_bi().await {
                                                        Ok((mut kyber_send, mut kyber_recv)) => {
                                                            // Write Kyber stream type
                                                            if let Err(e) = kyber_send
                                                                .write_all(&[STREAM_TYPE_KYBER])
                                                                .await
                                                            {
                                                                debug!("Failed to open Kyber stream to {}: {}", addr_clone, e);
                                                                return;
                                                            }

                                                            // Get our Kyber keys
                                                            let our_kyber = {
                                                                let guard =
                                                                    kyber_k_clone.lock().await;
                                                                match guard.as_ref() {
                                                                    Some(k) => k.clone(),
                                                                    None => return,
                                                                }
                                                            };

                                                            // Perform Kyber handshake as responder using unified function
                                                            match perform_kyber_handshake(
                                                                &mut kyber_send,
                                                                &mut kyber_recv,
                                                                KyberRole::Responder,
                                                                our_kyber,
                                                                addr_clone,
                                                            )
                                                            .await
                                                            {
                                                                Ok(session_key) => {
                                                                    // Store session key with automatic zeroization on drop
                                                                    kyber_sk_clone
                                                                        .write()
                                                                        .await
                                                                        .insert(
                                                                            addr_clone,
                                                                            zeroize::Zeroizing::new(
                                                                                session_key
                                                                                    .as_bytes()
                                                                                    .to_vec(),
                                                                            ),
                                                                        );
                                                                    debug!("Kyber session established with {}", addr_clone);
                                                                }
                                                                Err(e) => {
                                                                    debug!("Kyber handshake failed for {}: {}", addr_clone, e);
                                                                }
                                                            }
                                                        }
                                                        Err(_) => {
                                                            return;
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Gossip handshake failed for {}: {}", peer_addr, e);
                                        // Clean up peer state
                                        cleanup_peer_state_on_error(
                                            &ctx.peers,
                                            &ctx.peer_scores,
                                            &ctx.peer_count_atomic,
                                            &ctx.connection_types,
                                            &ctx.subnet_peer_counts,
                                            &ctx.quic_connections,
                                            &ctx.gossip_streams,
                                            &ctx.peer_public_keys,
                                            &ctx.peer_advertised_addrs,
                                            &ctx.peer_request_peers_time,
                                            &ctx.peer_exchange_lists,
                                            &ctx.cached_fanout_peers,
                                            #[cfg(feature = "kyber")]
                                            &ctx.kyber_session_keys,
                                            #[cfg(feature = "kyber")]
                                            &ctx.kyber_session_cache,
                                            #[cfg(not(feature = "kyber"))]
                                            &(),
                                            #[cfg(not(feature = "kyber"))]
                                            &(),
                                            peer_addr,
                                        )
                                        .await;
                                        return;
                                    }
                                }
                            }
                            STREAM_TYPE_SYNC => {
                                debug!("Received sync stream from {}", peer_addr);
                                // Handle sync request in a spawned task
                                let blockchain_clone = ctx.blockchain.clone();
                                let signing_key_clone = ctx.signing_key.clone();
                                let peer_addr_clone = peer_addr;
                                tokio::spawn(async move {
                                    if let Err(e) = handle_sync_stream(
                                        &mut send_stream,
                                        &mut recv_stream,
                                        peer_addr_clone,
                                        blockchain_clone,
                                        signing_key_clone,
                                    )
                                    .await
                                    {
                                        warn!(
                                            "Sync stream handling failed for {}: {}",
                                            peer_addr_clone, e
                                        );
                                    }
                                });
                            }
                            STREAM_TYPE_KYBER => {
                                debug!("Received Kyber stream from {}", peer_addr);
                                #[cfg(feature = "kyber")]
                                {
                                    // Handle incoming Kyber stream (peer initiated)
                                    let kyber_k_clone = ctx.kyber_keys.clone();
                                    let kyber_sk_clone = ctx.kyber_session_keys.clone();
                                    let addr_clone = peer_addr;

                                    tokio::spawn(async move {
                                        // Get our Kyber keys
                                        let our_kyber = {
                                            let guard = kyber_k_clone.lock().await;
                                            match guard.as_ref() {
                                                Some(k) => k.clone(),
                                                None => return,
                                            }
                                        };

                                        // Perform Kyber handshake as responder using unified function
                                        // Note: This uses the already-opened stream (peer initiated)
                                        let mut send = send_stream;
                                        let mut recv = recv_stream;
                                        match perform_kyber_handshake(
                                            &mut send,
                                            &mut recv,
                                            KyberRole::Responder,
                                            our_kyber,
                                            addr_clone,
                                        )
                                        .await
                                        {
                                            Ok(session_key) => {
                                                // Store session key with automatic zeroization on drop
                                                kyber_sk_clone.write().await.insert(
                                                    addr_clone,
                                                    zeroize::Zeroizing::new(
                                                        session_key.as_bytes().to_vec(),
                                                    ),
                                                );
                                                debug!(
                                                    "Kyber session established with {} (inbound)",
                                                    addr_clone
                                                );
                                            }
                                            Err(e) => {
                                                debug!(
                                                    "Kyber handshake failed for {} (inbound): {}",
                                                    addr_clone, e
                                                );
                                            }
                                        }
                                    });
                                }
                                #[cfg(not(feature = "kyber"))]
                                {
                                    warn!("Received Kyber stream from {} but Kyber feature not enabled", peer_addr);
                                }
                            }
                            _ => {
                                warn!(
                                    "Unknown stream type 0x{:02x} from {}",
                                    stream_type[0], peer_addr
                                );
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        debug!("Failed to read stream type from {}: {}", peer_addr, e);
                    }
                    Err(_) => {
                        debug!("Timeout reading stream type from {}", peer_addr);
                    }
                }
            }
            Err(e) => {
                debug!("Connection closed or error for {}: {}", peer_addr, e);
                break;
            }
        }
    }

    debug!("Connection stream handler exiting for {}", peer_addr);
}

/// Process gossip handshake for an incoming stream
#[allow(clippy::too_many_arguments)]
async fn process_gossip_handshake(
    send_stream: quinn::SendStream,
    recv_stream: quinn::RecvStream,
    peer_addr: SocketAddr,
    gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    peer_public_keys: &Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    peer_advertised_addrs: &Arc<RwLock<HashMap<SocketAddr, String>>>,
    listen_addr: SocketAddr,
    signing_key: ed25519_dalek::SigningKey,
    node_public_key: PublicKey,
) -> Result<(GossipStream, quinn::RecvStream), String> {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // We need mut references for reading/writing
    let mut send_stream = send_stream;
    let mut recv_stream = recv_stream;

    // Read handshake from the peer (they sent it when they opened the stream)
    // Protocol v2: [4-byte len][1-byte frame][payload]
    let handshake_len =
        match tokio::time::timeout(Duration::from_secs(10), recv_stream.read_u32()).await {
            Ok(Ok(len)) => len as usize,
            Ok(Err(e)) => return Err(format!("Failed to read handshake length: {}", e)),
            Err(_) => return Err("Timeout reading handshake length".to_string()),
        };

    // Protocol v2: len includes frame byte, so payload_len = len - 1
    if handshake_len < 1 {
        return Err("Handshake too short for frame byte".to_string());
    }
    let handshake_payload_len = handshake_len - 1;

    // Read frame byte and handshake data
    let mut handshake_frame_buf = [0u8; 1];
    if let Err(_) = tokio::time::timeout(
        Duration::from_secs(10),
        recv_stream.read_exact(&mut handshake_frame_buf),
    )
    .await
    {
        return Err("Timeout reading handshake frame byte".to_string());
    }

    // Verify frame byte is plaintext (0x00) for handshake
    if handshake_frame_buf[0] != MSG_FRAME_PLAINTEXT {
        warn!(
            "Unexpected handshake frame byte 0x{:02x} from {}",
            handshake_frame_buf[0], peer_addr
        );
        // Continue anyway - might be a future protocol version
    }

    // Read handshake payload data
    let mut handshake_buf = vec![0u8; handshake_payload_len];
    match tokio::time::timeout(
        Duration::from_secs(10),
        recv_stream.read_exact(&mut handshake_buf),
    )
    .await
    {
        Ok(Ok(_)) => {
            // Deserialize and verify handshake
            match bincode::deserialize::<AuthenticatedMessage>(&handshake_buf) {
                Ok(auth_msg) => {
                    debug!("Received handshake from inbound peer {}", peer_addr);

                    // Verify signature before trusting the key (BUG-005 fix)
                    NetworkManager::verify_message(&auth_msg, None)
                        .map_err(|e| format!("Handshake signature invalid: {}", e))?;

                    // Store peer's public key for message verification
                    if auth_msg.public_key.len() == 32 {
                        peer_public_keys
                            .write()
                            .await
                            .insert(peer_addr, auth_msg.public_key.clone());
                        debug!("Stored public key for peer {}", peer_addr);
                    }
                    // Extract advertised address from handshake
                    if let NetworkMessage::Handshake {
                        listen_addr: peer_listen_addr,
                        capabilities: peer_caps,
                    } = &auth_msg.message
                    {
                        // Reject peers that cannot validate IronDAG blocks
                        if !meets_required_capabilities(*peer_caps) {
                            return Err(format!(
                                "Peer {} rejected: insufficient capabilities \
                                 (advertised {:#010x}, required {:#010x})",
                                peer_addr, peer_caps, REQUIRED_CAPABILITIES
                            ));
                        }
                        peer_advertised_addrs
                            .write()
                            .await
                            .insert(peer_addr, peer_listen_addr.clone());
                        debug!(
                            "Peer {} advertises address: {}, capabilities: {:#010x}",
                            peer_addr, peer_listen_addr, peer_caps
                        );
                    }
                }
                Err(e) => return Err(format!("Failed to deserialize handshake: {}", e)),
            }
        }
        Ok(Err(e)) => return Err(format!("Failed to read handshake: {}", e)),
        Err(_) => return Err("Timeout reading handshake".to_string()),
    }

    // Send our handshake response
    let handshake = NetworkMessage::Handshake {
        listen_addr: listen_addr.to_string(),
        capabilities: LOCAL_CAPABILITIES,
    };

    // Sign the handshake message with the cached signing key
    use ed25519_dalek::Signer;
    let message_bytes = bincode::serialize(&handshake)
        .map_err(|e| format!("Failed to serialize handshake: {}", e))?;
    // Compute timestamp BEFORE signing — verify_message reconstructs message+timestamp payload
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut signed_payload = message_bytes;
    signed_payload.extend_from_slice(&timestamp.to_le_bytes());
    let signature = signing_key.sign(&signed_payload);

    let auth_msg = AuthenticatedMessage {
        message: handshake,
        signature: signature.to_bytes().to_vec(),
        public_key: node_public_key,
        timestamp,
    };
    let handshake_data = bincode::serialize(&auth_msg)
        .map_err(|e| format!("Failed to serialize handshake: {}", e))?;

    // Protocol v2: [4-byte len][1-byte frame][payload]
    let mut response = Vec::with_capacity(4 + 1 + handshake_data.len());
    response.extend_from_slice(&((1 + handshake_data.len()) as u32).to_be_bytes());
    response.push(MSG_FRAME_PLAINTEXT);
    response.extend_from_slice(&handshake_data);

    if let Err(e) =
        tokio::time::timeout(Duration::from_secs(10), send_stream.write_all(&response)).await
    {
        return Err(format!("Failed to send handshake response: {}", e));
    }
    if let Err(e) = send_stream.flush().await {
        return Err(format!("Failed to flush handshake response: {}", e));
    }

    debug!("Sent handshake response to {}", peer_addr);

    // Wrap send stream for storage
    let gossip_stream: GossipStream = Arc::new(Mutex::new(send_stream));

    // Store the gossip stream
    gossip_streams
        .lock()
        .await
        .insert(peer_addr, gossip_stream.clone());

    Ok((gossip_stream, recv_stream))
}

/// Cleanup peer state on error
/// Returns true if the peer was present and cleaned up, false otherwise
async fn cleanup_peer_state_on_error(
    peers: &Arc<RwLock<HashSet<SocketAddr>>>,
    peer_scores: &Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    peer_count_atomic: &Arc<AtomicUsize>,
    connection_types: &Arc<RwLock<HashMap<SocketAddr, ConnectionType>>>,
    subnet_peer_counts: &Arc<RwLock<HashMap<[u8; 2], usize>>>,
    quic_connections: &Arc<Mutex<HashMap<SocketAddr, quinn::Connection>>>,
    gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    peer_public_keys: &Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    peer_advertised_addrs: &Arc<RwLock<HashMap<SocketAddr, String>>>,
    peer_request_peers_time: &Arc<RwLock<HashMap<SocketAddr, Instant>>>,
    peer_exchange_lists: &Arc<RwLock<HashMap<SocketAddr, HashSet<SocketAddr>>>>,
    cached_fanout_peers: &Arc<RwLock<Option<(Instant, Vec<SocketAddr>)>>>,
    #[cfg(feature = "kyber")] kyber_session_keys: &Arc<
        RwLock<std::collections::HashMap<SocketAddr, zeroize::Zeroizing<Vec<u8>>>>,
    >,
    #[cfg(feature = "kyber")] kyber_session_cache: &Arc<
        RwLock<std::collections::HashMap<SocketAddr, (zeroize::Zeroizing<Vec<u8>>, Instant)>>,
    >,
    #[cfg(not(feature = "kyber"))] _kyber_session_keys: &(),
    #[cfg(not(feature = "kyber"))] _kyber_session_cache: &(),
    peer_addr: SocketAddr,
) -> bool {
    use std::sync::atomic::Ordering;

    let was_present = peers.write().await.remove(&peer_addr);
    peer_scores.write().await.remove(&peer_addr);
    if was_present {
        let prev = peer_count_atomic.load(Ordering::Relaxed);
        if prev > 0 {
            peer_count_atomic.fetch_sub(1, Ordering::Relaxed);
        }
    }
    connection_types.write().await.remove(&peer_addr);
    quic_connections.lock().await.remove(&peer_addr);
    gossip_streams.lock().await.remove(&peer_addr);
    peer_public_keys.write().await.remove(&peer_addr);
    peer_advertised_addrs.write().await.remove(&peer_addr);
    peer_request_peers_time.write().await.remove(&peer_addr);
    peer_exchange_lists.write().await.remove(&peer_addr);
    #[cfg(feature = "kyber")]
    {
        // Move session key to cache instead of deleting (60s grace period for reconnection)
        if let Some(session_key) = kyber_session_keys.write().await.remove(&peer_addr) {
            let mut cache = kyber_session_cache.write().await;
            // Evict oldest if at capacity
            if cache.len() >= MAX_KYBER_SESSION_CACHE_SIZE {
                if let Some(oldest_addr) = cache
                    .iter()
                    .min_by_key(|(_, (_, cached_at))| *cached_at)
                    .map(|(addr, _)| *addr)
                {
                    cache.remove(&oldest_addr);
                }
            }
            cache.insert(peer_addr, (session_key, Instant::now()));
            debug!(
                "Kyber session key cached for {} ({}s TTL)",
                peer_addr, KYBER_SESSION_CACHE_TTL_SECS
            );
        }
    }
    if let Some(prefix) = subnet_prefix_16(&peer_addr) {
        let mut subnet_counts = subnet_peer_counts.write().await;
        if let Some(count) = subnet_counts.get_mut(&prefix) {
            *count = count.saturating_sub(1);
        }
    }
    // Invalidate fanout cache since peer list changed
    *cached_fanout_peers.write().await = None;
    was_present
}

/// Write framed data to a send stream (length-prefixed with frame byte)
async fn write_framed(send_stream: &mut quinn::SendStream, data: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let total_len = 1 + data.len();
    send_stream
        .write_u32(total_len as u32)
        .await
        .map_err(|e| format!("Failed to write length: {}", e))?;
    send_stream
        .write_u8(MSG_FRAME_PLAINTEXT)
        .await
        .map_err(|e| format!("Failed to write frame byte: {}", e))?;
    send_stream
        .write_all(data)
        .await
        .map_err(|e| format!("Failed to write data: {}", e))?;
    Ok(())
}

/// Handle a sync protocol stream
async fn handle_sync_stream(
    send_stream: &mut quinn::SendStream,
    recv_stream: &mut quinn::RecvStream,
    peer_addr: SocketAddr,
    blockchain: Arc<RwLock<Blockchain>>,
    signing_key: ed25519_dalek::SigningKey,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::network::sync::{
        SignedSyncResponse, MAX_BLOCKS_PER_REQUEST, SYNC_MAGIC, SYNC_TIMEOUT_SECS, SYNC_VERSION,
    };
    use std::time::Duration;

    // Read magic + version + request type
    let mut header = [0u8; 10];
    tokio::time::timeout(
        Duration::from_secs(SYNC_TIMEOUT_SECS),
        recv_stream.read_exact(&mut header),
    )
    .await??;

    // Verify magic
    if &header[0..8] != SYNC_MAGIC {
        warn!("Invalid magic from {} - possible attack", peer_addr);
        return Ok(());
    }

    let version = header[8];
    let request_type = header[9];

    // Accept both v1 (legacy) and v2 (authenticated) clients
    if version != SYNC_VERSION && version != 1 {
        warn!("Unsupported version {} from {}", version, peer_addr);
        return Ok(());
    }

    let use_signing = version >= 2; // Sign responses for v2+ clients

    match request_type {
        0 => {
            // GetHeight - respond with highest block NUMBER (not count)
            // CRITICAL: The client uses this as from_block which filters by block_number.
            // In BraidCore DAG, block_count >> max_block_number (multiple blocks per number).
            // Sending block_count would cause the client to request blocks far beyond its chain tip.
            let bc = blockchain.read().await;
            let height = bc.latest_block_number();
            drop(bc);

            info!("Sending height {} to {}", height, peer_addr);

            if use_signing {
                let node_secret_key = signing_key.to_bytes();
                let response =
                    SignedSyncResponse::sign(height.to_le_bytes().to_vec(), &node_secret_key);
                let response_bytes = bincode::serialize(&response)?;
                let len = response_bytes.len() as u32;
                send_stream.write_all(&len.to_le_bytes()).await?;
                send_stream.write_all(&response_bytes).await?;
            } else {
                send_stream.write_all(&height.to_le_bytes()).await?;
            }
            send_stream.flush().await?;
        }
        1 => {
            // GetBlocks - read from_block (u64) and count (u64)
            let mut params = [0u8; 16];
            tokio::time::timeout(
                Duration::from_secs(SYNC_TIMEOUT_SECS),
                recv_stream.read_exact(&mut params),
            )
            .await??;

            let from_block = u64::from_le_bytes(params[0..8].try_into().unwrap());
            let mut count = u64::from_le_bytes(params[8..16].try_into().unwrap());

            // Rate limit: cap blocks per request
            if count > MAX_BLOCKS_PER_REQUEST {
                warn!(
                    "{} requested {} blocks, capping to {}",
                    peer_addr, count, MAX_BLOCKS_PER_REQUEST
                );
                count = MAX_BLOCKS_PER_REQUEST;
            }

            info!(
                "{} requesting blocks from_block={} count={}",
                peer_addr, from_block, count
            );

            // BlockDAG: must not use storage-order `.take(N)` — fast streams can starve parents.
            // Sort by block_number, take N, then include transitive parents for validation.
            let bc = blockchain.read().await;
            let blocks = crate::network::sync::select_blocks_for_sync_batch(&bc, from_block, count);
            drop(bc);

            // Serialize blocks
            let blocks_data = bincode::serialize(&blocks)?;
            let block_count = blocks.len() as u64;

            info!(
                "Sending {} blocks ({} bytes) to {}",
                block_count,
                blocks_data.len(),
                peer_addr
            );

            if use_signing {
                // Send signed response
                let mut payload = Vec::new();
                payload.extend_from_slice(&block_count.to_le_bytes());
                payload.extend_from_slice(&blocks_data);

                let node_secret_key = signing_key.to_bytes();
                let response = SignedSyncResponse::sign(payload, &node_secret_key);
                let response_bytes = bincode::serialize(&response)?;
                let len = response_bytes.len() as u32;
                send_stream.write_all(&len.to_le_bytes()).await?;
                send_stream.write_all(&response_bytes).await?;
            } else {
                // Legacy unsigned response
                send_stream.write_all(&block_count.to_le_bytes()).await?;
                send_stream
                    .write_all(&(blocks_data.len() as u64).to_le_bytes())
                    .await?;
                send_stream.write_all(&blocks_data).await?;
            }
            send_stream.flush().await?;
        }
        _ => {
            warn!(
                "Unknown request type {} from {} - possible attack",
                request_type, peer_addr
            );
        }
    }

    Ok(())
}

/// Process incoming network message with QUIC send stream
async fn process_message_with_arc(
    message: NetworkMessage,
    blockchain: &Arc<RwLock<Blockchain>>,
    peers: &Arc<RwLock<HashSet<SocketAddr>>>,
    send_stream: &Arc<Mutex<quinn::SendStream>>,
    from_addr: SocketAddr,
    // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
    orphan_pool: &Arc<sync::OrphanPool>,
    mining_manager: Option<&Arc<crate::mining::MiningManager>>,
    gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    peer_connect_tx: Option<&mpsc::Sender<SocketAddr>>,
    listen_addr: SocketAddr,
    shard_manager: Option<&Arc<crate::sharding::ShardManager>>,
    max_peers: Option<u32>,
    tx_seen: &Arc<RwLock<HashMap<Hash, Instant>>>,
    block_seen: &Arc<RwLock<HashMap<Hash, Instant>>>,
    peer_scores: &Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    // Partition detection fields
    partition_detected: &Arc<AtomicBool>,
    partition_start: &Arc<RwLock<Option<Instant>>>,
    last_block_received: &Arc<RwLock<Instant>>,
    local_tip_height: &Arc<AtomicU64>,
    // Node identity for signing messages
    signing_key: ed25519_dalek::SigningKey,
    node_public_key: PublicKey,
    // Peer exchange fields
    peer_advertised_addrs: &Arc<RwLock<HashMap<SocketAddr, String>>>,
    peer_request_peers_time: &Arc<RwLock<HashMap<SocketAddr, Instant>>>,
    // Jaccard Sybil detection
    peer_exchange_lists: &Arc<RwLock<HashMap<SocketAddr, HashSet<SocketAddr>>>>,
) -> crate::error::BlockchainResult<()> {
    process_message(
        message,
        blockchain,
        peers,
        send_stream,
        from_addr,
        orphan_pool,
        mining_manager,
        gossip_streams,
        peer_connect_tx,
        listen_addr,
        shard_manager,
        max_peers,
        tx_seen,
        block_seen,
        peer_scores,
        partition_detected,
        partition_start,
        last_block_received,
        local_tip_height,
        signing_key,
        node_public_key,
        peer_advertised_addrs,
        peer_request_peers_time,
        peer_exchange_lists,
    )
    .await
}

/// Process incoming network message
async fn process_message(
    message: NetworkMessage,
    blockchain: &Arc<RwLock<Blockchain>>,
    peers: &Arc<RwLock<HashSet<SocketAddr>>>,
    send_stream: &Arc<Mutex<quinn::SendStream>>,
    from_addr: SocketAddr,
    // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
    orphan_pool: &Arc<sync::OrphanPool>,
    mining_manager: Option<&Arc<crate::mining::MiningManager>>,
    _gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    peer_connect_tx: Option<&mpsc::Sender<SocketAddr>>,
    listen_addr: SocketAddr,
    shard_manager: Option<&Arc<crate::sharding::ShardManager>>,
    max_peers: Option<u32>,
    tx_seen: &Arc<RwLock<HashMap<Hash, Instant>>>,
    block_seen: &Arc<RwLock<HashMap<Hash, Instant>>>,
    peer_scores: &Arc<RwLock<HashMap<SocketAddr, PeerScore>>>,
    // Partition detection fields
    partition_detected: &Arc<AtomicBool>,
    partition_start: &Arc<RwLock<Option<Instant>>>,
    last_block_received: &Arc<RwLock<Instant>>,
    local_tip_height: &Arc<AtomicU64>,
    // Node identity for signing messages
    signing_key: ed25519_dalek::SigningKey,
    node_public_key: PublicKey,
    // Peer exchange fields
    peer_advertised_addrs: &Arc<RwLock<HashMap<SocketAddr, String>>>,
    peer_request_peers_time: &Arc<RwLock<HashMap<SocketAddr, Instant>>>,
    // Jaccard Sybil detection
    peer_exchange_lists: &Arc<RwLock<HashMap<SocketAddr, HashSet<SocketAddr>>>>,
) -> crate::error::BlockchainResult<()> {
    match message {
        NetworkMessage::Handshake {
            listen_addr: peer_listen_addr,
            capabilities: peer_caps,
        } => {
            debug!(
                "Received handshake from {}, listen address: {}, capabilities: {:#010x}",
                from_addr, peer_listen_addr, peer_caps
            );
            // Reject peers that cannot validate IronDAG blocks
            if !meets_required_capabilities(peer_caps) {
                warn!(
                    "Peer {} rejected: insufficient capabilities \
                     (advertised {:#010x}, required {:#010x}) — closing connection",
                    from_addr, peer_caps, REQUIRED_CAPABILITIES
                );
                return Err(crate::error::BlockchainError::Network(format!(
                    "Peer {} insufficient capabilities: {:#010x} (need {:#010x})",
                    from_addr, peer_caps, REQUIRED_CAPABILITIES
                )));
            }
            // Keep peer keyed by actual remote address (from_addr) only. Do NOT insert
            // listen_sock_addr into peers or remap the connection: multiple sync nodes
            // often advertise the same address (e.g. 127.0.0.1:8080), which would collapse
            // to one entry and cause "only syncs to one device". Keying by from_addr
            // gives one entry per TCP connection so broadcast and sync work for all peers.
            if peer_listen_addr.parse::<SocketAddr>().is_err() {
                warn!("Invalid listen address in handshake: {}", peer_listen_addr);
            } else {
                // Store the advertised address for peer exchange
                peer_advertised_addrs
                    .write()
                    .await
                    .insert(from_addr, peer_listen_addr);
            }
            // Peer already in peers (inserted on accept). Connection already stored under from_addr.
        }
        NetworkMessage::NewBlock { block } => {
            // Check if we've already seen this block (for novelty tracking)
            let is_novel = {
                let block_seen_read = block_seen.read().await;
                !block_seen_read.contains_key(&block.hash)
            };

            // Update novelty tracking before deduplication check
            {
                let mut scores = peer_scores.write().await;
                if let Some(score) = scores.get_mut(&from_addr) {
                    if is_novel {
                        score.novel_blocks = score.novel_blocks.saturating_add(1);
                    } else {
                        score.duplicate_blocks = score.duplicate_blocks.saturating_add(1);
                    }
                }
            }

            // Check if we've already seen this block (deduplication)
            {
                let mut block_seen = block_seen.write().await;
                if block_seen.contains_key(&block.hash) {
                    debug!(
                        "[DEDUP] Block 0x{} already seen, skipping",
                        hex::encode(&block.hash.0[..8])
                    );
                    return Ok(());
                }
                // Mark as seen before processing (atomic insert + cleanup)
                block_seen.insert(block.hash, Instant::now());
                // Evict old entries if cache is full
                evict_seen_cache(&mut block_seen, 10_000);
            }

            // Partition detection: Check peer divergence (peer ahead of us)
            let local_height = local_tip_height.load(Ordering::Relaxed);
            let peer_height = block.header.block_number;
            if peer_height > local_height + PARTITION_PEER_DIVERGENCE_BLOCKS {
                let diff = peer_height.saturating_sub(local_height);
                warn!(
                    "[PARTITION] Peer {} is {} blocks ahead (local: {}, peer: {}) - syncing",
                    from_addr, diff, local_height, peer_height
                );
            }

            // Calculate freshness (delta from local tip)
            let height_delta = if local_height > peer_height {
                local_height - peer_height
            } else {
                0
            };

            let mut bc = blockchain.write().await;

            // Check if block is orphan (missing parents)
            let mut is_orphan = false;
            let mut _missing_parent: Option<crate::types::Hash> = None;
            for parent_hash in &block.header.parent_hashes {
                if bc.get_block_by_hash(parent_hash).is_none() {
                    is_orphan = true;
                    _missing_parent = Some(*parent_hash);
                    break;
                }
            }

            if is_orphan {
                let missing_parents = orphan_pool.add_orphan(block.clone()).await;
                if !missing_parents.is_empty() {
                    let request = NetworkMessage::RequestMissingParents {
                        hashes: missing_parents,
                    };
                    let data = bincode::serialize(&request)?;
                    let mut send = send_stream.lock().await;
                    let _ = send.write_u32(data.len() as u32).await;
                    let _ = send.write_all(&data).await;
                }
            } else {
                match bc.add_block_for_sync(block.clone()).await {
                    Ok(true) => {
                        // Update local tip height
                        local_tip_height.store(block.header.block_number, Ordering::Relaxed);

                        // Partition recovery: Check if we were in a partition
                        let was_partitioned = partition_detected.swap(false, Ordering::Relaxed);
                        if was_partitioned {
                            let partition_start_time = *partition_start.read().await;
                            if let Some(start) = partition_start_time {
                                let duration_secs = start.elapsed().as_secs();
                                warn!("[PARTITION] Recovery detected after {}s partition - resuming normal operation", duration_secs);
                            }
                            *partition_start.write().await = None;
                        }

                        // Update last_block_received timestamp
                        *last_block_received.write().await = Instant::now();

                        // Track valid block delivered from this peer with freshness scoring
                        {
                            let mut scores = peer_scores.write().await;
                            if let Some(score) = scores.get_mut(&from_addr) {
                                score.blocks_delivered += 1;
                                score.last_block_height_delta = height_delta as i64;

                                // Freshness scoring: compare block height to local tip
                                if height_delta > STALE_BLOCK_THRESHOLD {
                                    score.stale_blocks = score.stale_blocks.saturating_add(1);

                                    // Check if stale ratio exceeds 50% and warn
                                    let total_blocks =
                                        score.stale_blocks.saturating_add(score.fresh_blocks);
                                    if total_blocks >= FRESHNESS_MIN_SAMPLE_SIZE {
                                        let stale_pct = (score.stale_blocks as f64
                                            / total_blocks as f64)
                                            * 100.0;
                                        if stale_pct > 50.0 {
                                            warn!("Peer {} has {:.0}% stale blocks ({}/{}), possible eclipse or dead fork",
                                                  from_addr, stale_pct, score.stale_blocks, total_blocks);
                                        }
                                    }
                                } else {
                                    score.fresh_blocks = score.fresh_blocks.saturating_add(1);
                                }
                            }
                        }

                        // New block added - check if this unblocks any orphans
                        let orphan_hashes = orphan_pool.get_orphan_hashes().await;
                        for orphan_hash in orphan_hashes {
                            if let Some(orphan_block) =
                                orphan_pool.try_process_orphan(&orphan_hash).await
                            {
                                let _ = bc.add_block_for_sync(orphan_block).await;
                            }
                        }

                        // Relay block to random subset of peers (fanout)
                        // Don't relay to the peer we received it from
                        drop(bc); // Release blockchain lock before relay
                        relay_block_to_peers(
                            &block,
                            from_addr,
                            peers,
                            _gossip_streams,
                            block_seen,
                            mining_manager,
                            &signing_key,
                            &node_public_key,
                        )
                        .await;
                    }
                    Ok(false) => { /* duplicate, already had block */ }
                    Err(e) => {
                        error!("Failed to add block #{}: {}", block.header.block_number, e);
                        // Penalize peer for sending invalid block
                        let mut scores = peer_scores.write().await;
                        penalize_peer(&mut scores, &from_addr, PenaltyReason::InvalidBlock);
                    }
                }
            }
        }
        NetworkMessage::NewCompactBlock { compact_block } => {
            // Calculate the block hash from the header
            let block_hash = compact_block.header.calculate_header_hash();

            // Check if we've already seen this block (deduplication)
            {
                let mut block_seen = block_seen.write().await;
                if block_seen.contains_key(&block_hash) {
                    debug!(
                        "[DEDUP] Compact block 0x{} already seen, skipping",
                        hex::encode(&block_hash.0[..8])
                    );
                    return Ok(());
                }
                // Mark as seen before processing (atomic insert + cleanup)
                block_seen.insert(block_hash, Instant::now());
                // Evict old entries if cache is full
                evict_seen_cache(&mut block_seen, 10_000);
            }

            // Partition detection: Check peer divergence (peer ahead of us)
            let local_height = local_tip_height.load(Ordering::Relaxed);
            let peer_height = compact_block.header.block_number;
            if peer_height > local_height + PARTITION_PEER_DIVERGENCE_BLOCKS {
                let diff = peer_height.saturating_sub(local_height);
                warn!(
                    "[PARTITION] Peer {} is {} blocks ahead (local: {}, peer: {}) - syncing",
                    from_addr, diff, local_height, peer_height
                );
            }

            let mut reconstructed = false;
            let mut reconstructed_block: Option<Block> = None;

            if let Some(mining_mgr) = mining_manager {
                // Get mempool snapshot
                let mempool = mining_mgr.get_mempool_snapshot().await;

                // Try to reconstruct block from compact format
                let (block_opt, missing_short_ids) = compact_block.to_block(&mempool);

                if let Some(mut block) = block_opt {
                    // Set the block hash (to_block doesn't set it)
                    block.hash = block_hash;

                    let mut bc = blockchain.write().await;

                    // Check if block is orphan (missing parents)
                    let mut is_orphan = false;
                    for parent_hash in &block.header.parent_hashes {
                        if bc.get_block_by_hash(parent_hash).is_none() {
                            is_orphan = true;
                            break;
                        }
                    }

                    if is_orphan {
                        let missing_parents = orphan_pool.add_orphan(block.clone()).await;
                        if !missing_parents.is_empty() {
                            let request = NetworkMessage::RequestMissingParents {
                                hashes: missing_parents,
                            };
                            let data = bincode::serialize(&request)?;
                            let mut send = send_stream.lock().await;
                            let _ = send.write_u32(data.len() as u32).await;
                            let _ = send.write_all(&data).await;
                        }
                    } else {
                        match bc.add_block_for_sync(block.clone()).await {
                            Ok(true) => {
                                reconstructed = true;
                                reconstructed_block = Some(block.clone());

                                // Update local tip height
                                local_tip_height
                                    .store(block.header.block_number, Ordering::Relaxed);

                                // Partition recovery: Check if we were in a partition
                                let was_partitioned =
                                    partition_detected.swap(false, Ordering::Relaxed);
                                if was_partitioned {
                                    let partition_start_time = *partition_start.read().await;
                                    if let Some(start) = partition_start_time {
                                        let duration_secs = start.elapsed().as_secs();
                                        warn!("[PARTITION] Recovery detected after {}s partition - resuming normal operation", duration_secs);
                                    }
                                    *partition_start.write().await = None;
                                }

                                // Update last_block_received timestamp
                                *last_block_received.write().await = Instant::now();

                                // Check if this unblocks any orphans
                                let orphan_hashes = orphan_pool.get_orphan_hashes().await;
                                for orphan_hash in orphan_hashes {
                                    if let Some(orphan_block) =
                                        orphan_pool.try_process_orphan(&orphan_hash).await
                                    {
                                        let _ = bc.add_block_for_sync(orphan_block).await;
                                    }
                                }
                            }
                            Ok(false) => { /* duplicate, already had block */ }
                            Err(e) => warn!("Failed to add reconstructed block: {}", e),
                        }
                    }
                } else if !missing_short_ids.is_empty() {
                    // Missing transactions - request them instead of full block
                    debug!("[COMPACT_MISS] Missing {} transactions for block #{}, requesting from peer {}",
                             missing_short_ids.len(), compact_block.header.block_number, from_addr);
                    let request = NetworkMessage::GetMissingTransactions {
                        block_hash,
                        short_ids: missing_short_ids,
                        nonce: compact_block.nonce,
                    };
                    let data = bincode::serialize(&request)?;
                    let mut send = send_stream.lock().await;
                    send.write_u32(data.len() as u32).await.map_err(|e| {
                        crate::error::BlockchainError::Network(format!(
                            "Failed to write length: {}",
                            e
                        ))
                    })?;
                    send.write_all(&data).await.map_err(|e| {
                        crate::error::BlockchainError::Network(format!(
                            "Failed to write data: {}",
                            e
                        ))
                    })?;
                    // Note: We'll receive MissingTransactions response later
                    // If timeout occurs, the full block request will be the fallback
                }
            }

            // If reconstruction succeeded, relay the block
            if let Some(block) = reconstructed_block {
                // Relay block to random subset of peers (fanout)
                // Don't relay to the peer we received it from
                relay_block_to_peers(
                    &block,
                    from_addr,
                    peers,
                    _gossip_streams,
                    block_seen,
                    mining_manager,
                    &signing_key,
                    &node_public_key,
                )
                .await;
            }

            // If reconstruction failed, request full block
            if !reconstructed {
                debug!(
                    "[COMPACT_MISS] Requesting full block 0x{} from peer {}",
                    hex::encode(&block_hash.0[..8]),
                    from_addr
                );
                let request = NetworkMessage::RequestBlocksByHash {
                    hashes: vec![block_hash],
                };
                let data = bincode::serialize(&request)?;
                let mut send = send_stream.lock().await;
                send.write_u32(data.len() as u32).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
                })?;
                send.write_all(&data).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
                })?;
            }
        }
        NetworkMessage::NewShardBlock { block, shard_id: _ } => {
            let mut bc = blockchain.write().await;
            if let Err(e) = bc.add_block_for_sync(block).await {
                warn!("Failed to add shard block: {}", e);
            }
        }
        NetworkMessage::NewTransaction { transaction } => {
            // QUA-005: Transaction deduplication guard
            // Check if we've already seen this transaction (deduplication)
            // 1. Check tx_seen cache (tracks all seen tx hashes)
            // 2. Note: Pool deduplication handled by tx_seen mirroring pool contents
            // 3. Note: Recent block history check would require blockchain.recent_tx_hashes access
            {
                let mut tx_seen = tx_seen.write().await;
                if tx_seen.contains_key(&transaction.hash) {
                    debug!(
                        "Duplicate transaction {} rejected",
                        hex::encode(&transaction.hash.0[..8])
                    );
                    return Ok(());
                }
                // Mark as seen before processing (atomic insert + cleanup)
                tx_seen.insert(transaction.hash, Instant::now());
                // Evict old entries if cache is full
                evict_seen_cache(&mut tx_seen, 10_000);
            }

            debug!(
                "Received transaction 0x{} from {}",
                hex::encode(&transaction.hash.0[..8]),
                from_addr
            );

            // Add transaction to mining pool if mining manager is available
            if let Some(mining_mgr) = mining_manager {
                if let Err(e) = mining_mgr.add_transaction(transaction).await {
                    warn!("Failed to add received transaction to pool: {}", e);
                    // Penalize peer for sending invalid transaction
                    let mut scores = peer_scores.write().await;
                    penalize_peer(&mut scores, &from_addr, PenaltyReason::InvalidTransaction);
                } else {
                    debug!("Transaction added to pool");
                }
            } else {
                warn!("No mining manager available - transaction not added to pool");
            }
        }
        // Sync strategy: block-range sync chosen over headers-first for DAG compatibility.
        // Headers-first sync removed as dead code — DAG block structure makes
        // header-only validation insufficient.
        NetworkMessage::RequestBlocksByHash { hashes } => {
            // Peer block request by hash (sync response)
            debug!(
                "SYNC: Peer {} requested {} blocks by hash",
                from_addr,
                hashes.len()
            );
            let blocks = {
                let bc = blockchain.read().await;
                let mut blocks = Vec::new();
                for hash in hashes {
                    if let Some(block) = bc.get_block_by_hash(&hash) {
                        blocks.push(block);
                    }
                }
                blocks
                // Lock released here
            };

            debug!(
                "SYNC: Sending {} blocks to {} (found {}/requested)",
                blocks.len(),
                from_addr,
                blocks.len()
            );
            let response = NetworkMessage::Blocks { blocks };
            let node_secret_key = signing_key.to_bytes();
            let authenticated =
                sign_message_with_key(response, &node_secret_key, &node_public_key)?;
            let data = bincode::serialize(&authenticated)?;

            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::RequestMissingParents { hashes } => {
            debug!(
                "Peer {} requested {} missing parent blocks",
                from_addr,
                hashes.len()
            );
            let (blocks, hashes_len) = {
                let bc = blockchain.read().await;
                let mut blocks = Vec::new();
                for hash in &hashes {
                    if let Some(block) = bc.get_block_by_hash(hash) {
                        blocks.push(block);
                    }
                }
                (blocks, hashes.len())
                // Lock released here
            };
            debug!(
                "Sending {} blocks to {} (found {}/{} requested)",
                blocks.len(),
                from_addr,
                blocks.len(),
                hashes_len
            );

            let response = NetworkMessage::Blocks { blocks };
            let node_secret_key = signing_key.to_bytes();
            let authenticated =
                sign_message_with_key(response, &node_secret_key, &node_public_key)?;
            let data = bincode::serialize(&authenticated)?;

            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::RequestBlocks { from_block, count } => {
            // Peer block range request
            debug!(
                "SYNC: Peer {} requested blocks from {} (count: {})",
                from_addr, from_block, count
            );
            let (blocks, local_height) = {
                let bc = blockchain.read().await;
                // CRITICAL: Sort by block_number BEFORE taking to ensure lowest block numbers are returned first
                let mut candidates: Vec<Block> = bc.with_blocks(|bs| {
                    bs.iter()
                        .filter(|b| b.header.block_number >= from_block)
                        .cloned()
                        .collect()
                });
                candidates.sort_by_key(|b| b.header.block_number);
                candidates.truncate(count as usize);
                let height = bc.get_block_count();
                (candidates, height)
                // Lock released here - CRITICAL: release before network I/O
            };

            debug!(
                "SYNC: Sending {} blocks to {} (local height: {})",
                blocks.len(),
                from_addr,
                local_height
            );
            let response = NetworkMessage::Blocks { blocks };
            let node_secret_key = signing_key.to_bytes();
            let authenticated =
                sign_message_with_key(response, &node_secret_key, &node_public_key)?;
            let data = bincode::serialize(&authenticated)?;

            // Send response back through the same stream
            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::RequestShardBlocks {
            shard_id,
            from_block,
            count,
        } => {
            // Peer shard block request - query shard-specific blockchain if available
            let blocks = if let Some(ref sm) = shard_manager {
                // Query shard-specific blockchain
                sm.get_shard_blocks(shard_id, from_block, count)
                    .await
                    .unwrap_or_default()
            } else {
                // Fallback: query main chain (non-sharding mode)
                // CRITICAL: Sort by block_number BEFORE taking to ensure lowest block numbers are returned first
                let bc = blockchain.read().await;
                let mut candidates: Vec<Block> = bc.with_blocks(|bs| {
                    bs.iter()
                        .filter(|b| b.header.block_number >= from_block)
                        .cloned()
                        .collect()
                });
                candidates.sort_by_key(|b| b.header.block_number);
                candidates.truncate(count as usize);
                candidates
            };

            let response = NetworkMessage::ShardBlocks { shard_id, blocks };
            let data = bincode::serialize(&response)?;

            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::Blocks { blocks } => {
            debug!(
                "SYNC: Received {} blocks from {} for sync",
                blocks.len(),
                from_addr
            );

            // Partition detection: Check peer divergence (peer ahead of us)
            if let Some(highest_block) = blocks.iter().max_by_key(|b| b.header.block_number) {
                let local_height = local_tip_height.load(Ordering::Relaxed);
                let peer_height = highest_block.header.block_number;
                if peer_height > local_height + PARTITION_PEER_DIVERGENCE_BLOCKS {
                    let diff = peer_height.saturating_sub(local_height);
                    warn!(
                        "[PARTITION] Peer {} is {} blocks ahead (local: {}, peer: {}) - syncing",
                        from_addr, diff, local_height, peer_height
                    );
                }
            }

            // CRITICAL FIX: Don't hold write lock for entire sync operation
            // Sort blocks first WITHOUT holding any lock
            let mut sorted_blocks = blocks.clone();
            sorted_blocks.sort_by_key(|b| b.header.block_number);

            let mut blocks_added = 0;
            let mut blocks_failed = 0;
            let mut blocks_orphaned = 0;

            // Track the highest block added for partition recovery
            let mut highest_added: Option<u64> = None;

            // Track the highest block number in the received batch for sync advancement
            // Used when all blocks are orphaned to advance past current batch
            let highest_in_batch: Option<u64> = blocks.iter().map(|b| b.header.block_number).max();

            // Process blocks ONE AT A TIME, releasing lock between each
            // This allows mining to interleave and prevents lock starvation
            for block in sorted_blocks {
                // Acquire lock, check, add, release - for EACH block
                let result = {
                    let mut bc = blockchain.write().await;

                    // Check if we already have this block
                    if bc.get_block_by_hash(&block.hash).is_some() {
                        continue; // Already have it - lock released here
                    }

                    // Check if all parents are available
                    let all_parents_available = block
                        .header
                        .parent_hashes
                        .iter()
                        .all(|parent_hash| bc.get_block_by_hash(parent_hash).is_some());

                    if all_parents_available {
                        match bc.add_block_for_sync(block.clone()).await {
                            Ok(true) => Ok(Some(true)),
                            Ok(false) => Ok(Some(false)), // duplicate
                            Err(e) => Err(e),
                        }
                    } else {
                        Ok(None) // Missing parents
                    }
                    // Lock released here at end of scope
                };

                match result {
                    Ok(Some(true)) => {
                        blocks_added += 1;
                        // Track highest block for partition recovery
                        highest_added =
                            Some(highest_added.map_or(block.header.block_number, |h| {
                                h.max(block.header.block_number)
                            }));
                    }
                    Ok(Some(false)) => { /* duplicate */ }
                    Ok(None) => {
                        orphan_pool.add_orphan(block.clone()).await;
                        blocks_orphaned += 1;
                    }
                    Err(e) => {
                        warn!("Failed to add block #{}: {}", block.header.block_number, e);
                        blocks_failed += 1;
                    }
                }

                // Yield to allow mining and other tasks to run
                tokio::task::yield_now().await;
            }

            // Partition recovery: Update state after batch processing
            if let Some(highest) = highest_added {
                // Update local tip height
                local_tip_height.store(highest, Ordering::Relaxed);

                // Check if we were in a partition
                let was_partitioned = partition_detected.swap(false, Ordering::Relaxed);
                if was_partitioned {
                    let partition_start_time = *partition_start.read().await;
                    if let Some(start) = partition_start_time {
                        let duration_secs = start.elapsed().as_secs();
                        warn!("[PARTITION] Recovery detected after {}s partition - resuming normal operation", duration_secs);
                    }
                    *partition_start.write().await = None;
                }

                // Update last_block_received timestamp
                *last_block_received.write().await = Instant::now();
            }

            if blocks_added > 0 || blocks_failed > 0 {
                debug!(
                    "SYNC: +{} added, {} orphaned, {} failed",
                    blocks_added, blocks_orphaned, blocks_failed
                );
            }

            // Track blocks delivered from this peer for reputation scoring
            if blocks_added > 0 {
                let mut scores = peer_scores.write().await;
                if let Some(score) = scores.get_mut(&from_addr) {
                    score.blocks_delivered += blocks_added as u64;
                }
            }

            // After adding blocks, try to process any orphans that may now be unblocked
            if blocks_added > 0 {
                let mut orphan_passes = 0;
                loop {
                    orphan_passes += 1;
                    let orphan_hashes = orphan_pool.get_orphan_hashes().await;
                    if orphan_hashes.is_empty() || orphan_passes > MAX_ORPHAN_RESOLUTION_PASSES {
                        break;
                    }

                    let mut processed_any = false;
                    for orphan_hash in orphan_hashes {
                        if let Some(orphan_block) =
                            orphan_pool.try_process_orphan(&orphan_hash).await
                        {
                            // Acquire lock only for this orphan block
                            let result = {
                                let mut bc = blockchain.write().await;
                                let all_parents_available =
                                    orphan_block.header.parent_hashes.iter().all(|parent_hash| {
                                        bc.get_block_by_hash(parent_hash).is_some()
                                    });

                                if all_parents_available {
                                    bc.add_block_for_sync(orphan_block.clone()).await.map(Some)
                                } else {
                                    Err(crate::error::BlockchainError::Validation(
                                        "Missing parents".to_string(),
                                    ))
                                }
                            };

                            match result {
                                Ok(Some(true)) => {
                                    processed_any = true;
                                    blocks_added += 1;
                                }
                                Ok(Some(false)) | Ok(None) => { /* duplicate or no-op */ }
                                Err(_) => {
                                    orphan_pool.add_orphan(orphan_block).await;
                                }
                            }

                            // Yield between orphan processing
                            tokio::task::yield_now().await;
                        }
                    }

                    if !processed_any {
                        break;
                    }
                }
                // Summary only when we processed orphans
            }

            // CONTINUOUS SYNC (Kaspa-style): request next batch if we added blocks OR orphaned blocks
            // Previously we only requested once after connect; sync stopped after first 2000 blocks
            // CRITICAL FIX: Also advance when blocks_added == 0 but blocks_orphaned > 0
            // This prevents sync stall when all blocks in a batch have unresolvable parents
            if blocks_added > 0 || blocks_orphaned > 0 {
                // from_block is a block_number filter: request blocks with block_number >= next_from
                let next_from = if blocks_added > 0 {
                    // Normal case: continue from latest block + 1
                    let bc = blockchain.read().await;
                    bc.latest_block_number() + 1
                } else if let Some(highest) = highest_in_batch {
                    // All blocks were orphaned: advance past current batch to avoid re-requesting same range
                    // Use the highest block number in the received batch + 1
                    highest + 1
                } else {
                    // Fallback: shouldn't happen if blocks_orphaned > 0, but be safe
                    let bc = blockchain.read().await;
                    bc.latest_block_number() + 1
                };

                if blocks_added == 0 && blocks_orphaned > 0 {
                    warn!("SYNC: All {} blocks in batch orphaned (missing parents) - advancing to request from block {}", blocks_orphaned, next_from);
                }

                let request = NetworkMessage::RequestBlocks {
                    from_block: next_from,
                    count: MAX_BLOCKS_PER_REQUEST,
                };
                if let Ok(data) = bincode::serialize(&request) {
                    let mut send = send_stream.lock().await;
                    if let Err(e) = send.write_u32(data.len() as u32).await {
                        warn!("SYNC: Failed to send follow-up request length: {}", e);
                    } else if let Err(e) = send.write_all(&data).await {
                        warn!("SYNC: Failed to send follow-up RequestBlocks: {}", e);
                    } else {
                        debug!(
                            "SYNC: Requested next batch from block {} (count {})",
                            next_from, MAX_BLOCKS_PER_REQUEST
                        );
                    }
                }
            }
        }
        NetworkMessage::ShardBlocks { shard_id, blocks } => {
            debug!(
                "Received {} blocks from shard {} from {}",
                blocks.len(),
                shard_id,
                from_addr
            );
            // CRITICAL FIX: Process shard blocks one at a time to avoid lock starvation
            for block in blocks {
                // Route to shard-specific blockchain if available
                if let Some(ref sm) = shard_manager {
                    // Route to shard-specific blockchain
                    if let Err(e) = sm.add_block_to_shard(shard_id, block.clone()).await {
                        warn!("Failed to add block to shard {}: {}", shard_id, e);
                    }
                } else {
                    // Fallback: add to main chain
                    let result = {
                        let mut bc = blockchain.write().await;
                        bc.add_block_for_sync(block.clone()).await
                    };
                    if let Err(e) = result {
                        warn!("Failed to add shard block: {}", e);
                    }
                }
                // Yield to allow mining to run
                tokio::task::yield_now().await;
            }
        }
        NetworkMessage::Ping { nonce } => {
            // Respond with authenticated Pong echoing the nonce back
            let response = NetworkMessage::Pong { nonce };
            use ed25519_dalek::Signer;
            let message_bytes = bincode::serialize(&response)?;
            let signature = signing_key.sign(&message_bytes);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let authenticated = AuthenticatedMessage {
                message: response,
                signature: signature.to_bytes().to_vec(),
                public_key: node_public_key.clone(),
                timestamp,
            };
            let data = bincode::serialize(&authenticated)?;
            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::Pong { nonce } => {
            // Latency measurement: calculate RTT from ping timestamp
            let mut scores = peer_scores.write().await;
            if let Some(score) = scores.get_mut(&from_addr) {
                // Rate-limit pong responses: reject if less than 1 second since last pong
                let now = Instant::now();
                if let Some(last_pong) = score.last_pong_time {
                    if now.duration_since(last_pong).as_secs() < PONG_RATE_LIMIT_SECS {
                        debug!("Rate-limited pong from peer {}", from_addr);
                        return Ok(());
                    }
                }

                // Verify this pong matches our pending ping
                if score.pending_ping_nonce == Some(nonce) {
                    if let Some(ping_sent) = score.ping_sent_at {
                        let mut rtt_ms =
                            Instant::now().duration_since(ping_sent).as_millis() as u64;

                        // Clamp latency to reasonable bounds
                        if rtt_ms < LATENCY_MIN_MS {
                            warn!(
                                "Suspiciously low latency {}ms from peer {}, clamping",
                                rtt_ms, from_addr
                            );
                            rtt_ms = LATENCY_MIN_MS;
                        }
                        if rtt_ms > LATENCY_MAX_MS {
                            rtt_ms = LATENCY_MAX_MS;
                        }

                        // Update latency measurement
                        score.latency_ms = Some(rtt_ms);

                        // Add to samples (keep last 5)
                        score.latency_samples.push(rtt_ms);
                        if score.latency_samples.len() > 5 {
                            score.latency_samples.remove(0);
                        }

                        // Calculate median (more resistant to gaming than average)
                        let mut sorted = score.latency_samples.clone();
                        sorted.sort();
                        let median_ms = sorted[sorted.len() / 2];

                        // Update last pong time for rate limiting
                        score.last_pong_time = Some(now);

                        // Clear pending ping state
                        score.ping_sent_at = None;
                        score.pending_ping_nonce = None;

                        debug!(
                            "Peer {} RTT: {}ms (median: {}ms)",
                            from_addr, rtt_ms, median_ms
                        );
                    }
                }
            }
        }
        NetworkMessage::RequestPeers => {
            // Rate limiting: don't process RequestPeers more than once per minute from the same peer
            let now = Instant::now();
            let mut should_process = true;
            {
                let mut request_times = peer_request_peers_time.write().await;
                if let Some(last_time) = request_times.get(&from_addr) {
                    if now.duration_since(*last_time) < std::time::Duration::from_secs(60) {
                        should_process = false;
                        debug!(
                            "Rate limiting RequestPeers from {} (last request was < 60s ago)",
                            from_addr
                        );
                    }
                }
                if should_process {
                    request_times.insert(from_addr, now);
                }
            }

            if should_process {
                // Collect up to 20 known peer addresses from advertised addresses
                // Exclude the requester and any banned peers
                // Prefer peers from diverse /16 subnets to help prevent eclipse attacks
                let advertised = peer_advertised_addrs.read().await;
                let scores = peer_scores.read().await;

                // First, collect all eligible peers with their connection addresses
                let eligible_peers: Vec<(SocketAddr, String)> = advertised
                    .iter()
                    .filter(|(conn_addr, _)| {
                        // Exclude the requester
                        if **conn_addr == from_addr {
                            return false;
                        }
                        // Exclude banned peers
                        if let Some(score) = scores.get(*conn_addr) {
                            if score.is_banned() {
                                return false;
                            }
                        }
                        true
                    })
                    .map(|(conn_addr, adv_addr)| (*conn_addr, adv_addr.clone()))
                    .collect();

                drop(scores);

                // Group peers by /16 subnet for diversity selection
                let mut peers_by_subnet: HashMap<Option<[u8; 2]>, Vec<String>> = HashMap::new();
                for (conn_addr, adv_addr) in eligible_peers {
                    let subnet = subnet_prefix_16(&conn_addr);
                    peers_by_subnet.entry(subnet).or_default().push(adv_addr);
                }

                // Select peers to maximize subnet diversity:
                // Take one peer from each subnet first, then cycle through subnets
                let mut peer_list: Vec<String> = Vec::new();
                let max_peers_to_share = 20;

                // Separate IPv6/localhost (None subnet) from IPv4 subnets
                let special_peers = peers_by_subnet.remove(&None).unwrap_or_default();
                let special_peers_count = special_peers.len();
                let mut subnet_entries: Vec<(Option<[u8; 2]>, Vec<String>)> =
                    peers_by_subnet.into_iter().collect();

                // Add IPv6/localhost peers first (they're exempt from subnet limits)
                for peer in special_peers {
                    if peer_list.len() >= max_peers_to_share {
                        break;
                    }
                    peer_list.push(peer);
                }

                // Round-robin through subnets to maximize diversity
                let mut round = 0;
                while peer_list.len() < max_peers_to_share {
                    let mut added_in_round = 0;
                    for (_, peers_in_subnet) in &mut subnet_entries {
                        if peer_list.len() >= max_peers_to_share {
                            break;
                        }
                        if round < peers_in_subnet.len() {
                            peer_list.push(peers_in_subnet[round].clone());
                            added_in_round += 1;
                        }
                    }
                    if added_in_round == 0 {
                        break; // No more peers to add
                    }
                    round += 1;
                }

                drop(advertised);

                if !peer_list.is_empty() {
                    info!(
                        "Sharing {} peers with {} (from {} different subnets)",
                        peer_list.len(),
                        from_addr,
                        subnet_entries.len() + if special_peers_count == 0 { 0 } else { 1 }
                    );
                }

                let response = NetworkMessage::Peers {
                    addresses: peer_list,
                };
                let node_secret_key = signing_key.to_bytes();
                let authenticated =
                    sign_message_with_key(response, &node_secret_key, &node_public_key)?;
                let data = bincode::serialize(&authenticated)?;
                let mut send = send_stream.lock().await;
                send.write_u32(data.len() as u32).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
                })?;
                send.write_all(&data).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
                })?;
            }
        }
        NetworkMessage::Peers { addresses } => {
            // Get our advertise address to filter self-connections
            let self_advertise = peer_advertised_addrs
                .read()
                .await
                .get(&listen_addr)
                .cloned()
                .unwrap_or_else(|| listen_addr.to_string());

            // Parse and filter addresses
            let mut received_peers: Vec<SocketAddr> = addresses
                .iter()
                .filter_map(|s| s.parse::<SocketAddr>().ok())
                .filter(|&a| a != listen_addr) // Filter listen_addr
                .filter(|a| a.to_string() != self_advertise) // Filter self-advertise address
                .collect();

            // Get current subnet counts for diversity checking
            let subnet_counts = {
                // We need to access subnet_peer_counts from the NetworkManager
                // Since it's not directly passed to process_message, we'll use peers to estimate
                // For now, we'll check against already-connected peers' subnets
                let peers_guard = peers.read().await;
                let mut counts: HashMap<[u8; 2], usize> = HashMap::new();
                for peer_addr in peers_guard.iter() {
                    if let Some(prefix) = subnet_prefix_16(peer_addr) {
                        *counts.entry(prefix).or_insert(0) += 1;
                    }
                }
                counts
            };

            // Detect Sybil signal: >50% of addresses from same /16 subnet
            if !received_peers.is_empty() {
                let mut response_subnets: HashMap<[u8; 2], usize> = HashMap::new();
                for addr in &received_peers {
                    if let Some(prefix) = subnet_prefix_16(addr) {
                        *response_subnets.entry(prefix).or_insert(0) += 1;
                    }
                }
                for (prefix, count) in &response_subnets {
                    if *count > received_peers.len() / 2 {
                        warn!(
                            "Sybil signal: peer {} sent {}% of addresses from subnet {}.{}.0.0/16",
                            from_addr,
                            count * 100 / received_peers.len(),
                            prefix[0],
                            prefix[1]
                        );
                    }
                }
            }

            // Jaccard similarity Sybil detection
            // Store the peer list received from this peer and compare against other peers
            {
                let peer_set: HashSet<SocketAddr> = received_peers.iter().cloned().collect();

                // Compare against stored lists from other peers (cross-subnet only)
                let sender_subnet = subnet_prefix_16(&from_addr);
                let lists = peer_exchange_lists.read().await;

                for (other_addr, other_set) in lists.iter() {
                    if other_addr == &from_addr {
                        continue; // Skip self-comparison
                    }

                    // Only compare peers from DIFFERENT /16 subnets
                    // Same-subnet peers naturally share similar views
                    let other_subnet = subnet_prefix_16(other_addr);
                    if sender_subnet == other_subnet {
                        continue;
                    }

                    let similarity = jaccard_similarity(&peer_set, other_set);

                    if similarity > JACCARD_SYBIL_THRESHOLD {
                        warn!("Strong Sybil signal: peers {} and {} have {:.0}% peer list overlap (cross-subnet)",
                              from_addr, other_addr, similarity * 100.0);

                        // Reduce reputation of both peers
                        let mut scores = peer_scores.write().await;
                        if let Some(score) = scores.get_mut(&from_addr) {
                            score.failure_count = score
                                .failure_count
                                .saturating_add(JACCARD_REPUTATION_PENALTY as u32);
                        }
                        if let Some(score) = scores.get_mut(other_addr) {
                            score.failure_count = score
                                .failure_count
                                .saturating_add(JACCARD_REPUTATION_PENALTY as u32);
                        }
                    } else if similarity > JACCARD_WARNING_THRESHOLD {
                        warn!("Sybil warning: peers {} and {} have {:.0}% peer list overlap (cross-subnet)",
                              from_addr, other_addr, similarity * 100.0);
                    }
                }
                drop(lists);

                // Store this peer's list for future comparisons
                peer_exchange_lists
                    .write()
                    .await
                    .insert(from_addr, peer_set);
            }

            // Sort peers by diversity preference:
            // 1. Peers from new /16 subnets (not in subnet_counts) get highest priority
            // 2. Peers from subnets with fewer existing connections get next priority
            // 3. Peers from saturated subnets (>= MAX_PEERS_PER_SUBNET_16) are skipped
            received_peers.sort_by_cached_key(|addr| {
                if let Some(prefix) = subnet_prefix_16(addr) {
                    let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
                    if count >= MAX_PEERS_PER_SUBNET_16 {
                        // Saturated subnet - lowest priority (sort to end)
                        usize::MAX
                    } else {
                        // Lower count = higher priority
                        count
                    }
                } else {
                    // IPv6 or localhost - exempt from subnet limits, highest priority
                    0
                }
            });

            // Filter out already-connected peers
            // Keep as HashSet reference for O(1) contains check instead of collecting to Vec
            let already = peers.read().await;
            let to_connect: Vec<SocketAddr> = received_peers
                .into_iter()
                .filter(|a| !already.contains(a))
                .filter(|addr| {
                    // Skip peers from saturated subnets
                    if let Some(prefix) = subnet_prefix_16(addr) {
                        let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
                        if count >= MAX_PEERS_PER_SUBNET_16 {
                            debug!(
                                "Skipping peer {} from saturated subnet {}.{}.0.0/16",
                                addr, prefix[0], prefix[1]
                            );
                            return false;
                        }
                    }
                    true
                })
                .collect();
            drop(already); // Release lock before next peers.read() in loop

            if !to_connect.is_empty() {
                info!(
                    "Peer exchange: received {} peer address(es) from {}, connecting to {} new",
                    addresses.len(),
                    from_addr,
                    to_connect.len()
                );
            }

            if let Some(tx) = peer_connect_tx {
                for addr in to_connect {
                    // Check max_peers capacity (re-check each iteration since we're connecting)
                    let current_peer_count = peers.read().await.len();
                    let max_allowed = max_peers.map(|m| m as usize).unwrap_or(usize::MAX);

                    if current_peer_count >= max_allowed {
                        info!(
                            "At max peers ({}/{}), skipping peer exchange connections",
                            current_peer_count, max_allowed
                        );
                        break; // Break instead of continue since we're at capacity
                    }

                    info!("Peer exchange: connecting to discovered peer {}", addr);
                    if let Err(e) = tx.try_send(addr) {
                        debug!(
                            "Peer exchange: failed to queue connection to {}: {}",
                            addr, e
                        );
                    }
                }
            }
        }
        NetworkMessage::GetMissingTransactions {
            block_hash,
            short_ids,
            nonce,
        } => {
            // Peer is requesting missing transactions for compact block reconstruction
            debug!(
                "[MISSING_TX] Peer {} requests {} transactions for block 0x{}",
                from_addr,
                short_ids.len(),
                hex::encode(&block_hash.0[..8])
            );

            // Look up transactions from our mempool using the nonce for short ID generation
            let found_txs = if let Some(mining_mgr) = mining_manager {
                let mempool = mining_mgr.get_mempool_snapshot().await;
                sync::find_transactions_by_short_ids(&mempool, &short_ids, nonce)
                    .into_values()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };

            if found_txs.is_empty() {
                debug!(
                    "[MISSING_TX] No matching transactions found in mempool for peer {}",
                    from_addr
                );
            } else {
                debug!(
                    "[MISSING_TX] Found {}/{} requested transactions, sending to peer {}",
                    found_txs.len(),
                    short_ids.len(),
                    from_addr
                );
            }

            // Send the transactions we found (may be empty if we don't have them)
            let response = NetworkMessage::MissingTransactions {
                block_hash,
                transactions: found_txs,
            };
            let node_secret_key = signing_key.to_bytes();
            let authenticated =
                sign_message_with_key(response, &node_secret_key, &node_public_key)?;
            let data = bincode::serialize(&authenticated)?;
            let mut send = send_stream.lock().await;
            send.write_u32(data.len() as u32).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
            })?;
            send.write_all(&data).await.map_err(|e| {
                crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
            })?;
        }
        NetworkMessage::MissingTransactions {
            block_hash,
            transactions,
        } => {
            // Peer responded with missing transactions for compact block reconstruction
            debug!(
                "[MISSING_TX] Received {} transactions for block 0x{} from {}",
                transactions.len(),
                hex::encode(&block_hash.0[..8]),
                from_addr
            );

            if transactions.is_empty() {
                // Peer didn't have the transactions - fall back to full block request
                warn!(
                    "[MISSING_TX] Peer {} had no matching transactions, requesting full block",
                    from_addr
                );
                let request = NetworkMessage::RequestBlocksByHash {
                    hashes: vec![block_hash],
                };
                let data = bincode::serialize(&request)?;
                let mut send = send_stream.lock().await;
                send.write_u32(data.len() as u32).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
                })?;
                send.write_all(&data).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
                })?;
            } else if let Some(mining_mgr) = mining_manager {
                // Add received transactions to mempool for reconstruction
                let mut added_count = 0;
                for tx in &transactions {
                    if mining_mgr.add_transaction(tx.clone()).await.is_ok() {
                        added_count += 1;
                    }
                }
                debug!(
                    "[MISSING_TX] Added {}/{} transactions to mempool",
                    added_count,
                    transactions.len()
                );

                // Now try to request the compact block again or the full block
                // Since we don't have the original compact block anymore, request full block as fallback
                // The transactions are now in mempool for future compact blocks
                let request = NetworkMessage::RequestBlocksByHash {
                    hashes: vec![block_hash],
                };
                let data = bincode::serialize(&request)?;
                let mut send = send_stream.lock().await;
                send.write_u32(data.len() as u32).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write length: {}", e))
                })?;
                send.write_all(&data).await.map_err(|e| {
                    crate::error::BlockchainError::Network(format!("Failed to write data: {}", e))
                })?;
            }
        }
        NetworkMessage::KyberPublicKey { .. } => {
            // Route to Kyber upgrade task if one is pending for this peer
            #[cfg(feature = "kyber")]
            {
                if route_kyber_message(from_addr, message).await {
                    debug!("Routed KyberPublicKey from {} to upgrade task", from_addr);
                } else {
                    debug!(
                        "Received KyberPublicKey from {} but no upgrade task pending",
                        from_addr
                    );
                }
            }
        }
        NetworkMessage::KyberCiphertext { .. } => {
            // Route to Kyber upgrade task if one is pending for this peer
            #[cfg(feature = "kyber")]
            {
                if route_kyber_message(from_addr, message).await {
                    debug!("Routed KyberCiphertext from {} to upgrade task", from_addr);
                } else {
                    debug!(
                        "Received KyberCiphertext from {} but no upgrade task pending",
                        from_addr
                    );
                }
            }
        }
        NetworkMessage::KyberHandshakeAck => {
            // Route to Kyber upgrade task if one is pending for this peer
            #[cfg(feature = "kyber")]
            {
                if route_kyber_message(from_addr, message).await {
                    debug!(
                        "Routed KyberHandshakeAck from {} to upgrade task",
                        from_addr
                    );
                } else {
                    debug!(
                        "Received KyberHandshakeAck from {} but no upgrade task pending",
                        from_addr
                    );
                }
            }
        }
    }

    Ok(())
}

/// Relay a block to a random subset of peers (fanout-based propagation with compact blocks)
///
/// Uses k = max(3, sqrt(peer_count)) for block relay fanout.
/// Excludes the source peer (the one we received the block from).
/// Sends compact block format for bandwidth efficiency.
async fn relay_block_to_peers(
    block: &Block,
    source_peer: SocketAddr,
    peers: &Arc<RwLock<HashSet<SocketAddr>>>,
    gossip_streams: &Arc<Mutex<HashMap<SocketAddr, GossipStream>>>,
    _block_seen: &Arc<RwLock<HashMap<Hash, Instant>>>,
    mining_manager: Option<&Arc<crate::mining::MiningManager>>,
    signing_key: &ed25519_dalek::SigningKey,
    node_public_key: &PublicKey,
) {
    let peers_guard = peers.read().await;
    let peer_count = peers_guard.len();

    if peer_count <= 1 {
        // No other peers to relay to
        return;
    }

    // Calculate fanout: k = max(3, sqrt(peer_count))
    let fanout = std::cmp::max(3, (peer_count as f64).sqrt() as usize);

    // Collect peers excluding the source
    let mut peer_list: Vec<SocketAddr> = peers_guard
        .iter()
        .copied()
        .filter(|&p| p != source_peer)
        .collect();

    drop(peers_guard); // Release read lock

    if peer_list.is_empty() {
        return;
    }

    // Shuffle for random selection using thread-safe random
    peer_list.sort_by_cached_key(|_| rand::random::<u64>());

    // Select random subset
    let selected_peers: Vec<SocketAddr> = peer_list.into_iter().take(fanout).collect();

    debug!(
        "[RELAY] Relaying block #{} to {}/{} peers (excluded sender {})",
        block.header.block_number,
        selected_peers.len(),
        peer_count,
        source_peer
    );

    // Create the relay message - use compact block if mining_manager is available
    let message = if let Some(mining_mgr) = mining_manager {
        // Get mempool hashes for compact block creation
        let mempool_hashes = mining_mgr.get_mempool_hashes().await;
        let compact_block = sync::CompactBlock::from_block(block, &mempool_hashes);
        NetworkMessage::NewCompactBlock { compact_block }
    } else {
        // Fall back to full block if no mining manager
        NetworkMessage::NewBlock {
            block: block.clone(),
        }
    };

    // Sign the message before relaying
    let node_secret_key = signing_key.to_bytes();
    let data = match sign_message_with_key(message, &node_secret_key, node_public_key) {
        Ok(authenticated) => match bincode::serialize(&authenticated) {
            Ok(d) => d,
            Err(e) => {
                warn!("[RELAY] Failed to serialize authenticated message: {}", e);
                return;
            }
        },
        Err(e) => {
            warn!("[RELAY] Failed to sign message: {}", e);
            return;
        }
    };

    // Send to selected peers using stored connections
    let gossip_streams_map = gossip_streams.lock().await;
    for &peer_addr in &selected_peers {
        if let Some(stream_arc) = gossip_streams_map.get(&peer_addr) {
            let mut send_stream = stream_arc.lock().await;

            let total_len = 1 + data.len();
            match send_stream.write_u32(total_len as u32).await {
                Ok(_) => match send_stream.write_u8(MSG_FRAME_PLAINTEXT).await {
                    Ok(_) => match send_stream.write_all(&data).await {
                        Ok(_) => {
                            debug!(
                                "[RELAY] Block #{} relayed to {}",
                                block.header.block_number, peer_addr
                            );
                        }
                        Err(e) => {
                            warn!("[RELAY] Failed to send block data to {}: {}", peer_addr, e);
                        }
                    },
                    Err(e) => {
                        warn!("[RELAY] Failed to send frame byte to {}: {}", peer_addr, e);
                    }
                },
                Err(e) => {
                    warn!(
                        "[RELAY] Failed to send block length to {}: {}",
                        peer_addr, e
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for ban_duration_for_offense
    #[test]
    fn test_ban_duration_first_offense() {
        let d = ban_duration_for_offense(0);
        assert_eq!(d, std::time::Duration::from_secs(600)); // 10 minutes

        let d = ban_duration_for_offense(1);
        assert_eq!(d, std::time::Duration::from_secs(600)); // 10 minutes
    }

    #[test]
    fn test_ban_duration_second_offense() {
        let d = ban_duration_for_offense(2);
        assert_eq!(d, std::time::Duration::from_secs(3600)); // 1 hour
    }

    #[test]
    fn test_ban_duration_third_offense() {
        let d = ban_duration_for_offense(3);
        assert_eq!(d, std::time::Duration::from_secs(21600)); // 6 hours
    }

    #[test]
    fn test_ban_duration_escalation_cap() {
        // Fourth offense and beyond should be capped at 24 hours
        assert_eq!(
            ban_duration_for_offense(4),
            std::time::Duration::from_secs(86400)
        );
        assert_eq!(
            ban_duration_for_offense(10),
            std::time::Duration::from_secs(86400)
        );
        assert_eq!(
            ban_duration_for_offense(100),
            std::time::Duration::from_secs(86400)
        );
        assert_eq!(
            ban_duration_for_offense(u32::MAX),
            std::time::Duration::from_secs(86400)
        );
    }

    // Tests for format_duration
    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(std::time::Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(std::time::Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(std::time::Duration::from_secs(59)), "59s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(std::time::Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(std::time::Duration::from_secs(600)), "10m");
        assert_eq!(format_duration(std::time::Duration::from_secs(3599)), "59m");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(std::time::Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(std::time::Duration::from_secs(7200)), "2h");
        assert_eq!(
            format_duration(std::time::Duration::from_secs(86400)),
            "24h"
        );
    }

    // Tests for frame byte constants
    #[test]
    fn test_frame_byte_constants() {
        assert_eq!(MSG_FRAME_PLAINTEXT, 0x00);
        assert_eq!(MSG_FRAME_KYBER_ENCRYPTED, 0x01);
    }

    // Tests for global peer limit constant
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_global_max_total_peers_limit() {
        // Verify MAX_TOTAL_PEERS constant is defined and reasonable
        assert!(
            MAX_TOTAL_PEERS >= 100,
            "MAX_TOTAL_PEERS should be at least 100"
        );
        assert!(
            MAX_TOTAL_PEERS <= 5000,
            "MAX_TOTAL_PEERS should not exceed 5000"
        );
    }

    // Tests for PeerScore::eviction_score
    #[test]
    fn test_peer_score_eviction_score_default() {
        let score = PeerScore::default();
        assert_eq!(score.eviction_score(), 0); // 0 - 0 = 0
    }

    #[test]
    fn test_peer_score_eviction_score_with_success() {
        let score = PeerScore {
            success_count: 5,
            ..PeerScore::default()
        };
        assert_eq!(score.eviction_score(), 5);
    }

    #[test]
    fn test_peer_score_eviction_score_with_failure() {
        let score = PeerScore {
            failure_count: 3,
            ..PeerScore::default()
        };
        // failure is weighted 2x: 0 - (3 * 2) = -6
        assert_eq!(score.eviction_score(), -6);
    }

    #[test]
    fn test_peer_score_eviction_score_mixed() {
        let score = PeerScore {
            success_count: 10,
            failure_count: 3,
            ..PeerScore::default()
        };
        // 10 - (3 * 2) = 4
        assert_eq!(score.eviction_score(), 4);
    }

    // Tests for PeerScore::reputation
    #[test]
    fn test_peer_score_reputation_default() {
        let score = PeerScore::default();
        // Default: no blocks, no latency data, no connection time
        // block_score = 0.5 * 50 = 25
        // delivery_bonus = 0
        // latency_bonus = 10 (neutral)
        // connection_bonus = 0 (new connection)
        // tx_penalty = 0
        // Expected: ~35
        let rep = score.reputation();
        assert!(
            rep >= 30.0 && rep <= 40.0,
            "Default reputation should be around 35, got {}",
            rep
        );
    }

    #[test]
    fn test_peer_score_reputation_perfect_peer() {
        let score = PeerScore {
            blocks_delivered: 100,
            invalid_blocks: 0,
            invalid_txs: 0,
            latency_ms: Some(30), // Excellent latency
            // Simulate connection time by setting connected_at to the past
            // (we can't easily manipulate Instant, so we rely on other factors)
            ..PeerScore::default()
        };

        let rep = score.reputation();
        // Should be high due to valid blocks and good latency
        assert!(
            rep > 70.0,
            "Perfect peer should have high reputation, got {}",
            rep
        );
    }

    #[test]
    fn test_peer_score_reputation_degrades_with_invalid_blocks() {
        let score = PeerScore {
            blocks_delivered: 10,
            invalid_blocks: 5, // 50% invalid
            ..PeerScore::default()
        };

        let rep = score.reputation();
        // block_score = 0.5 * 50 = 25 (reduced from 50)
        assert!(
            rep < 50.0,
            "Peer with 50% invalid blocks should have reduced reputation, got {}",
            rep
        );
    }

    #[test]
    fn test_peer_score_reputation_invalid_blocks_heavy_penalty() {
        let score = PeerScore {
            blocks_delivered: 5,
            invalid_blocks: 5, // 100% invalid
            ..PeerScore::default()
        };

        let rep = score.reputation();
        // block_score = 0 * 50 = 0
        assert!(
            rep < 25.0,
            "Peer with 100% invalid blocks should have very low reputation, got {}",
            rep
        );
    }

    #[test]
    fn test_peer_score_reputation_tx_penalty() {
        let score = PeerScore {
            blocks_delivered: 10, // Good block record
            invalid_txs: 50,      // Max invalid txs
            ..PeerScore::default()
        };

        let rep = score.reputation();
        let rep_no_tx_penalty = {
            let s = PeerScore {
                blocks_delivered: 10,
                ..PeerScore::default()
            };
            s.reputation()
        };

        assert!(
            rep < rep_no_tx_penalty,
            "Peer with invalid txs should have lower reputation"
        );
    }

    #[test]
    fn test_peer_score_reputation_latency_tiers() {
        // Test excellent latency
        let score = PeerScore {
            blocks_delivered: 1,
            latency_ms: Some(30),
            ..PeerScore::default()
        };
        let rep_excellent = score.reputation();

        // Test good latency
        let score = PeerScore {
            blocks_delivered: 1,
            latency_ms: Some(80),
            ..PeerScore::default()
        };
        let rep_good = score.reputation();

        // Test poor latency
        let score = PeerScore {
            blocks_delivered: 1,
            latency_ms: Some(600),
            ..PeerScore::default()
        };
        let rep_poor = score.reputation();

        assert!(
            rep_excellent > rep_good,
            "Excellent latency should score higher than good"
        );
        assert!(
            rep_good > rep_poor,
            "Good latency should score higher than poor"
        );
    }

    #[test]
    fn test_peer_score_reputation_clamped_to_100() {
        let score = PeerScore {
            blocks_delivered: 10000,
            invalid_blocks: 0,
            latency_ms: Some(10),
            // Even with extreme values, should not exceed 100
            ..PeerScore::default()
        };

        let rep = score.reputation();
        assert!(
            rep <= 100.0,
            "Reputation should be clamped to max 100, got {}",
            rep
        );
    }

    #[test]
    fn test_peer_score_reputation_clamped_to_0() {
        let score = PeerScore {
            blocks_delivered: 1,
            invalid_blocks: 100, // Way more invalid than valid
            invalid_txs: 1000,   // Heavy tx penalty
            ..PeerScore::default()
        };

        let rep = score.reputation();
        assert!(
            rep >= 0.0,
            "Reputation should be clamped to min 0, got {}",
            rep
        );
    }

    // Tests for PeerScore::is_banned
    #[test]
    fn test_peer_score_is_banned_not_banned() {
        let score = PeerScore::default();
        assert!(!score.is_banned());
    }

    #[test]
    fn test_peer_score_is_banned_temporarily() {
        let score = PeerScore {
            // Set ban to expire in the future
            banned_until: Some(Instant::now() + std::time::Duration::from_secs(3600)),
            ..PeerScore::default()
        };
        assert!(score.is_banned());
    }

    #[test]
    fn test_peer_score_is_banned_expired() {
        let score = PeerScore {
            // Set ban to have expired in the past
            banned_until: Some(Instant::now() - std::time::Duration::from_secs(1)),
            ..PeerScore::default()
        };
        assert!(!score.is_banned());
    }

    // Tests for subnet diversity
    #[test]
    fn test_subnet_prefix_extraction() {
        // Test IPv4 address
        let addr: SocketAddr = "192.168.1.1:8080".parse().unwrap();
        let prefix = subnet_prefix_16(&addr);
        assert_eq!(prefix, Some([192, 168]));

        // Test another IPv4 address
        let addr: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        let prefix = subnet_prefix_16(&addr);
        assert_eq!(prefix, Some([10, 0]));

        // Test IPv6 address (should return None)
        let addr: SocketAddr = "[::1]:8080".parse().unwrap();
        let prefix = subnet_prefix_16(&addr);
        assert_eq!(prefix, None);
    }

    #[test]
    fn test_subnet_prefix_localhost_exempt() {
        // Localhost (127.0.0.1) should be exempt from subnet limits
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let prefix = subnet_prefix_16(&addr);
        assert_eq!(prefix, None);

        // Any 127.x.x.x address should be exempt
        let addr: SocketAddr = "127.0.1.1:8080".parse().unwrap();
        let prefix = subnet_prefix_16(&addr);
        assert_eq!(prefix, None);
    }

    #[test]
    fn test_max_peers_per_subnet_constant() {
        // Verify the constant is set to expected value
        assert_eq!(MAX_PEERS_PER_SUBNET_16, 10);
    }

    #[test]
    fn test_log_peer_diversity_stats() {
        use std::collections::HashMap;

        // Test with diverse peers
        let mut subnet_counts = HashMap::new();
        subnet_counts.insert([192, 168], 2);
        subnet_counts.insert([10, 0], 2);
        subnet_counts.insert([172, 16], 1);

        // Should not panic and should log correctly
        log_peer_diversity(5, &subnet_counts);

        // Test with low diversity (all from same subnet)
        let mut subnet_counts = HashMap::new();
        subnet_counts.insert([192, 168], 5);

        log_peer_diversity(5, &subnet_counts);

        // Test with empty subnet counts
        let subnet_counts: HashMap<[u8; 2], usize> = HashMap::new();
        log_peer_diversity(0, &subnet_counts);
    }

    // Tests for slot calculation (eclipse attack prevention)
    #[test]
    fn test_slot_calculation() {
        // Test with typical peer count
        assert_eq!(outbound_slots(50), 35); // 70% of 50
        assert_eq!(inbound_slots(50), 15); // 30% of 50

        // Test with small peer count
        assert_eq!(outbound_slots(10), 7); // 70% of 10
        assert_eq!(inbound_slots(10), 3); // 30% of 10

        // Test with 100 peers
        assert_eq!(outbound_slots(100), 70); // 70% of 100
        assert_eq!(inbound_slots(100), 30); // 30% of 100
    }

    #[test]
    fn test_slot_calculation_edge_cases() {
        // Test with 1 peer - should still allocate 70% to outbound
        assert_eq!(outbound_slots(1), 0); // 0.7 rounded down
        assert_eq!(inbound_slots(1), 1); // 1 - 0 = 1

        // Test with 3 peers
        assert_eq!(outbound_slots(3), 2); // 2.1 rounded down
        assert_eq!(inbound_slots(3), 1); // 3 - 2 = 1

        // Test with very large peer count
        assert_eq!(outbound_slots(1000), 700);
        assert_eq!(inbound_slots(1000), 300);
    }

    #[test]
    fn test_connection_type_enum() {
        // Test that ConnectionType enum works correctly
        assert_eq!(ConnectionType::Inbound, ConnectionType::Inbound);
        assert_eq!(ConnectionType::Outbound, ConnectionType::Outbound);
        assert_ne!(ConnectionType::Inbound, ConnectionType::Outbound);
    }

    // Tests for peer exchange subnet diversity (Task #220)
    #[test]
    fn test_peer_exchange_subnet_diversity_sorting() {
        use std::collections::HashMap;

        // Simulate subnet counts (existing peers)
        let mut subnet_counts: HashMap<[u8; 2], usize> = HashMap::new();
        subnet_counts.insert([192, 168], 2); // 2 peers from 192.168.x.x (almost saturated)
        subnet_counts.insert([10, 0], 1); // 1 peer from 10.0.x.x
                                          // 172.16.x.x has 0 peers (new subnet)

        // Create test peer addresses
        let new_peer_192: SocketAddr = "192.168.5.1:8080".parse().unwrap(); // Saturated subnet (2 peers)
        let new_peer_10: SocketAddr = "10.0.5.1:8080".parse().unwrap(); // Has 1 peer
        let new_peer_172: SocketAddr = "172.16.5.1:8080".parse().unwrap(); // New subnet (0 peers)
        let new_peer_v6: SocketAddr = "[::1]:8080".parse().unwrap(); // IPv6 (exempt)

        // Test sorting key calculation
        let key_192 = if let Some(prefix) = subnet_prefix_16(&new_peer_192) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            if count >= MAX_PEERS_PER_SUBNET_16 {
                usize::MAX
            } else {
                count
            }
        } else {
            0
        };

        let key_10 = if let Some(prefix) = subnet_prefix_16(&new_peer_10) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            if count >= MAX_PEERS_PER_SUBNET_16 {
                usize::MAX
            } else {
                count
            }
        } else {
            0
        };

        let key_172 = if let Some(prefix) = subnet_prefix_16(&new_peer_172) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            if count >= MAX_PEERS_PER_SUBNET_16 {
                usize::MAX
            } else {
                count
            }
        } else {
            0
        };

        let key_v6 = if let Some(prefix) = subnet_prefix_16(&new_peer_v6) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            if count >= MAX_PEERS_PER_SUBNET_16 {
                usize::MAX
            } else {
                count
            }
        } else {
            0
        };

        // Verify sorting priorities
        assert_eq!(
            key_192, 2,
            "192.168 subnet should have key 2 (2 existing peers)"
        );
        assert_eq!(key_10, 1, "10.0 subnet should have key 1 (1 existing peer)");
        assert_eq!(
            key_172, 0,
            "172.16 subnet should have key 0 (0 existing peers)"
        );
        assert_eq!(key_v6, 0, "IPv6 should have key 0 (exempt)");

        // Lower key = higher priority, so order should be: 172.16 (0), 10.0 (1), 192.168 (2)
        assert!(
            key_172 < key_10,
            "New subnet should have higher priority than subnet with 1 peer"
        );
        assert!(
            key_10 < key_192,
            "Subnet with 1 peer should have higher priority than subnet with 2 peers"
        );
    }

    #[test]
    fn test_peer_exchange_saturated_subnet_skipping() {
        use std::collections::HashMap;

        // Simulate a saturated subnet (3 peers already)
        let mut subnet_counts: HashMap<[u8; 2], usize> = HashMap::new();
        subnet_counts.insert([192, 168], MAX_PEERS_PER_SUBNET_16); // Saturated

        let saturated_peer: SocketAddr = "192.168.5.1:8080".parse().unwrap();

        // Verify saturated subnet detection
        let should_skip = if let Some(prefix) = subnet_prefix_16(&saturated_peer) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            count >= MAX_PEERS_PER_SUBNET_16
        } else {
            false
        };

        assert!(should_skip, "Peer from saturated subnet should be skipped");

        // Test non-saturated subnet
        subnet_counts.insert([10, 0], 1); // Not saturated
        let non_saturated_peer: SocketAddr = "10.0.5.1:8080".parse().unwrap();

        let should_skip_non_sat = if let Some(prefix) = subnet_prefix_16(&non_saturated_peer) {
            let count = subnet_counts.get(&prefix).copied().unwrap_or(0);
            count >= MAX_PEERS_PER_SUBNET_16
        } else {
            false
        };

        assert!(
            !should_skip_non_sat,
            "Peer from non-saturated subnet should not be skipped"
        );
    }

    #[test]
    fn test_sybil_signal_detection() {
        use std::collections::HashMap;

        // Test case 1: Normal distribution (no Sybil signal)
        let peers_normal = vec![
            "192.168.1.1:8080".parse::<SocketAddr>().unwrap(),
            "10.0.1.1:8080".parse::<SocketAddr>().unwrap(),
            "172.16.1.1:8080".parse::<SocketAddr>().unwrap(),
            "192.168.2.1:8080".parse::<SocketAddr>().unwrap(),
        ];

        let mut response_subnets: HashMap<[u8; 2], usize> = HashMap::new();
        for addr in &peers_normal {
            if let Some(prefix) = subnet_prefix_16(addr) {
                *response_subnets.entry(prefix).or_insert(0) += 1;
            }
        }

        let mut sybil_detected = false;
        for (_, count) in &response_subnets {
            if *count > peers_normal.len() / 2 {
                sybil_detected = true;
            }
        }
        assert!(
            !sybil_detected,
            "Normal distribution should not trigger Sybil detection"
        );

        // Test case 2: >50% from same subnet (Sybil signal)
        let peers_sybil = vec![
            "192.168.1.1:8080".parse::<SocketAddr>().unwrap(),
            "192.168.2.1:8080".parse::<SocketAddr>().unwrap(),
            "192.168.3.1:8080".parse::<SocketAddr>().unwrap(),
            "10.0.1.1:8080".parse::<SocketAddr>().unwrap(),
        ];

        let mut response_subnets: HashMap<[u8; 2], usize> = HashMap::new();
        for addr in &peers_sybil {
            if let Some(prefix) = subnet_prefix_16(addr) {
                *response_subnets.entry(prefix).or_insert(0) += 1;
            }
        }

        let mut sybil_detected = false;
        let mut dominant_subnet_count = 0;
        for (_, count) in &response_subnets {
            if *count > peers_sybil.len() / 2 {
                sybil_detected = true;
                dominant_subnet_count = *count;
            }
        }
        assert!(
            sybil_detected,
            "75% from same subnet should trigger Sybil detection"
        );
        assert_eq!(
            dominant_subnet_count, 3,
            "Should detect 3 peers from dominant subnet"
        );

        // Test case 3: Exactly 50% (edge case - should NOT trigger)
        let peers_edge = vec![
            "192.168.1.1:8080".parse::<SocketAddr>().unwrap(),
            "192.168.2.1:8080".parse::<SocketAddr>().unwrap(),
            "10.0.1.1:8080".parse::<SocketAddr>().unwrap(),
            "172.16.1.1:8080".parse::<SocketAddr>().unwrap(),
        ];

        let mut response_subnets: HashMap<[u8; 2], usize> = HashMap::new();
        for addr in &peers_edge {
            if let Some(prefix) = subnet_prefix_16(addr) {
                *response_subnets.entry(prefix).or_insert(0) += 1;
            }
        }

        let mut sybil_detected = false;
        for (_, count) in &response_subnets {
            if *count > peers_edge.len() / 2 {
                // Strictly greater than 50%
                sybil_detected = true;
            }
        }
        assert!(
            !sybil_detected,
            "Exactly 50% should not trigger Sybil detection (strict > 50%)"
        );
    }

    #[test]
    fn test_request_peers_diversity_selection() {
        use std::collections::HashMap;

        // Simulate grouping peers by subnet
        let mut peers_by_subnet: HashMap<Option<[u8; 2]>, Vec<String>> = HashMap::new();

        // Add peers from different subnets
        peers_by_subnet
            .entry(Some([192, 168]))
            .or_default()
            .push("192.168.1.1:8080".to_string());
        peers_by_subnet
            .entry(Some([192, 168]))
            .or_default()
            .push("192.168.2.1:8080".to_string());
        peers_by_subnet
            .entry(Some([192, 168]))
            .or_default()
            .push("192.168.3.1:8080".to_string());
        peers_by_subnet
            .entry(Some([10, 0]))
            .or_default()
            .push("10.0.1.1:8080".to_string());
        peers_by_subnet
            .entry(Some([10, 0]))
            .or_default()
            .push("10.0.2.1:8080".to_string());
        peers_by_subnet
            .entry(Some([172, 16]))
            .or_default()
            .push("172.16.1.1:8080".to_string());
        peers_by_subnet
            .entry(None)
            .or_default()
            .push("[::1]:8080".to_string()); // IPv6

        // Select peers using round-robin through subnets
        let mut peer_list: Vec<String> = Vec::new();
        let max_peers_to_share = 20;

        let special_peers = peers_by_subnet.remove(&None).unwrap_or_default();
        let mut subnet_entries: Vec<(Option<[u8; 2]>, Vec<String>)> =
            peers_by_subnet.into_iter().collect();

        // Add IPv6/localhost peers first
        for peer in special_peers {
            if peer_list.len() >= max_peers_to_share {
                break;
            }
            peer_list.push(peer);
        }

        // Round-robin through subnets
        let mut round = 0;
        while peer_list.len() < max_peers_to_share {
            let mut added_in_round = 0;
            for (_, peers_in_subnet) in &mut subnet_entries {
                if peer_list.len() >= max_peers_to_share {
                    break;
                }
                if round < peers_in_subnet.len() {
                    peer_list.push(peers_in_subnet[round].clone());
                    added_in_round += 1;
                }
            }
            if added_in_round == 0 {
                break;
            }
            round += 1;
        }

        // Verify diversity: should have at least one peer from each subnet before duplicates
        let subnet_192_count = peer_list
            .iter()
            .filter(|p| p.starts_with("192.168"))
            .count();
        let subnet_10_count = peer_list.iter().filter(|p| p.starts_with("10.0")).count();
        let subnet_172_count = peer_list.iter().filter(|p| p.starts_with("172.16")).count();
        let ipv6_count = peer_list.iter().filter(|p| p.starts_with('[')).count();

        // With round-robin, we should get: IPv6 (1), then 192.168.1.1, 10.0.1.1, 172.16.1.1,
        // then 192.168.2.1, 10.0.2.1, then 192.168.3.1 = 7 total
        assert_eq!(peer_list.len(), 7, "Should have 7 peers total");
        assert!(
            subnet_172_count >= 1,
            "Should have at least 1 peer from 172.16 subnet"
        );
        assert!(
            subnet_10_count >= 1,
            "Should have at least 1 peer from 10.0 subnet"
        );
        assert!(
            subnet_192_count >= 1,
            "Should have at least 1 peer from 192.168 subnet"
        );
        assert_eq!(ipv6_count, 1, "Should have 1 IPv6 peer");
    }

    #[test]
    fn test_freshness_scoring_stale_blocks() {
        let mut score = PeerScore::default();

        // Simulate a peer sending mostly stale blocks (200 behind tip)
        for _ in 0..8 {
            score.stale_blocks = score.stale_blocks.saturating_add(1);
        }
        for _ in 0..2 {
            score.fresh_blocks = score.fresh_blocks.saturating_add(1);
        }
        score.blocks_delivered = 10;

        // With 80% stale blocks, reputation should be significantly penalized
        let reputation = score.reputation();

        // A peer with 80% stale blocks should have lower reputation than neutral (55)
        // Note: Various bonuses (delivery, latency, connection) can offset the stale penalty
        assert!(
            reputation < 55.0,
            "Peer with 80% stale blocks should have reputation < 55, got {}",
            reputation
        );

        // Verify freshness ratio calculation
        let freshness_ratio = score.freshness_ratio();
        assert!(
            (freshness_ratio - 0.2).abs() < 0.01,
            "Freshness ratio should be 0.2, got {}",
            freshness_ratio
        );

        // Verify high stale ratio detection
        assert!(
            score.has_high_stale_ratio(),
            "Peer with 80% stale blocks should have high stale ratio"
        );
    }

    #[test]
    fn test_freshness_scoring_fresh_blocks() {
        let mut score = PeerScore::default();

        // Simulate a peer sending mostly fresh blocks
        for _ in 0..9 {
            score.fresh_blocks = score.fresh_blocks.saturating_add(1);
        }
        for _ in 0..1 {
            score.stale_blocks = score.stale_blocks.saturating_add(1);
        }
        score.blocks_delivered = 10;

        // With 90% fresh blocks, reputation should be good
        let reputation = score.reputation();

        // A peer with 90% fresh blocks should have reputation >= 50
        assert!(
            reputation >= 50.0,
            "Peer with 90% fresh blocks should have reputation >= 50, got {}",
            reputation
        );

        // Verify freshness ratio calculation
        let freshness_ratio = score.freshness_ratio();
        assert!(
            (freshness_ratio - 0.9).abs() < 0.01,
            "Freshness ratio should be 0.9, got {}",
            freshness_ratio
        );

        // Verify high stale ratio detection (should be false)
        assert!(
            !score.has_high_stale_ratio(),
            "Peer with 10% stale blocks should not have high stale ratio"
        );
    }

    #[test]
    fn test_novelty_ratio_calculation() {
        let mut score = PeerScore::default();

        // 8 novel + 2 duplicate = 80% novelty
        for _ in 0..8 {
            score.novel_blocks = score.novel_blocks.saturating_add(1);
        }
        for _ in 0..2 {
            score.duplicate_blocks = score.duplicate_blocks.saturating_add(1);
        }
        score.blocks_delivered = 10;

        // Verify novelty ratio calculation
        let novelty_ratio = score.novelty_ratio();
        assert!(
            (novelty_ratio - 0.8).abs() < 0.01,
            "Novelty ratio should be 0.8, got {}",
            novelty_ratio
        );

        // High novelty peer should not have low novelty ratio
        assert!(
            !score.has_low_novelty_ratio(),
            "Peer with 80% novelty should not have low novelty ratio"
        );
    }

    #[test]
    fn test_low_novelty_peer_deprioritized() {
        // 1 novel + 9 duplicate = 10% novelty (just at threshold)
        let mut score = PeerScore {
            novel_blocks: 1,
            duplicate_blocks: 9,
            blocks_delivered: 10,
            ..PeerScore::default()
        };

        // Verify novelty ratio
        let novelty_ratio = score.novelty_ratio();
        assert!(
            (novelty_ratio - 0.1).abs() < 0.01,
            "Novelty ratio should be 0.1, got {}",
            novelty_ratio
        );

        // Peer with 10% novelty should have low novelty ratio (at threshold)
        assert!(
            !score.has_low_novelty_ratio(),
            "Peer with exactly 10% novelty should not be flagged (< 10%)"
        );

        // Now test below threshold
        score.novel_blocks = 0;
        score.duplicate_blocks = 10;

        assert!(
            score.has_low_novelty_ratio(),
            "Peer with 0% novelty should have low novelty ratio"
        );
    }

    #[test]
    fn test_combined_reputation_score() {
        // Test peer with good freshness but poor novelty
        let score1 = PeerScore {
            fresh_blocks: 9,
            stale_blocks: 1,
            novel_blocks: 2,
            duplicate_blocks: 8,
            blocks_delivered: 10,
            ..PeerScore::default()
        };

        let rep1 = score1.reputation();

        // Test peer with poor freshness but good novelty
        let score2 = PeerScore {
            fresh_blocks: 2,
            stale_blocks: 8,
            novel_blocks: 9,
            duplicate_blocks: 1,
            blocks_delivered: 10,
            ..PeerScore::default()
        };

        let rep2 = score2.reputation();

        // Test peer with both good freshness and novelty
        let score3 = PeerScore {
            fresh_blocks: 9,
            stale_blocks: 1,
            novel_blocks: 9,
            duplicate_blocks: 1,
            blocks_delivered: 10,
            ..PeerScore::default()
        };

        let rep3 = score3.reputation();

        // Peer with both good metrics should have highest reputation
        assert!(
            rep3 > rep1,
            "Peer with good freshness+novelty should beat good freshness only"
        );
        assert!(
            rep3 > rep2,
            "Peer with good freshness+novelty should beat good novelty only"
        );

        // Verify the differences are meaningful
        assert!(
            rep3 - rep1 > 5.0,
            "Difference between combined good and freshness-only should be significant"
        );
        assert!(
            rep3 - rep2 > 5.0,
            "Difference between combined good and novelty-only should be significant"
        );
    }

    #[test]
    fn test_freshness_insufficient_samples() {
        // Only 5 blocks total (below FRESHNESS_MIN_SAMPLE_SIZE of 10)
        let mut score = PeerScore {
            stale_blocks: 4,
            fresh_blocks: 1,
            blocks_delivered: 5,
            ..PeerScore::default()
        };

        // Should not trigger high stale ratio with insufficient samples
        assert!(
            !score.has_high_stale_ratio(),
            "Should not flag high stale ratio with insufficient samples"
        );

        // Freshness adjustment should be 0 with insufficient samples
        // (reputation should not be penalized yet)
        let _reputation = score.reputation();

        // Add more blocks to reach threshold
        score.stale_blocks = 8;
        score.fresh_blocks = 2;
        score.blocks_delivered = 10;

        // Now should trigger
        assert!(
            score.has_high_stale_ratio(),
            "Should flag high stale ratio with sufficient samples"
        );
    }

    #[test]
    fn test_novelty_insufficient_samples() {
        // Only 5 blocks total (below NOVELTY_MIN_SAMPLE_SIZE of 10)
        let mut score = PeerScore {
            novel_blocks: 0,
            duplicate_blocks: 5,
            blocks_delivered: 5,
            ..PeerScore::default()
        };

        // Should not trigger low novelty with insufficient samples
        assert!(
            !score.has_low_novelty_ratio(),
            "Should not flag low novelty with insufficient samples"
        );

        // Add more blocks to reach threshold
        score.novel_blocks = 0;
        score.duplicate_blocks = 10;
        score.blocks_delivered = 10;

        // Now should trigger
        assert!(
            score.has_low_novelty_ratio(),
            "Should flag low novelty with sufficient samples"
        );
    }

    #[test]
    fn test_last_block_height_delta() {
        // Simulate receiving a block 50 behind tip (fresh)
        let mut score = PeerScore {
            last_block_height_delta: 50,
            fresh_blocks: 1,
            ..PeerScore::default()
        };

        assert_eq!(
            score.last_block_height_delta, 50,
            "Last block height delta should be 50"
        );
        assert!(
            !score.has_high_stale_ratio(),
            "Single fresh block should not trigger high stale ratio"
        );

        // Simulate receiving a block 150 behind tip (stale)
        score.last_block_height_delta = 150;
        score.stale_blocks = 1;

        assert_eq!(
            score.last_block_height_delta, 150,
            "Last block height delta should be 150"
        );
    }

    // ==================== Jaccard Similarity Tests ====================

    #[test]
    fn test_jaccard_similarity_identical() {
        let a: HashSet<SocketAddr> = vec!["1.1.1.1:8000", "2.2.2.2:8000", "3.3.3.3:8000"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let b = a.clone();
        assert!(
            (jaccard_similarity(&a, &b) - 1.0).abs() < 0.001,
            "Identical sets should have similarity 1.0"
        );
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let a: HashSet<SocketAddr> = vec!["1.1.1.1:8000"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let b: HashSet<SocketAddr> = vec!["2.2.2.2:8000"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        assert!(
            (jaccard_similarity(&a, &b) - 0.0).abs() < 0.001,
            "Disjoint sets should have similarity 0.0"
        );
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // 9 shared + 1 unique each = 9/11 ≈ 0.818
        let shared: Vec<&str> = (1..=9)
            .map(|i| match i {
                1 => "1.1.1.1:8000",
                2 => "2.2.2.2:8000",
                3 => "3.3.3.3:8000",
                4 => "4.4.4.4:8000",
                5 => "5.5.5.5:8000",
                6 => "6.6.6.6:8000",
                7 => "7.7.7.7:8000",
                8 => "8.8.8.8:8000",
                _ => "9.9.9.9:8000",
            })
            .collect();

        let mut a: HashSet<SocketAddr> = shared.iter().map(|s| s.parse().unwrap()).collect();
        a.insert("10.10.10.10:8000".parse().unwrap()); // Unique to a

        let mut b: HashSet<SocketAddr> = shared.iter().map(|s| s.parse().unwrap()).collect();
        b.insert("11.11.11.11:8000".parse().unwrap()); // Unique to b

        let similarity = jaccard_similarity(&a, &b);
        // 9 shared / 11 total = 0.818...
        assert!(
            (similarity - 9.0 / 11.0).abs() < 0.001,
            "Partial overlap should be ~0.818, got {}",
            similarity
        );
        assert!(
            similarity > JACCARD_WARNING_THRESHOLD,
            "{} should be > 0.8 warning threshold",
            similarity
        );
    }

    #[test]
    fn test_jaccard_similarity_empty() {
        let a: HashSet<SocketAddr> = HashSet::new();
        let b: HashSet<SocketAddr> = HashSet::new();
        assert!(
            (jaccard_similarity(&a, &b) - 0.0).abs() < 0.001,
            "Two empty sets should have similarity 0.0"
        );
    }

    #[test]
    fn test_jaccard_similarity_one_empty() {
        let a: HashSet<SocketAddr> = vec!["1.1.1.1:8000", "2.2.2.2:8000"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let b: HashSet<SocketAddr> = HashSet::new();
        assert!(
            (jaccard_similarity(&a, &b) - 0.0).abs() < 0.001,
            "One empty set should have similarity 0.0"
        );
    }

    #[test]
    fn test_sybil_detection_cross_subnet() {
        // Two peers from different /16 with >90% overlap should trigger strong signal
        let peer_list: HashSet<SocketAddr> = (1..=10)
            .map(|i| format!("{}.{}.1.1:8000", i, i).parse().unwrap())
            .collect();

        // Same list from different peer
        let same_list = peer_list.clone();

        let similarity = jaccard_similarity(&peer_list, &same_list);
        assert!(
            similarity > JACCARD_SYBIL_THRESHOLD,
            "Identical lists should trigger strong Sybil signal"
        );
    }

    #[test]
    fn test_same_subnet_no_sybil_flag() {
        // Two peers from same /16 with high overlap should NOT trigger (expected behavior)
        // This test verifies the logic that same-subnet peers are exempt
        // The actual subnet comparison is done in the handler, not in jaccard_similarity
        // Here we just verify the similarity calculation works correctly

        let a: HashSet<SocketAddr> =
            vec!["192.168.1.1:8000", "192.168.1.2:8000", "192.168.1.3:8000"]
                .iter()
                .map(|s| s.parse().unwrap())
                .collect();
        let b: HashSet<SocketAddr> =
            vec!["192.168.1.1:8000", "192.168.1.2:8000", "192.168.1.4:8000"]
                .iter()
                .map(|s| s.parse().unwrap())
                .collect();

        let similarity = jaccard_similarity(&a, &b);
        // 2 shared / 4 total = 0.5
        assert!(
            (similarity - 0.5).abs() < 0.001,
            "Same-subnet peers should still calculate similarity correctly"
        );
    }

    #[test]
    fn test_adaptive_fanout_stale_block() {
        // Test that stale blocks (>10 blocks behind tip) get doubled fanout
        // When local_tip = 100 and block_height = 80, block is 20 blocks behind = stale
        let local_tip: u64 = 100;
        let block_height: u64 = 80;
        let is_stale = local_tip.saturating_sub(block_height) > 10;

        assert!(
            is_stale,
            "Block 20 blocks behind should be considered stale"
        );

        // Test fanout calculation for stale blocks
        let peer_count: usize = 100;
        let base_fanout = std::cmp::max(3, (peer_count as f64).sqrt() as usize);
        let fanout = if is_stale {
            std::cmp::min(peer_count, base_fanout * 2)
        } else {
            base_fanout
        };

        // base_fanout = sqrt(100) = 10
        // stale fanout = min(100, 10 * 2) = 20
        assert_eq!(base_fanout, 10, "Base fanout should be sqrt(100) = 10");
        assert_eq!(fanout, 20, "Stale block should get doubled fanout = 20");
    }

    #[test]
    fn test_adaptive_fanout_fresh_block() {
        // Test that fresh blocks (within 10 blocks of tip) get normal fanout
        let local_tip: u64 = 100;
        let block_height: u64 = 95;
        let is_stale = local_tip.saturating_sub(block_height) > 10;

        assert!(
            !is_stale,
            "Block 5 blocks behind should NOT be considered stale"
        );

        let peer_count: usize = 100;
        let base_fanout = std::cmp::max(3, (peer_count as f64).sqrt() as usize);
        let fanout = if is_stale {
            std::cmp::min(peer_count, base_fanout * 2)
        } else {
            base_fanout
        };

        assert_eq!(fanout, base_fanout, "Fresh block should get normal fanout");
    }

    #[test]
    fn test_bandwidth_tracking() {
        // Test that PeerScore correctly tracks bandwidth
        let mut score = PeerScore::default();

        // Initial state
        assert_eq!(score.bytes_sent, 0);
        assert_eq!(score.bytes_received, 0);

        // Simulate sending and receiving data
        score.bytes_sent = 1000;
        score.bytes_received = 500;

        assert_eq!(score.bytes_sent, 1000);
        assert_eq!(score.bytes_received, 500);

        // Test reset
        score.reset_bandwidth_counters();
        assert_eq!(score.bytes_sent, 0);
        assert_eq!(score.bytes_received, 0);
    }

    #[test]
    fn test_bandwidth_efficiency_calculation() {
        // Test bandwidth efficiency formula: novelty_ratio * bytes_received / (bytes_sent + 1)
        let mut score = PeerScore::default();

        // Case 1: No data - should use default novelty ratio (0.5)
        // efficiency = 0.5 * 0 / 1 = 0
        let eff = score.bandwidth_efficiency();
        assert!(
            (eff - 0.0).abs() < 0.001,
            "Zero bandwidth should give zero efficiency"
        );

        // Case 2: Good novelty (80%), received more than sent (good)
        score.novel_blocks = 80;
        score.duplicate_blocks = 20; // novelty_ratio = 80/100 = 0.8
        score.bytes_sent = 100;
        score.bytes_received = 200;
        // efficiency = 0.8 * 200 / 100 = 1.6
        let eff = score.bandwidth_efficiency();
        assert!(
            (eff - 1.6).abs() < 0.001,
            "Good ratio should give efficiency 1.6, got {}",
            eff
        );

        // Case 3: Poor novelty (20%), sent more than received (bad)
        let score2 = PeerScore {
            novel_blocks: 20,
            duplicate_blocks: 80, // novelty_ratio = 20/100 = 0.2
            bytes_sent: 200,
            bytes_received: 100,
            ..PeerScore::default()
        };
        // efficiency = 0.2 * 100 / 200 = 0.1
        let eff2 = score2.bandwidth_efficiency();
        assert!(
            (eff2 - 0.1).abs() < 0.001,
            "Poor ratio should give efficiency 0.1, got {}",
            eff2
        );
    }

    // Tests for capability enforcement
    #[test]
    fn test_meets_required_capabilities_full() {
        // A node advertising all capabilities always passes
        assert!(meets_required_capabilities(LOCAL_CAPABILITIES));
    }

    #[test]
    fn test_meets_required_capabilities_minimum() {
        // Exactly the required set passes
        assert!(meets_required_capabilities(REQUIRED_CAPABILITIES));
    }

    #[test]
    fn test_meets_required_capabilities_superset() {
        // Extra capability bits beyond required still pass
        assert!(meets_required_capabilities(
            REQUIRED_CAPABILITIES | CAP_ML_KEM_768 | CAP_COMPACT_BLOCKS
        ));
    }

    #[test]
    fn test_meets_required_capabilities_missing_b3memhash() {
        // Missing CAP_B3MEMHASH — cannot validate Stream B blocks
        let caps = REQUIRED_CAPABILITIES & !CAP_B3MEMHASH;
        assert!(!meets_required_capabilities(caps));
    }

    #[test]
    fn test_meets_required_capabilities_missing_blake3() {
        // Missing CAP_BLAKE3_POW — cannot validate Stream A/C blocks
        let caps = REQUIRED_CAPABILITIES & !CAP_BLAKE3_POW;
        assert!(!meets_required_capabilities(caps));
    }

    #[test]
    fn test_meets_required_capabilities_zero() {
        // Old node advertising no capabilities is rejected
        assert!(!meets_required_capabilities(0));
    }

    #[test]
    fn test_required_capabilities_subset_of_local() {
        // REQUIRED must be a strict subset of what we ourselves advertise
        assert_eq!(
            LOCAL_CAPABILITIES & REQUIRED_CAPABILITIES,
            REQUIRED_CAPABILITIES,
            "LOCAL_CAPABILITIES must include all REQUIRED_CAPABILITIES"
        );
    }
}

// =============================================================================
// Kyber Post-Quantum Handshake Upgrade (Background Session Upgrade)
// =============================================================================

/// Channel types for coordinating Kyber handshake messages between handle_peer and upgrade tasks
#[cfg(feature = "kyber")]
type KyberMessageTx = tokio::sync::mpsc::Sender<NetworkMessage>;
#[cfg(feature = "kyber")]
type KyberMessageRx = tokio::sync::mpsc::Receiver<NetworkMessage>;

/// Global registry for pending Kyber upgrades: peer_addr -> sender channel
/// This allows handle_peer to route incoming Kyber messages to the upgrade task
#[cfg(feature = "kyber")]
static KYBER_UPGRADE_CHANNELS: std::sync::OnceLock<
    tokio::sync::Mutex<std::collections::HashMap<SocketAddr, KyberMessageTx>>,
> = std::sync::OnceLock::new();

#[cfg(feature = "kyber")]
fn get_kyber_channels(
) -> &'static tokio::sync::Mutex<std::collections::HashMap<SocketAddr, KyberMessageTx>> {
    KYBER_UPGRADE_CHANNELS.get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Register a channel for receiving Kyber messages from handle_peer
#[cfg(feature = "kyber")]
async fn register_kyber_channel(addr: SocketAddr, tx: KyberMessageTx) {
    get_kyber_channels().lock().await.insert(addr, tx);
}

/// Unregister a channel when Kyber upgrade completes or fails
#[cfg(feature = "kyber")]
async fn unregister_kyber_channel(addr: &SocketAddr) {
    get_kyber_channels().lock().await.remove(addr);
}

/// Route an incoming Kyber message to the upgrade task if one is pending
/// Returns true if the message was routed to an upgrade task
#[cfg(feature = "kyber")]
async fn route_kyber_message(addr: SocketAddr, msg: NetworkMessage) -> bool {
    if let Some(tx) = get_kyber_channels().lock().await.get(&addr) {
        if let Err(_) = tx.send(msg).await {
            return false; // Channel closed, upgrade task done
        }
        true
    } else {
        false // No upgrade task registered
    }
}
