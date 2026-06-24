//! Network synchronization and Initial Block Download (IBD)
//!
//! Implements headers-first sync for efficient chain synchronization.
//!
//! Strategy:
//! 1. Download block headers first (lightweight, fast validation)
//! 2. Validate PoW chain of headers
//! 3. Download full block data in parallel batches only after header validation
//!
//! Performance optimizations:
//! - Pre-allocated collections for better memory locality
//! - Cached timestamp calculations (avoid syscalls in loops)
//! - O(1) short ID lookup using HashMap
//! - Proper FIFO orphan eviction using VecDeque
//! - Reduced lock contention with batch operations

use crate::blockchain::{Block, BlockHeader, Blockchain};
use crate::types::{Hash, StreamType};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// SYNC-001: Used to ensure mining is always resumed, even on error
use scopeguard;

/// Minimum number of independent peers that must agree on a superior height
/// before triggering a full chain resync.
const RESYNC_QUORUM: usize = 3;

/// Get the effective resync quorum, allowing override via RESYNC_QUORUM env var.
/// Minimum value is 2 to prevent single-peer chain wipe attacks.
fn get_resync_quorum() -> usize {
    std::env::var("RESYNC_QUORUM")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.max(2)) // never allow below 2
        .unwrap_or(RESYNC_QUORUM)
}

/// Cooldown period between resyncs (30 minutes)
const RESYNC_COOLDOWN_SECS: u64 = 1800;
/// Last time a resync was triggered
static LAST_RESYNC_TIME: OnceLock<Mutex<u64>> = OnceLock::new();

/// Peer height attestations for multi-peer consensus before chain wipe.
/// Maps peer IP addresses to their reported chain heights.
static PEER_HEIGHT_ATTESTATIONS: OnceLock<Mutex<HashMap<IpAddr, u64>>> = OnceLock::new();

/// Check if enough independent peers attest to a height significantly above ours.
/// Records this peer's attestation and returns true only if quorum is met.
fn should_trigger_resync(peer_ip: IpAddr, peer_height: u64, local_height: u64) -> bool {
    // Enforce cooldown between resyncs
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last_resync = LAST_RESYNC_TIME.get_or_init(|| Mutex::new(0));
    {
        let last = last_resync.lock().unwrap_or_else(|e| e.into_inner());
        if now - *last < RESYNC_COOLDOWN_SECS {
            return false;
        }
    }

    // If peer isn't significantly ahead or we're at genesis, no need for resync
    if peer_height <= local_height + 50 || local_height == 0 {
        return false;
    }

    // Get or initialize the attestation map
    let attestations = PEER_HEIGHT_ATTESTATIONS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut attestations = attestations.lock().unwrap_or_else(|e| e.into_inner());
    attestations.insert(peer_ip, peer_height);

    // Count how many unique peers report height significantly above ours
    let agreeing_peers = attestations
        .values()
        .filter(|&&h| h > local_height + 50)
        .count();

    let quorum = get_resync_quorum();
    if agreeing_peers >= quorum {
        // Update last resync time
        if let Some(last_resync) = LAST_RESYNC_TIME.get() {
            let mut last = last_resync.lock().unwrap_or_else(|e| e.into_inner());
            *last = now;
        }
        // Clear attestations after triggering (one-shot)
        attestations.clear();
        true
    } else {
        warn!(peer = %peer_ip, peer_height = peer_height, local_height = local_height,
              peers_needed = quorum - agreeing_peers,
              "Waiting for more peers to confirm before resync");
        false
    }
}

/// Block header for fast sync (lightweight, no transaction data)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockHeaderSync {
    pub header: BlockHeader,
    pub hash: Hash,
}

/// Combined state for validated headers and requested blocks
///
/// Consolidated into a single struct to prevent deadlock from dual write locks.
struct ValidatedState {
    /// Headers validated and ready for block download
    headers: HashMap<Hash, BlockHeaderSync>,
    /// Blocks requested but not yet received (for tracking)
    requested: HashSet<Hash>,
}

/// Headers-first sync manager
pub struct HeadersFirstSync {
    /// Headers downloaded but not yet validated
    pending_headers: Arc<RwLock<VecDeque<BlockHeaderSync>>>,
    /// Combined validated headers and requested blocks state
    validated_state: Arc<RwLock<ValidatedState>>,
    /// Sync state
    sync_state: Arc<RwLock<SyncState>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    /// Not syncing
    Idle,
    /// Downloading headers
    DownloadingHeaders {
        start_height: u64,
        current_height: u64,
    },
    /// Validating headers
    ValidatingHeaders,
    // NOTE: DownloadingBlocks variant removed - was defined but never used in state transitions
    /// Sync complete
    Complete,
}

impl HeadersFirstSync {
    pub fn new() -> Self {
        // Pre-allocate with expected capacity for better memory locality
        Self {
            pending_headers: Arc::new(RwLock::new(VecDeque::with_capacity(1000))),
            validated_state: Arc::new(RwLock::new(ValidatedState {
                headers: HashMap::with_capacity(1000),
                requested: HashSet::with_capacity(1000),
            })),
            sync_state: Arc::new(RwLock::new(SyncState::Idle)),
        }
    }

    /// Start headers-first sync
    ///
    /// Returns the starting block number to request headers from
    pub async fn start_sync(&self, local_height: u64) -> u64 {
        let mut state = self.sync_state.write().await;
        *state = SyncState::DownloadingHeaders {
            start_height: local_height,
            current_height: local_height,
        };
        local_height
    }

    /// Add headers received from peer (optimized batch insert)
    pub async fn add_headers(&self, headers: Vec<BlockHeaderSync>) {
        if headers.is_empty() {
            return;
        }

        let mut pending = self.pending_headers.write().await;
        // Reserve capacity for batch insert
        pending.reserve(headers.len());
        for header in headers {
            pending.push_back(header);
        }
    }

    /// Validate pending headers (check PoW chain)
    ///
    /// Validates:
    /// 1. Header hash matches calculated hash
    /// 2. Header structure is valid (block number, difficulty, timestamp)
    /// 3. Basic PoW validation (header hash structure)
    ///
    /// Returns number of headers validated
    ///
    /// Note: Full PoW validation (with transactions root) is deferred until full blocks are downloaded,
    /// as we don't have transaction data during headers-first sync. Parent hash validation is also
    /// deferred until we have the full blocks.
    pub async fn validate_headers(&self) -> usize {
        let mut state = self.sync_state.write().await;
        *state = SyncState::ValidatingHeaders;
        drop(state);

        let mut pending = self.pending_headers.write().await;
        let mut validated_state = self.validated_state.write().await;

        // Pre-allocate capacity for validated headers
        validated_state.headers.reserve(pending.len());

        let mut validated_count = 0;
        let mut rejected_count = 0;

        // OPTIMIZATION: Cache current time to avoid syscall per iteration
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        while let Some(header_sync) = pending.pop_front() {
            // 1. Validate header hash matches calculated hash
            let calculated_hash = header_sync.header.calculate_header_hash();
            if calculated_hash != header_sync.hash {
                warn!(block_number = header_sync.header.block_number,
                      expected = ?calculated_hash, got = ?header_sync.hash,
                      "Header hash mismatch");
                rejected_count += 1;
                continue;
            }

            // 2. Basic structure validation
            // Check difficulty is reasonable (not zero, not impossibly high)
            if header_sync.header.difficulty == 0 {
                warn!(
                    block_number = header_sync.header.block_number,
                    "Header has zero difficulty"
                );
                rejected_count += 1;
                continue;
            }

            // Check timestamp is reasonable (not too far in future)
            // Allow 10 minutes in future (clock skew tolerance)
            if header_sync.header.timestamp > current_time + 600 {
                warn!(
                    block_number = header_sync.header.block_number,
                    timestamp = header_sync.header.timestamp,
                    current_time = current_time,
                    "Header timestamp too far in future"
                );
                rejected_count += 1;
                continue;
            }

            // Check block number is reasonable (not impossibly high)
            // This is a sanity check - actual validation happens with full blocks
            if header_sync.header.block_number > 1_000_000_000 {
                warn!(
                    block_number = header_sync.header.block_number,
                    "Header block number unreasonably high"
                );
                rejected_count += 1;
                continue;
            }

            // 3. Validate nonce is present (for PoW streams)
            // Stream C doesn't use PoW, so nonce can be 0
            if header_sync.header.stream_type != StreamType::StreamC
                && header_sync.header.nonce == 0
            {
                // Nonce of 0 is valid if difficulty is very low, but we'll allow it
                // Full PoW validation will happen when we have the transactions root
            }

            // Note: Full PoW validation (checking if hash meets difficulty with transactions root)
            // will be performed when full blocks are downloaded. At this stage, we only validate
            // the header structure and hash integrity.

            // OPTIMIZATION: Move instead of clone (header_sync is consumed)
            validated_state
                .headers
                .insert(header_sync.hash, header_sync);
            validated_count += 1;
        }

        if rejected_count > 0 {
            warn!(
                rejected = rejected_count,
                validated = validated_count,
                "Rejected invalid headers"
            );
        } else if validated_count > 0 {
            info!(count = validated_count, "Validated headers");
        }

        validated_count
    }

    /// Get headers ready for block download
    ///
    /// Returns up to `batch_size` header hashes that need full blocks
    pub async fn get_headers_for_download(&self, batch_size: usize) -> Vec<Hash> {
        let validated_state = self.validated_state.read().await;
        let mut hashes: Vec<Hash> = validated_state
            .headers
            .keys()
            .take(batch_size)
            .copied()
            .collect();
        hashes.sort(); // Deterministic ordering
        hashes
    }

    /// Mark block as downloaded (optimized - single lock scope)
    pub async fn mark_block_downloaded(&self, hash: Hash) {
        let (validated_empty, requested_empty) = {
            let mut state = self.validated_state.write().await;
            state.headers.remove(&hash);
            state.requested.remove(&hash);
            (state.headers.is_empty(), state.requested.is_empty())
        };

        // Update sync state outside lock scope
        let mut state = self.sync_state.write().await;
        if validated_empty && requested_empty {
            *state = SyncState::Complete;
        }
    }

    /// Get current sync state
    pub async fn get_sync_state(&self) -> SyncState {
        self.sync_state.read().await.clone()
    }

    /// Check if sync is complete
    pub async fn is_complete(&self) -> bool {
        let state = self.sync_state.read().await;
        matches!(*state, SyncState::Complete)
    }
}

/// Compact block representation (BIP 152 style)
///
/// Instead of sending full block [Header + 1000 Txs],
/// send [Header + ShortIDs]. Receiving node reconstructs
/// block from mempool if it has the transactions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompactBlock {
    /// Full block header
    pub header: BlockHeader,
    /// Short transaction IDs (6 bytes each, instead of 32-byte hashes)
    pub short_ids: Vec<[u8; 6]>,
    /// Full transactions that are NOT in mempool (prefilled)
    pub prefilled_txs: Vec<crate::blockchain::Transaction>,
    /// Nonce for SipHash short ID generation (prevents collision attacks)
    pub nonce: u64,
}

/// Generate 6-byte short ID from transaction hash using SipHash13
#[inline]
fn generate_short_id(tx_hash: &Hash, nonce: u64) -> [u8; 6] {
    use siphasher::sip::SipHasher13;
    use std::hash::Hasher;

    let mut hasher = SipHasher13::new_with_keys(nonce, nonce.wrapping_add(1));
    hasher.write(tx_hash.as_ref());
    let short_hash = hasher.finish();
    let bytes = short_hash.to_le_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]]
}

impl CompactBlock {
    /// Create compact block from full block
    ///
    /// Uses SipHash to generate 6-byte short IDs from transaction hashes
    pub fn from_block(block: &Block, mempool_hashes: &HashSet<Hash>) -> Self {
        use rand::Rng;

        let mut short_ids = Vec::with_capacity(block.transactions.len());
        let mut prefilled_txs = Vec::new();
        let nonce: u64 = rand::thread_rng().gen();

        for tx in &block.transactions {
            if mempool_hashes.contains(&tx.hash) {
                // Transaction is in mempool - use short ID
                short_ids.push(generate_short_id(&tx.hash, nonce));
            } else {
                // Transaction not in mempool - include full transaction
                prefilled_txs.push(tx.clone());
            }
        }

        Self {
            header: block.header.clone(),
            short_ids,
            prefilled_txs,
            nonce,
        }
    }

    /// Reconstruct full block from compact block (O(1) short ID lookup)
    ///
    /// Requires mempool to match short IDs to full transactions
    /// Returns (Option<Block>, missing_short_ids) where missing_short_ids contains
    /// the short IDs that couldn't be matched to mempool transactions
    pub fn to_block(
        &self,
        mempool: &HashMap<Hash, crate::blockchain::Transaction>,
    ) -> (Option<Block>, Vec<[u8; 6]>) {
        // OPTIMIZATION: Build short ID -> tx_hash map once (O(n) build, O(1) lookup)
        let short_id_map: HashMap<[u8; 6], &crate::blockchain::Transaction> = mempool
            .iter()
            .map(|(hash, tx)| (generate_short_id(hash, self.nonce), tx))
            .collect();

        let mut transactions = Vec::with_capacity(self.prefilled_txs.len() + self.short_ids.len());
        let mut missing_short_ids = Vec::new();

        // Add prefilled transactions
        transactions.extend(self.prefilled_txs.clone());

        // Match short IDs to mempool transactions (O(1) lookup per ID)
        for short_id in &self.short_ids {
            if let Some(tx) = short_id_map.get(short_id) {
                transactions.push((*tx).clone());
            } else {
                // Short ID not found in mempool - track for GetMissingTransactions
                missing_short_ids.push(*short_id);
            }
        }

        // If we have missing transactions, return None with the missing short IDs
        if !missing_short_ids.is_empty() {
            return (None, missing_short_ids);
        }

        // Reconstruct block
        (
            Some(Block::new(self.header.clone(), transactions)),
            Vec::new(),
        )
    }

    /// Get the block hash for this compact block
    pub fn block_hash(&self) -> Hash {
        self.header.calculate_header_hash()
    }
}

/// Find transactions matching the given short IDs from mempool and recent blocks
/// Returns a map of short_id -> transaction for all found transactions
pub fn find_transactions_by_short_ids(
    mempool: &HashMap<Hash, crate::blockchain::Transaction>,
    short_ids: &[[u8; 6]],
    nonce: u64,
) -> HashMap<[u8; 6], crate::blockchain::Transaction> {
    let mut result = HashMap::new();

    // Build short ID -> tx map from mempool
    for (hash, tx) in mempool.iter() {
        let short_id = generate_short_id(hash, nonce);
        if short_ids.contains(&short_id) {
            result.insert(short_id, tx.clone());
        }
    }

    result
}

/// Combined state for orphan blocks
///
/// Consolidated into a single struct to prevent deadlock from dual write locks.
struct OrphanState {
    /// Orphan blocks waiting for parents (block_hash -> block)
    orphans: HashMap<Hash, Block>,
    /// Orphan blocks in FIFO order (for proper eviction)
    queue: VecDeque<Hash>,
}

/// Orphan pool for out-of-order BlockDAG blocks
///
/// In a BlockDAG, blocks arrive out of order constantly.
/// This pool holds blocks that can't be processed yet (missing parents).
pub struct OrphanPool {
    /// Combined orphan blocks state
    orphan_state: Arc<RwLock<OrphanState>>,
    /// Blocks we're actively requesting from peers (block_hash -> peer_addresses)
    requested: Arc<RwLock<HashMap<Hash, Vec<std::net::SocketAddr>>>>,
    /// Maximum orphan pool size (DoS protection)
    max_orphans: usize,
}

impl OrphanPool {
    pub fn new(max_orphans: usize) -> Self {
        Self {
            orphan_state: Arc::new(RwLock::new(OrphanState {
                orphans: HashMap::with_capacity(max_orphans),
                queue: VecDeque::with_capacity(max_orphans),
            })),
            requested: Arc::new(RwLock::new(HashMap::with_capacity(max_orphans / 2))),
            max_orphans,
        }
    }

    /// Add orphan block (missing parents) - proper FIFO eviction
    pub async fn add_orphan(&self, block: Block) -> Vec<Hash> {
        let hash = block.hash;
        let parent_hashes = block.header.parent_hashes.clone();

        let mut state = self.orphan_state.write().await;

        // Skip if already in pool
        if state.orphans.contains_key(&hash) {
            return parent_hashes;
        }

        // Enforce size limit with proper FIFO eviction
        while state.orphans.len() >= self.max_orphans {
            if let Some(oldest_hash) = state.queue.pop_front() {
                state.orphans.remove(&oldest_hash);
            } else {
                break;
            }
        }

        // Add to both map and queue
        state.orphans.insert(hash, block);
        state.queue.push_back(hash);

        parent_hashes
    }

    /// Check if block is an orphan
    pub async fn is_orphan(&self, hash: &Hash) -> bool {
        let state = self.orphan_state.read().await;
        state.orphans.contains_key(hash)
    }

    /// Try to process orphan (if parents are now available)
    ///
    /// Returns the block if it can be processed, None if still orphaned
    pub async fn try_process_orphan(&self, hash: &Hash) -> Option<Block> {
        let mut state = self.orphan_state.write().await;

        if let Some(block) = state.orphans.remove(hash) {
            // Remove from queue (maintain consistency)
            state.queue.retain(|h| h != hash);
            Some(block)
        } else {
            None
        }
    }

    /// Get all orphan hashes (for requesting missing parents)
    pub async fn get_orphan_hashes(&self) -> Vec<Hash> {
        let state = self.orphan_state.read().await;
        state.orphans.keys().copied().collect()
    }

    /// Mark that we're requesting a block from peers
    pub async fn mark_requested(&self, hash: Hash, peer: std::net::SocketAddr) {
        let mut requested = self.requested.write().await;
        requested.entry(hash).or_insert_with(Vec::new).push(peer);
    }

    /// Remove from requested list (block received)
    pub async fn unmark_requested(&self, hash: &Hash) {
        let mut requested = self.requested.write().await;
        requested.remove(hash);
    }

    /// Get orphan count
    pub async fn orphan_count(&self) -> usize {
        let state = self.orphan_state.read().await;
        state.orphans.len()
    }
}

// =============================================================================
// DEDICATED SYNC PROTOCOL (IBD - Initial Block Download)
// =============================================================================
//
// SECURITY FEATURES:
// 1. Block integrity validation - PoW and hash verification before acceptance
// 2. Response signing - Ed25519 signatures on sync responses
// 3. Rate limiting - Max requests per second per IP
// 4. Connection timeout - Prevents slow-loris attacks
// 5. Size limits - DoS protection against oversized responses
//
// Protocol format (v2 - authenticated):
// - Magic: "MSHWSYNC" (8 bytes)
// - Version: u8 (2 = authenticated)
// - Request type: u8 (0=GetHeight, 1=GetBlocks)
// - Payload: varies by request type
//
// Response format (authenticated):
// - Data payload
// - Signature (64 bytes Ed25519)
// - Public key (32 bytes)

use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// QUIC sync stream type constant (must match quic_transport.rs)
const STREAM_TYPE_SYNC: u8 = 0x02;

/// Sync protocol magic bytes
pub const SYNC_MAGIC: &[u8; 8] = b"MSHWSYNC";
/// Sync protocol version (2 = authenticated)
pub const SYNC_VERSION: u8 = 2;
/// Maximum sync response size (5MB pre-auth limit to prevent memory exhaustion)
pub const MAX_SYNC_RESPONSE_SIZE: u64 = 5 * 1024 * 1024;
/// Sync timeout in seconds (2 minutes for large block transfers)
pub const SYNC_TIMEOUT_SECS: u64 = 120;
/// Maximum blocks per request
pub const MAX_BLOCKS_PER_REQUEST: u64 = 500;

/// Maximum blocks per sync batch. Clients request multiple batches sequentially.
const MAX_SYNC_PRIMARY_BLOCKS: usize = 128;

/// Maximum concurrent sync connections (global limit)
const MAX_CONCURRENT_SYNC_CONNECTIONS: usize = 50;
/// Maximum connections per IP address
const MAX_CONNECTIONS_PER_IP: usize = 3;

// =============================================================================
// TRANSPORT ABSTRACTION FOR SYNC OPERATIONS
// =============================================================================
//
// Provides a transport-agnostic interface for sync operations, allowing the
// same sync logic to work with both QUIC and TCP transports.
//

/// Transport-agnostic sync operations trait.
///
/// This trait abstracts the transport-specific details (QUIC vs TCP) so that
/// the core sync logic can be shared between both implementations.
#[async_trait]
pub trait SyncTransportOps: Send + Sync {
    /// Get the peer's current block height.
    async fn get_peer_height(
        &self,
        expected_pubkey: &[u8; 32],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;

    /// Download blocks from the peer.
    async fn download_blocks(
        &self,
        from_block: u64,
        count: u64,
        expected_pubkey: &[u8; 32],
    ) -> Result<Vec<Block>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get a description of the peer for logging purposes.
    fn peer_description(&self) -> String;

    /// Get the peer's IP address for fork detection quorum.
    fn peer_ip(&self) -> IpAddr;
}

/// QUIC transport implementation for sync operations.
pub struct QuicTransport<'a> {
    connection: &'a quinn::Connection,
}

impl<'a> QuicTransport<'a> {
    pub fn new(connection: &'a quinn::Connection) -> Self {
        Self { connection }
    }
}

/// TCP transport implementation for sync operations.
pub struct TcpTransport {
    peer_addr: SocketAddr,
}

impl TcpTransport {
    pub fn new(peer_addr: SocketAddr) -> Self {
        Self { peer_addr }
    }
}

#[async_trait]
impl<'a> SyncTransportOps for QuicTransport<'a> {
    async fn get_peer_height(
        &self,
        expected_pubkey: &[u8; 32],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        // Open a bidirectional stream for sync
        let (mut send_stream, mut recv_stream) =
            tokio::time::timeout(Duration::from_secs(10), self.connection.open_bi()).await??;

        // Write stream type byte for sync
        send_stream.write_all(&[STREAM_TYPE_SYNC]).await?;

        // Send GetHeight request (v2 authenticated)
        let mut request = Vec::with_capacity(10);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION); // v2 = authenticated
        request.push(SyncRequestType::GetHeight as u8);
        send_stream.write_all(&request).await?;
        send_stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len > 1024 {
            return Err("Height response too large".into());
        }

        // Read and verify signed response
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on height response - possible MITM attack".into());
        }

        if response.data.len() != 8 {
            return Err("Invalid height response length".into());
        }

        let height = u64::from_le_bytes(response.data[0..8].try_into().unwrap());
        info!(height = height, "Verified height from peer");

        Ok(height)
    }

    async fn download_blocks(
        &self,
        from_block: u64,
        count: u64,
        expected_pubkey: &[u8; 32],
    ) -> Result<Vec<Block>, Box<dyn std::error::Error + Send + Sync>> {
        // Open a bidirectional stream for sync
        let (mut send_stream, mut recv_stream) =
            tokio::time::timeout(Duration::from_secs(10), self.connection.open_bi()).await??;

        // Write stream type byte for sync
        send_stream.write_all(&[STREAM_TYPE_SYNC]).await?;

        // Cap request size
        let count = std::cmp::min(count, MAX_BLOCKS_PER_REQUEST);

        // Send GetBlocks request (v2 authenticated)
        let mut request = Vec::with_capacity(26);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION);
        request.push(SyncRequestType::GetBlocks as u8);
        request.extend_from_slice(&from_block.to_le_bytes());
        request.extend_from_slice(&count.to_le_bytes());
        send_stream.write_all(&request).await?;
        send_stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len as u64 > MAX_SYNC_RESPONSE_SIZE {
            return Err(format!("Response too large ({} bytes)", response_len).into());
        }

        // Read signed response with timeout
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        // CRITICAL: Verify signature before trusting data
        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on blocks response - possible MITM attack".into());
        }

        info!("Response signature verified");

        // Parse block count and blocks from signed payload
        if response.data.len() < 8 {
            return Err("Response payload too small".into());
        }

        let block_count = u64::from_le_bytes(response.data[0..8].try_into().unwrap());
        let blocks_data = &response.data[8..];

        // Deserialize blocks
        let blocks: Vec<Block> = bincode::deserialize(blocks_data)?;

        if blocks.len() != block_count as usize {
            return Err(format!(
                "Block count mismatch: expected {}, got {}",
                block_count,
                blocks.len()
            )
            .into());
        }

        // CRITICAL: Validate each block's integrity
        let mut validated_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            // Verify block hash matches calculated hash
            let calculated_hash = block.calculate_hash();
            if calculated_hash != block.hash {
                error!(block_number = block.header.block_number,
                       calculated = ?calculated_hash, claimed = ?block.hash,
                       "Block hash mismatch - rejecting");
                continue;
            }

            // Verify PoW (hash meets difficulty target)
            if !verify_block_pow(&block) {
                error!(
                    block_number = block.header.block_number,
                    "Block failed PoW verification - rejecting"
                );
                continue;
            }

            validated_blocks.push(block);
        }

        info!(
            validated = validated_blocks.len(),
            total = block_count,
            "Validated blocks"
        );

        Ok(validated_blocks)
    }

    fn peer_description(&self) -> String {
        format!("{}", self.connection.remote_address())
    }

    fn peer_ip(&self) -> IpAddr {
        self.connection.remote_address().ip()
    }
}

#[async_trait]
impl SyncTransportOps for TcpTransport {
    async fn get_peer_height(
        &self,
        expected_pubkey: &[u8; 32],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        // Connect to sync port with timeout
        let sync_addr = SocketAddr::new(
            self.peer_addr.ip(),
            self.peer_addr
                .port()
                .checked_add(1)
                .ok_or("Peer port 65535 is not supported (sync port would overflow)")?,
        );
        let stream = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(sync_addr),
        )
        .await??;

        let mut stream = stream;

        // Send GetHeight request (v2 authenticated)
        let mut request = Vec::with_capacity(10);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION); // v2 = authenticated
        request.push(SyncRequestType::GetHeight as u8);
        stream.write_all(&request).await?;
        stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await?;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len > 1024 {
            return Err("Height response too large".into());
        }

        // Read and verify signed response
        let mut response_bytes = vec![0u8; response_len];
        stream.read_exact(&mut response_bytes).await?;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on height response - possible MITM attack".into());
        }

        if response.data.len() != 8 {
            return Err("Invalid height response length".into());
        }

        let height = u64::from_le_bytes(response.data[0..8].try_into().unwrap());
        info!(height = height, "Verified height from peer");

        Ok(height)
    }

    async fn download_blocks(
        &self,
        from_block: u64,
        count: u64,
        expected_pubkey: &[u8; 32],
    ) -> Result<Vec<Block>, Box<dyn std::error::Error + Send + Sync>> {
        // Connect to sync port with timeout
        let sync_addr = SocketAddr::new(
            self.peer_addr.ip(),
            self.peer_addr
                .port()
                .checked_add(1)
                .ok_or("Peer port 65535 is not supported (sync port would overflow)")?,
        );
        let stream = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(sync_addr),
        )
        .await??;

        let mut stream = stream;

        // Cap request size
        let count = std::cmp::min(count, MAX_BLOCKS_PER_REQUEST);

        // Send GetBlocks request (v2 authenticated)
        let mut request = Vec::with_capacity(26);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION);
        request.push(SyncRequestType::GetBlocks as u8);
        request.extend_from_slice(&from_block.to_le_bytes());
        request.extend_from_slice(&count.to_le_bytes());
        stream.write_all(&request).await?;
        stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len as u64 > MAX_SYNC_RESPONSE_SIZE {
            return Err(format!("Response too large ({} bytes)", response_len).into());
        }

        // Read signed response with timeout
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        // CRITICAL: Verify signature before trusting data
        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on blocks response - possible MITM attack".into());
        }

        info!("Response signature verified");

        // Parse block count and blocks from signed payload
        if response.data.len() < 8 {
            return Err("Response payload too small".into());
        }

        let block_count = u64::from_le_bytes(response.data[0..8].try_into().unwrap());
        let blocks_data = &response.data[8..];

        // Deserialize blocks
        let blocks: Vec<Block> = bincode::deserialize(blocks_data)?;

        if blocks.len() != block_count as usize {
            return Err(format!(
                "Block count mismatch: expected {}, got {}",
                block_count,
                blocks.len()
            )
            .into());
        }

        // CRITICAL: Validate each block's integrity
        let mut validated_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            // Verify block hash matches calculated hash
            let calculated_hash = block.calculate_hash();
            if calculated_hash != block.hash {
                error!(block_number = block.header.block_number,
                       calculated = ?calculated_hash, claimed = ?block.hash,
                       "Block hash mismatch - rejecting");
                continue;
            }

            // Verify PoW (hash meets difficulty target)
            if !verify_block_pow(&block) {
                error!(
                    block_number = block.header.block_number,
                    "Block failed PoW verification - rejecting"
                );
                continue;
            }

            validated_blocks.push(block);
        }

        info!(
            validated = validated_blocks.len(),
            total = block_count,
            "Validated blocks"
        );

        Ok(validated_blocks)
    }

    fn peer_description(&self) -> String {
        format!("{}", self.peer_addr)
    }

    fn peer_ip(&self) -> IpAddr {
        self.peer_addr.ip()
    }
}

/// Build a sync `GetBlocks` batch: sort by `block_number` (not storage order), take the
/// first blocks with `number >= from_block`.
///
/// CRITICAL: This function queries persistent storage (sled) for blocks, not just the in-memory cache.
/// This allows fresh nodes to sync ALL blocks from genesis, even if the in-memory Vec only holds
/// recent blocks (~2300). The sled database is the source of truth for all mined blocks.
///
/// NOTE: Transitive parents are NOT included. When a node syncs sequentially (from_block=0, then 128,
/// then 255, etc.), it already has parent blocks from previous batches. Including all ancestors
/// would create a snowball effect where batch size grows linearly with block number.
/// The client-side multi-pass topological insertion handles missing parents by retrying on the next sync cycle.
pub fn select_blocks_for_sync_batch(bc: &Blockchain, from_block: u64, count: u64) -> Vec<Block> {
    use std::collections::HashSet;
    let take_n = (count as usize)
        .min(MAX_BLOCKS_PER_REQUEST as usize)
        .min(MAX_SYNC_PRIMARY_BLOCKS);

    // Merge blocks from BOTH sled storage AND in-memory Vec.
    // - Sled has historical blocks persisted across restarts
    // - In-memory has ALL blocks from the current session (including ones not yet in sled)
    // Merging both sources covers all cases:
    //   Fresh node: in-memory has everything
    //   Long-running node: sled has history, in-memory has recent
    //   Sled gaps: in-memory fills them for the current session
    let mut seen_hashes: HashSet<[u8; 32]> = HashSet::new();
    let mut all_blocks: Vec<Block> = Vec::new();

    // 1. Get blocks from sled storage
    let storage_blocks = bc.get_blocks_from_storage(from_block, take_n * 2);
    for block in storage_blocks {
        if block.header.block_number >= from_block && seen_hashes.insert(block.hash.0) {
            all_blocks.push(block);
        }
    }

    // 2. Get blocks from in-memory Vec (fills gaps that sled might miss)
    bc.with_blocks(|blocks| {
        for block in blocks {
            if block.header.block_number >= from_block && seen_hashes.insert(block.hash.0) {
                all_blocks.push(block.clone());
            }
        }
    });

    // Sort by block_number
    all_blocks.sort_by_key(|b| b.header.block_number);

    // Filter to only chain-continuous blocks.
    // When from_block is far below the earliest block we actually have (peer
    // has pruned old history), treat the first available block as a snapshot
    // boundary so we can serve our actual chain instead of returning empty.
    let earliest_available = all_blocks
        .first()
        .map(|b| b.header.block_number)
        .unwrap_or(from_block);
    let pruned_gap = earliest_available > from_block.saturating_add(100);
    // Skip the snapshot-boundary logic when there is no pruning gap.
    let mut first_above_from_seen = !pruned_gap;
    let mut valid_hashes: HashSet<[u8; 32]> = HashSet::new();

    let filtered: Vec<Block> = all_blocks
        .into_iter()
        .filter(|block| {
            if block.header.block_number == 0 {
                valid_hashes.insert(block.hash.0);
                true
            } else if block.header.block_number < from_block {
                valid_hashes.insert(block.hash.0);
                true
            } else {
                // Treat the standard from_block boundary OR — when serving
                // from a pruned range — the very first block served as a
                // boundary whose parents the requester will accept as a
                // trusted snapshot anchor.
                let is_boundary_block = block.header.block_number == from_block
                    || (!first_above_from_seen && {
                        first_above_from_seen = true;
                        true
                    });
                let has_valid_parent = is_boundary_block
                    || block
                        .header
                        .parent_hashes
                        .iter()
                        .any(|parent_hash| valid_hashes.contains(&parent_hash.0));

                if has_valid_parent {
                    valid_hashes.insert(block.hash.0);
                    true
                } else {
                    false
                }
            }
        })
        .collect();

    // Take first N blocks after filtering
    let all_blocks: Vec<Block> = filtered.into_iter().take(take_n).collect();

    // Debug: log the batch being served
    if let (Some(first), Some(last)) = (all_blocks.first(), all_blocks.last()) {
        info!(
            from_block = from_block,
            count = count,
            take_n = take_n,
            blocks_served = all_blocks.len(),
            first_block = first.header.block_number,
            last_block = last.header.block_number,
            "Serving sync batch"
        );
    } else {
        info!(
            from_block = from_block,
            count = count,
            blocks_served = all_blocks.len(),
            "Serving sync batch (no blocks found)"
        );
    }

    all_blocks
}

/// Sync request types
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum SyncRequestType {
    GetHeight = 0,
    GetBlocks = 1,
}

/// Signed sync response
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedSyncResponse {
    /// Response data (serialized)
    pub data: Vec<u8>,
    /// Ed25519 signature over data (64 bytes)
    pub signature: Vec<u8>,
    /// Signer's public key (32 bytes)
    pub public_key: Vec<u8>,
    /// Timestamp (Unix epoch) - for replay protection
    pub timestamp: u64,
}

impl SignedSyncResponse {
    /// Create and sign a response
    pub fn sign(data: Vec<u8>, secret_key: &[u8; 32]) -> Self {
        use ed25519_dalek::{Signer, SigningKey};
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Sign: data || timestamp
        let mut to_sign = data.clone();
        to_sign.extend_from_slice(&timestamp.to_le_bytes());

        let signing_key = SigningKey::from_bytes(secret_key);
        let signature = signing_key.sign(&to_sign);
        let public_key = signing_key.verifying_key().to_bytes();

        Self {
            data,
            signature: signature.to_bytes().to_vec(),
            public_key: public_key.to_vec(),
            timestamp,
        }
    }

    /// Verify signature and check timestamp freshness
    /// SECURITY: The expected_pubkey parameter pins the peer's identity to prevent
    /// self-signing attacks where an attacker could sign with their own key.
    pub fn verify(&self, expected_pubkey: &[u8; 32]) -> bool {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        use std::time::{SystemTime, UNIX_EPOCH};

        // SECURITY: First check that the response's public key matches the expected peer key
        // This prevents an attacker from self-signing with their own key
        if self.public_key.len() != 32 || self.public_key.as_slice() != expected_pubkey.as_slice() {
            warn!("Public key mismatch in signed response - possible MITM attack");
            return false;
        }

        // Check timestamp freshness (must be within 5 minutes)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if self.timestamp > now + 300 || self.timestamp < now.saturating_sub(300) {
            warn!(
                timestamp = self.timestamp,
                now = now,
                "Response timestamp out of range"
            );
            return false;
        }

        // Convert public key Vec to array
        let Ok(pk_array): Result<[u8; 32], _> = self.public_key.clone().try_into() else {
            warn!(key_len = self.public_key.len(), "Invalid public key length");
            return false;
        };

        // Verify signature
        let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
            warn!("Invalid public key");
            return false;
        };

        // Convert signature Vec to array
        let Ok(sig_array): Result<[u8; 64], _> = self.signature.clone().try_into() else {
            warn!(sig_len = self.signature.len(), "Invalid signature length");
            return false;
        };

        let signature = Signature::from_bytes(&sig_array);

        // Reconstruct signed data: data || timestamp
        let mut to_verify = self.data.clone();
        to_verify.extend_from_slice(&self.timestamp.to_le_bytes());

        verifying_key.verify(&to_verify, &signature).is_ok()
    }
}

/// Dedicated sync server - runs on P2P port + 1
pub struct DedicatedSyncServer {
    blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
    listen_port: u16,
    is_running: Arc<tokio::sync::RwLock<bool>>,
    /// Node's signing key for response authentication
    secret_key: [u8; 32],
}

impl DedicatedSyncServer {
    pub fn new(
        blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
        p2p_port: u16,
    ) -> Self {
        // Generate ephemeral signing key if not provided
        // In production, this should be the node's persistent identity key
        let mut secret_key = [0u8; 32];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut secret_key);

        Self {
            blockchain,
            listen_port: p2p_port
                .checked_add(1)
                .expect("P2P port 65535 is not supported (sync port would overflow)"), // Sync port = P2P port + 1
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
            secret_key,
        }
    }

    /// Create with specific signing key (for persistent identity)
    pub fn with_key(
        blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
        p2p_port: u16,
        secret_key: [u8; 32],
    ) -> Self {
        Self {
            blockchain,
            listen_port: p2p_port
                .checked_add(1)
                .expect("P2P port 65535 is not supported (sync port would overflow)"),
            is_running: Arc::new(tokio::sync::RwLock::new(false)),
            secret_key,
        }
    }

    /// Start the dedicated sync server
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = format!("0.0.0.0:{}", self.listen_port);
        let listener = TcpListener::bind(&addr).await?;

        info!(port = self.listen_port, "Dedicated sync server listening");
        info!("Sync responses will be Ed25519 signed");

        *self.is_running.write().await = true;

        let blockchain = self.blockchain.clone();
        let is_running = self.is_running.clone();
        let secret_key = self.secret_key;

        // Connection limiters for rate limiting
        let connection_semaphore =
            Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SYNC_CONNECTIONS));
        let per_ip_counts: Arc<std::sync::Mutex<HashMap<std::net::IpAddr, usize>>> =
            Arc::new(std::sync::Mutex::new(HashMap::new()));

        tokio::spawn(async move {
            while *is_running.read().await {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        // Global connection limit
                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(limit = MAX_CONCURRENT_SYNC_CONNECTIONS, peer = %addr,
                                      "Global connection limit reached, rejecting");
                                continue;
                            }
                        };

                        // Per-IP limit
                        let ip = addr.ip();
                        {
                            let mut counts =
                                per_ip_counts.lock().unwrap_or_else(|e| e.into_inner());
                            let count = counts.entry(ip).or_insert(0);
                            if *count >= MAX_CONNECTIONS_PER_IP {
                                warn!(peer_ip = %ip, limit = MAX_CONNECTIONS_PER_IP,
                                      "Too many connections from peer");
                                continue;
                            }
                            *count += 1;
                        }

                        info!(peer = %addr, "Incoming sync connection");
                        let bc = blockchain.clone();
                        let key = secret_key;
                        let per_ip_cleanup = per_ip_counts.clone();
                        let client_ip = ip;

                        tokio::spawn(async move {
                            // permit is moved in — dropped automatically when task ends
                            let _permit = permit;

                            // Apply timeout to entire connection
                            let result = tokio::time::timeout(
                                Duration::from_secs(SYNC_TIMEOUT_SECS),
                                handle_sync_client_secure(stream, addr, bc, key),
                            )
                            .await;

                            // Decrement per-IP count on exit
                            {
                                let mut counts =
                                    per_ip_cleanup.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(count) = counts.get_mut(&client_ip) {
                                    *count = count.saturating_sub(1);
                                    if *count == 0 {
                                        counts.remove(&client_ip);
                                    }
                                }
                            }

                            match result {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    warn!(peer = %addr, error = %e, "Error handling sync client")
                                }
                                Err(_) => warn!(peer = %addr, "Timeout handling sync client"),
                            }
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "Accept error");
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the sync server
    pub async fn stop(&self) {
        *self.is_running.write().await = false;
    }

    /// Get sync port
    pub fn port(&self) -> u16 {
        self.listen_port
    }
}

/// Handle a sync client connection (server side) - SECURE VERSION
async fn handle_sync_client_secure(
    mut stream: tokio::net::TcpStream,
    addr: SocketAddr,
    blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
    secret_key: [u8; 32],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read magic + version + request type
    let mut header = [0u8; 10];
    stream.read_exact(&mut header).await?;

    // Verify magic
    if &header[0..8] != SYNC_MAGIC {
        warn!(peer = %addr, "Invalid magic from peer - possible attack");
        return Ok(());
    }

    let version = header[8];
    let request_type = header[9];

    // Only accept v2+ (authenticated) clients - v1 is deprecated and insecure
    if version < 2 {
        warn!(peer = %addr, version = version, "Rejected connection with unsupported protocol version");
        return Ok(());
    }

    match request_type {
        0 => {
            // GetHeight - respond with current block count
            let bc = blockchain.read().await;
            let height = bc.get_block_count() as u64;
            drop(bc);

            info!(height = height, peer = %addr, "Sending height to peer");

            let response = SignedSyncResponse::sign(height.to_le_bytes().to_vec(), &secret_key);
            let response_bytes = bincode::serialize(&response)?;
            let len = response_bytes.len() as u32;
            stream.write_all(&len.to_le_bytes()).await?;
            stream.write_all(&response_bytes).await?;
        }
        1 => {
            // GetBlocks - read from_block (u64) and count (u64)
            let mut params = [0u8; 16];
            stream.read_exact(&mut params).await?;

            let from_block = u64::from_le_bytes(
                params[0..8]
                    .try_into()
                    .map_err(|_| "Invalid from_block bytes")?,
            );
            let mut count = u64::from_le_bytes(
                params[8..16]
                    .try_into()
                    .map_err(|_| "Invalid count bytes")?,
            );

            // Rate limit: cap blocks per request
            if count > MAX_BLOCKS_PER_REQUEST {
                warn!(peer = %addr, requested = count, capped = MAX_BLOCKS_PER_REQUEST,
                      "Capping blocks per request");
                count = MAX_BLOCKS_PER_REQUEST;
            }

            info!(peer = %addr, from_block = from_block, count = count, "Peer requesting blocks");

            let bc = blockchain.read().await;
            let blocks = select_blocks_for_sync_batch(&bc, from_block, count);
            drop(bc);

            // Serialize blocks
            let blocks_data = bincode::serialize(&blocks)?;
            let block_count = blocks.len() as u64;

            info!(block_count = block_count, bytes = blocks_data.len(), peer = %addr,
                  "Sending blocks to peer");

            // Send signed response
            let mut payload = Vec::new();
            payload.extend_from_slice(&block_count.to_le_bytes());
            payload.extend_from_slice(&blocks_data);

            let response = SignedSyncResponse::sign(payload, &secret_key);
            let response_bytes = bincode::serialize(&response)?;
            let len = response_bytes.len() as u32;
            stream.write_all(&len.to_le_bytes()).await?;
            stream.write_all(&response_bytes).await?;
            stream.flush().await?;
        }
        _ => {
            warn!(request_type = request_type, peer = %addr, "Unknown sync request type");
        }
    }

    Ok(())
}

/// Sync client - connects to peer's sync port to download blocks
///
/// SECURITY:
/// - Uses authenticated protocol (v2) with Ed25519 signed responses
/// - Validates block integrity (hash, PoW) before accepting
/// - Connection timeout protection
/// - Size limits on responses
pub struct SyncClient;

impl SyncClient {
    /// Get peer's block height (authenticated) via QUIC
    ///
    /// # Arguments
    /// * `connection` - The QUIC connection to the peer
    /// * `expected_pubkey` - The peer's expected Ed25519 public key (32 bytes) for signature verification
    pub async fn get_peer_height_quic(
        connection: &quinn::Connection,
        expected_pubkey: &[u8; 32],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        // Open a bidirectional stream for sync
        let (mut send_stream, mut recv_stream) =
            tokio::time::timeout(Duration::from_secs(10), connection.open_bi()).await??;

        // Write stream type byte for sync
        send_stream.write_all(&[STREAM_TYPE_SYNC]).await?;

        // Send GetHeight request (v2 authenticated)
        let mut request = Vec::with_capacity(10);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION); // v2 = authenticated
        request.push(SyncRequestType::GetHeight as u8);
        send_stream.write_all(&request).await?;
        send_stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len > 1024 {
            return Err("Height response too large".into());
        }

        // Read and verify signed response
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on height response - possible MITM attack".into());
        }

        if response.data.len() != 8 {
            return Err("Invalid height response length".into());
        }

        let height = u64::from_le_bytes(
            response.data[0..8]
                .try_into()
                .map_err(|_| "Invalid height bytes")?,
        );
        info!(height = height, "Verified height from peer");

        Ok(height)
    }

    /// Get peer's block height (authenticated) - legacy TCP version
    /// DEPRECATED: Use get_peer_height_quic instead
    pub async fn get_peer_height(
        peer_addr: SocketAddr,
        expected_pubkey: &[u8; 32],
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        // Connect to sync port with timeout
        let sync_addr = SocketAddr::new(
            peer_addr.ip(),
            peer_addr
                .port()
                .checked_add(1)
                .ok_or("Peer port 65535 is not supported (sync port would overflow)")?,
        );
        let stream = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(sync_addr),
        )
        .await??;

        let mut stream = stream;

        // Send GetHeight request (v2 authenticated)
        let mut request = Vec::with_capacity(10);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION); // v2 = authenticated
        request.push(SyncRequestType::GetHeight as u8);
        stream.write_all(&request).await?;
        stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await?;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len > 1024 {
            return Err("Height response too large".into());
        }

        // Read and verify signed response
        let mut response_bytes = vec![0u8; response_len];
        stream.read_exact(&mut response_bytes).await?;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on height response - possible MITM attack".into());
        }

        if response.data.len() != 8 {
            return Err("Invalid height response length".into());
        }

        let height = u64::from_le_bytes(
            response.data[0..8]
                .try_into()
                .map_err(|_| "Invalid height bytes")?,
        );
        info!(height = height, "Verified height from peer");

        Ok(height)
    }

    /// Download blocks from peer via QUIC (authenticated with block validation)
    ///
    /// # Arguments
    /// * `connection` - The QUIC connection to the peer
    /// * `from_block` - Starting block number
    /// * `count` - Number of blocks to request
    /// * `expected_pubkey` - The peer's expected Ed25519 public key (32 bytes) for signature verification
    pub async fn download_blocks_quic(
        connection: &quinn::Connection,
        from_block: u64,
        count: u64,
        expected_pubkey: &[u8; 32],
    ) -> Result<Vec<crate::blockchain::Block>, Box<dyn std::error::Error + Send + Sync>> {
        // Open a bidirectional stream for sync
        let (mut send_stream, mut recv_stream) =
            tokio::time::timeout(Duration::from_secs(10), connection.open_bi()).await??;

        // Write stream type byte for sync
        send_stream.write_all(&[STREAM_TYPE_SYNC]).await?;

        // Cap request size
        let count = std::cmp::min(count, MAX_BLOCKS_PER_REQUEST);

        // Send GetBlocks request (v2 authenticated)
        let mut request = Vec::with_capacity(26);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION);
        request.push(SyncRequestType::GetBlocks as u8);
        request.extend_from_slice(&from_block.to_le_bytes());
        request.extend_from_slice(&count.to_le_bytes());
        send_stream.write_all(&request).await?;
        send_stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len as u64 > MAX_SYNC_RESPONSE_SIZE {
            return Err(format!("Response too large ({} bytes)", response_len).into());
        }

        // Read signed response with timeout
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            recv_stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        // CRITICAL: Verify signature before trusting data
        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on blocks response - possible MITM attack".into());
        }

        info!("Response signature verified");

        // Parse block count and blocks from signed payload
        if response.data.len() < 8 {
            return Err("Response payload too small".into());
        }

        let block_count = u64::from_le_bytes(
            response.data[0..8]
                .try_into()
                .map_err(|_| "Invalid block count bytes")?,
        );
        let blocks_data = &response.data[8..];

        // Deserialize blocks
        let blocks: Vec<crate::blockchain::Block> = bincode::deserialize(blocks_data)?;

        if blocks.len() != block_count as usize {
            return Err(format!(
                "Block count mismatch: expected {}, got {}",
                block_count,
                blocks.len()
            )
            .into());
        }

        // CRITICAL: Validate each block's integrity
        let mut validated_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            // Verify block hash matches calculated hash
            let calculated_hash = block.calculate_hash();
            if calculated_hash != block.hash {
                error!(block_number = block.header.block_number,
                       calculated = ?calculated_hash, claimed = ?block.hash,
                       "Block hash mismatch - rejecting");
                continue;
            }

            // Verify PoW (hash meets difficulty target)
            if !verify_block_pow(&block) {
                error!(
                    block_number = block.header.block_number,
                    "Block failed PoW verification - rejecting"
                );
                continue;
            }

            validated_blocks.push(block);
        }

        info!(
            validated = validated_blocks.len(),
            total = block_count,
            "Validated blocks"
        );

        Ok(validated_blocks)
    }

    /// Download blocks from peer (authenticated with block validation) - legacy TCP version
    /// DEPRECATED: Use download_blocks_quic instead
    pub async fn download_blocks(
        peer_addr: SocketAddr,
        from_block: u64,
        count: u64,
        expected_pubkey: &[u8; 32],
    ) -> Result<Vec<crate::blockchain::Block>, Box<dyn std::error::Error + Send + Sync>> {
        // Connect to sync port with timeout
        let sync_addr = SocketAddr::new(
            peer_addr.ip(),
            peer_addr
                .port()
                .checked_add(1)
                .ok_or("Peer port 65535 is not supported (sync port would overflow)")?,
        );
        let stream = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(sync_addr),
        )
        .await??;

        let mut stream = stream;

        // Cap request size
        let count = std::cmp::min(count, MAX_BLOCKS_PER_REQUEST);

        // Send GetBlocks request (v2 authenticated)
        let mut request = Vec::with_capacity(26);
        request.extend_from_slice(SYNC_MAGIC);
        request.push(SYNC_VERSION);
        request.push(SyncRequestType::GetBlocks as u8);
        request.extend_from_slice(&from_block.to_le_bytes());
        request.extend_from_slice(&count.to_le_bytes());
        stream.write_all(&request).await?;
        stream.flush().await?;

        // Read signed response length
        let mut len_bytes = [0u8; 4];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            stream.read_exact(&mut len_bytes),
        )
        .await??;
        let response_len = u32::from_le_bytes(len_bytes) as usize;

        if response_len as u64 > MAX_SYNC_RESPONSE_SIZE {
            return Err(format!("Response too large ({} bytes)", response_len).into());
        }

        // Read signed response with timeout
        let mut response_bytes = vec![0u8; response_len];
        tokio::time::timeout(
            Duration::from_secs(SYNC_TIMEOUT_SECS),
            stream.read_exact(&mut response_bytes),
        )
        .await??;

        let response: SignedSyncResponse = bincode::deserialize(&response_bytes)?;

        // CRITICAL: Verify signature before trusting data
        if !response.verify(expected_pubkey) {
            return Err("Invalid signature on blocks response - possible MITM attack".into());
        }

        info!("Response signature verified");

        // Parse block count and blocks from signed payload
        if response.data.len() < 8 {
            return Err("Response payload too small".into());
        }

        let block_count = u64::from_le_bytes(
            response.data[0..8]
                .try_into()
                .map_err(|_| "Invalid block count bytes")?,
        );
        let blocks_data = &response.data[8..];

        // Deserialize blocks
        let blocks: Vec<crate::blockchain::Block> = bincode::deserialize(blocks_data)?;

        if blocks.len() != block_count as usize {
            return Err(format!(
                "Block count mismatch: expected {}, got {}",
                block_count,
                blocks.len()
            )
            .into());
        }

        // CRITICAL: Validate each block's integrity
        let mut validated_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            // Verify block hash matches calculated hash
            let calculated_hash = block.calculate_hash();
            if calculated_hash != block.hash {
                error!(block_number = block.header.block_number,
                       calculated = ?calculated_hash, claimed = ?block.hash,
                       "Block hash mismatch - rejecting");
                continue;
            }

            // Verify PoW (hash meets difficulty target)
            if !verify_block_pow(&block) {
                error!(
                    block_number = block.header.block_number,
                    "Block failed PoW verification - rejecting"
                );
                continue;
            }

            validated_blocks.push(block);
        }

        info!(
            validated = validated_blocks.len(),
            total = block_count,
            "Validated blocks"
        );

        Ok(validated_blocks)
    }

    /// Transport-agnostic full sync implementation.
    ///
    /// This is the core sync logic shared between QUIC and TCP transports.
    /// The `transport` parameter provides transport-specific operations.
    async fn full_sync_impl(
        transport: &dyn SyncTransportOps,
        blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
        mining_manager: Option<Arc<crate::mining::MiningManager>>,
        expected_pubkey: &[u8; 32],
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let peer_desc = transport.peer_description();
        let peer_ip = transport.peer_ip();

        // Wait for blockchain to be fully loaded before syncing
        {
            let bc = blockchain.read().await;
            if !bc.is_ready() {
                info!("Waiting for blockchain to finish loading from storage");
                drop(bc);
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                return Ok(0);
            }
        }

        info!(peer = %peer_desc, "Starting full sync");

        // Get peer's height (authenticated)
        debug!("About to get peer height");
        let mut peer_height = match transport.get_peer_height(expected_pubkey).await {
            Ok(h) => {
                debug!(height = h, "Got peer height");
                h
            }
            Err(e) => {
                error!(error = %e, "get_peer_height failed");
                return Err(e);
            }
        };
        info!(peer = %peer_desc, peer_height = peer_height, "Peer block count");

        // Get local height (highest block number we have)
        debug!("Getting local height");
        let local_height = {
            let bc = blockchain.read().await;
            bc.latest_block_number()
        };
        info!(local_height = local_height, "Local block height");

        // peer_height is a block count; local_height is the highest block number
        // We're in sync if peer_height <= local_height + 1 (i.e., peer has no more blocks than us)
        if peer_height <= local_height + 1 {
            info!(
                local_height = local_height,
                peer_height = peer_height,
                "Already synced"
            );
            return Ok(0);
        }

        // SYNC-001: Pause mining during IBD to prevent DAG tip contamination
        // Mining must be paused BEFORE we determine we need to sync
        if let Some(ref mm) = mining_manager {
            mm.pause_for_sync();
        }
        // Use scopeguard to ensure mining is always resumed, even on error
        let _guard = scopeguard::guard(mining_manager, |mm| {
            if let Some(mm) = mm {
                mm.resume_after_sync();
            }
        });

        // CRITICAL: If peer is significantly ahead, this indicates independent chain growth
        // (e.g., two nodes started separately). Clear local chain to prevent DAG tip contamination
        // from orphaned local blocks. Threshold: peer has 50+ more blocks than us.
        // This prevents locally-mined blocks from becoming DAG tips that corrupt future mining.
        // VULN-002: Multi-peer consensus required before chain wipe.
        let needs_full_resync = should_trigger_resync(peer_ip, peer_height, local_height);
        let start_from = if needs_full_resync {
            // Safety check: verify sled storage isn't still loading before clearing
            // If sled has significantly more blocks than in-memory reports, skip fork recovery
            let sled_block_count = {
                let bc = blockchain.read().await;
                bc.get_storage_block_count()
            };

            if sled_block_count > local_height as usize + 10 {
                // Sled has more blocks than in-memory — still loading, don't wipe
                warn!(
                    sled_count = sled_block_count,
                    local_height = local_height,
                    "Fork detection skipped: storage still loading"
                );
                info!("Waiting 5 seconds for storage to finish loading");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                return Ok(0);
            }

            info!(
                peer_height = peer_height,
                local_height = local_height,
                "Chain fork detected"
            );
            info!("Clearing local chain and starting full resync");

            // Clear the local chain before syncing
            {
                let mut bc = blockchain.write().await;
                bc.clear_for_resync();
            }

            0 // Start from genesis
        } else {
            local_height + 1 // Request blocks AFTER the highest we have
        };

        debug!(
            blocks_to_sync = peer_height - start_from,
            from = start_from,
            to = peer_height,
            "Need to sync blocks"
        );

        // Download missing blocks in batches
        let mut total_synced = 0;
        let mut current = start_from;
        // Accumulated orphans: blocks with unresolvable parents that we'll retry after each batch
        const MAX_ACCUMULATED_ORPHANS: usize = 10000;
        let mut accumulated_orphans: Vec<crate::blockchain::Block> = Vec::new();
        let mut total_orphans_resolved = 0usize;

        while current < peer_height {
            // Re-query peer tip each batch so a mining peer does not outrun us during IBD
            // This ensures we catch up to a peer that's actively mining while we sync
            match transport.get_peer_height(expected_pubkey).await {
                Ok(h) if h > peer_height => {
                    info!(
                        new_height = h,
                        old_height = peer_height,
                        "Peer tip advanced"
                    );
                    peer_height = h;
                }
                Ok(_) => {}
                Err(e) => warn!(error = %e, "Could not refresh peer height, continuing"),
            }
            if current >= peer_height {
                break;
            }
            let count = std::cmp::min(MAX_BLOCKS_PER_REQUEST, peer_height - current);
            info!(from_block = current, to_block = current + count, peer = %peer_desc,
                  "Downloading blocks");

            let blocks = match transport
                .download_blocks(current, count, expected_pubkey)
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "Download failed");
                    break;
                }
            };

            if blocks.is_empty() {
                // If we're far behind and got no blocks, the peer has likely
                // pruned this range.  Jump toward the peer's available window
                // rather than stalling forever at the pruned start.
                if peer_height > current + 1000 {
                    let jump_to = peer_height.saturating_sub(500);
                    warn!(
                        from = current,
                        jump_to,
                        "No blocks returned (peer pruned this range) — jumping to peer's available window"
                    );
                    current = jump_to;
                    continue;
                }
                break;
            }

            // Sort blocks by number as baseline ordering
            let mut remaining_blocks = blocks;
            remaining_blocks.sort_by_key(|b| b.header.block_number);

            // Compute highest block number in this batch BEFORE processing
            // This is needed to advance past orphaned batches
            let highest_in_batch = remaining_blocks
                .iter()
                .map(|b| b.header.block_number)
                .max()
                .unwrap_or(current);

            // Debug: show first block info
            if let Some(first) = remaining_blocks.first() {
                debug!(
                    block_number = first.header.block_number,
                    hash_prefix = hex::encode(&first.hash.as_ref()[..8]),
                    "First block to add"
                );
            }

            // Topological sort: reorder the batch so parents come before children.
            // After sorting, most batches complete in a single pass, greatly reducing
            // the time blockchain.write() is held.
            remaining_blocks = topological_sort_blocks(remaining_blocks);

            // PRUNED-PEER CATCHUP: when the first block in this batch has a very high
            // number relative to our local tip, install it as a trusted checkpoint so
            // subsequent blocks can resolve their parents without needing the full
            // pruned history. Triggers at genesis (local_tip==0) OR when there's a
            // large gap (e.g. peer pruned the range between our tip and available blocks).
            const CHECKPOINT_GAP_THRESHOLD: u64 = 500;
            let mut checkpoint_installed = false;
            if !remaining_blocks.is_empty() {
                let local_tip = {
                    let bc_r = blockchain.read().await;
                    bc_r.latest_block_number()
                };
                let first_num = remaining_blocks[0].header.block_number;
                let large_gap = first_num > local_tip + CHECKPOINT_GAP_THRESHOLD;
                if (local_tip == 0 || large_gap) && first_num > 100 {
                    let checkpoint = remaining_blocks.remove(0);
                    let checkpoint_num = checkpoint.header.block_number;
                    let mut bc_w = blockchain.write().await;
                    if bc_w.install_sync_checkpoint(checkpoint) {
                        checkpoint_installed = true;
                        info!(
                            block_number = checkpoint_num,
                            local_tip, "Installed pruned-peer sync checkpoint"
                        );
                    }
                }
            }

            // Multi-pass insertion for BlockDAG: parents must exist before children
            let mut bc = blockchain.write().await;
            let mut total_added = if checkpoint_installed { 1 } else { 0 };
            let mut pass = 0;
            const MAX_PASSES: usize = 50; // Increased from 10 for deeper DAG chains

            while !remaining_blocks.is_empty() && pass < MAX_PASSES {
                pass += 1;
                let mut added_this_pass = 0;
                let mut still_remaining = Vec::new();

                for block in remaining_blocks {
                    let block_num = block.header.block_number;

                    // Skip if already exists
                    if bc.get_block_by_hash(&block.hash).is_some() {
                        continue;
                    }

                    // BlockDAG: accept if ANY parent exists (snapshot-sync nodes
                    // may not have the full DAG history, but one known ancestor suffices).
                    let any_parent_exists = block.header.parent_hashes.iter().any(|parent_hash| {
                        if *parent_hash == crate::types::Hash::default() {
                            return true; // Genesis-like parent
                        }
                        bc.get_block_by_hash(parent_hash).is_some()
                    });

                    if any_parent_exists {
                        // At least one parent exists — try to add the block
                        match bc.add_block_for_sync(block.clone()).await {
                            Ok(true) => {
                                added_this_pass += 1;
                            }
                            Ok(false) => {
                                // Block was already present (duplicate)
                            }
                            Err(e) => {
                                warn!(block_number = block_num, error = %e, "Failed to add block");
                                // Keep block in remaining for retry if it's a transient error
                                still_remaining.push(block);
                            }
                        }
                    } else {
                        // Parents don't exist yet, keep for next pass
                        still_remaining.push(block);
                    }
                }

                total_added += added_this_pass;

                // If no blocks were added this pass, remaining blocks have unresolvable parents
                if added_this_pass == 0 {
                    if !still_remaining.is_empty() {
                        // Save orphaned blocks for retry after future batches
                        // Enforce max accumulated orphan limit (discard oldest if exceeded)
                        if accumulated_orphans.len() + still_remaining.len()
                            > MAX_ACCUMULATED_ORPHANS
                        {
                            let overflow = accumulated_orphans.len() + still_remaining.len()
                                - MAX_ACCUMULATED_ORPHANS;
                            if overflow < still_remaining.len() {
                                // Discard oldest accumulated orphans to make room
                                accumulated_orphans.drain(0..overflow);
                            }
                            // If still_remaining itself exceeds limit, truncate it
                            if still_remaining.len() > MAX_ACCUMULATED_ORPHANS {
                                warn!(
                                    truncated = still_remaining.len() - MAX_ACCUMULATED_ORPHANS,
                                    "Truncating orphaned blocks to fit limit"
                                );
                                still_remaining.truncate(MAX_ACCUMULATED_ORPHANS);
                            }
                        }
                        warn!(
                            orphan_count = still_remaining.len(),
                            accumulated = accumulated_orphans.len() + still_remaining.len(),
                            "Blocks have unresolvable parents, saving for retry"
                        );
                        accumulated_orphans.extend(still_remaining);
                    }
                    break;
                }

                remaining_blocks = still_remaining;
            }

            drop(bc);

            info!(added = total_added, passes = pass, "Added validated blocks");
            total_synced += total_added;

            // Try to resolve accumulated orphans now that new blocks were added
            if total_added > 0 && !accumulated_orphans.is_empty() {
                let mut resolved_count = 0usize;
                let mut orphan_pass = 0u32;

                'orphan_resolution: loop {
                    orphan_pass += 1;
                    let mut still_orphaned = Vec::new();
                    let mut added_this_orphan_pass = 0usize;

                    for block in accumulated_orphans.drain(..) {
                        // BlockDAG: any parent suffices for orphan resolution too
                        let any_parent_exists = {
                            let bc = blockchain.read().await;
                            block.header.parent_hashes.iter().any(|parent_hash| {
                                if *parent_hash == crate::types::Hash::default() {
                                    return true;
                                }
                                bc.get_block_by_hash(parent_hash).is_some()
                            })
                        };

                        if any_parent_exists {
                            // Parents exist, try to add the block
                            let mut bc = blockchain.write().await;
                            match bc.add_block_for_sync(block.clone()).await {
                                Ok(true) => {
                                    added_this_orphan_pass += 1;
                                    resolved_count += 1;
                                    total_added += 1;
                                }
                                _ => {
                                    // Block failed to add (duplicate or error) - skip it
                                }
                            }
                        } else {
                            still_orphaned.push(block);
                        }
                    }

                    accumulated_orphans = still_orphaned;

                    // Stop if no progress, no orphans left, or too many passes
                    if added_this_orphan_pass == 0
                        || accumulated_orphans.is_empty()
                        || orphan_pass > 50
                    {
                        break 'orphan_resolution;
                    }
                }

                if resolved_count > 0 {
                    info!(
                        resolved = resolved_count,
                        "Resolved previously orphaned blocks"
                    );
                    total_orphans_resolved += resolved_count;
                }
                if !accumulated_orphans.is_empty() {
                    warn!(
                        orphan_count = accumulated_orphans.len(),
                        "Orphaned blocks still unresolved, will keep retrying"
                    );
                }
            }

            // Rate limit protection: add delay between sync batches to avoid triggering
            // rate limits on remote servers (typically 300 msgs/min = 5 msgs/sec)
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // Advance sync position: if all blocks were orphaned, jump past this batch
            // to avoid infinite loop re-requesting the same range
            current = if total_added == 0 {
                // All blocks in batch were orphaned - advance past them
                warn!(
                    count = count,
                    from = current,
                    to = highest_in_batch + 1,
                    "All blocks in batch orphaned - advancing sync"
                );
                highest_in_batch + 1
            } else {
                // Some blocks were added - continue from latest block number
                let bc = blockchain.read().await;
                bc.latest_block_number() + 1 // Use block NUMBER, not count
            };
        }

        // Final attempt to resolve any remaining accumulated orphans
        if !accumulated_orphans.is_empty() {
            info!(
                orphan_count = accumulated_orphans.len(),
                "Final attempt to resolve accumulated orphans"
            );
            let mut resolved_count = 0usize;
            let mut orphan_pass = 0u32;

            'final_orphan_resolution: loop {
                orphan_pass += 1;
                let mut still_orphaned = Vec::new();
                let mut added_this_orphan_pass = 0usize;

                for block in accumulated_orphans.drain(..) {
                    let any_parent_exists = {
                        let bc = blockchain.read().await;
                        block.header.parent_hashes.iter().any(|parent_hash| {
                            if *parent_hash == crate::types::Hash::default() {
                                return true;
                            }
                            bc.get_block_by_hash(parent_hash).is_some()
                        })
                    };

                    if any_parent_exists {
                        let mut bc = blockchain.write().await;
                        match bc.add_block_for_sync(block.clone()).await {
                            Ok(true) => {
                                added_this_orphan_pass += 1;
                                resolved_count += 1;
                            }
                            _ => {}
                        }
                    } else {
                        still_orphaned.push(block);
                    }
                }

                accumulated_orphans = still_orphaned;

                if added_this_orphan_pass == 0 || accumulated_orphans.is_empty() || orphan_pass > 50
                {
                    break 'final_orphan_resolution;
                }
            }

            total_orphans_resolved += resolved_count;
            if resolved_count > 0 {
                info!(
                    added = resolved_count,
                    "Final resolution: added orphan blocks"
                );
            }
            if !accumulated_orphans.is_empty() {
                warn!(
                    orphan_count = accumulated_orphans.len(),
                    "Orphan blocks could not be resolved (missing parents)"
                );
            }
        }

        info!(
            total_added = total_synced + total_orphans_resolved,
            orphans_resolved = total_orphans_resolved,
            "Full sync complete"
        );
        Ok(total_synced)
    }

    /// Perform full sync with peer via QUIC (secure)
    ///
    /// # Arguments
    /// * `connection` - The QUIC connection to the peer
    /// * `blockchain` - The blockchain to sync blocks into
    /// * `mining_manager` - Optional mining manager to pause during IBD (SYNC-001)
    pub async fn full_sync_quic(
        connection: &quinn::Connection,
        blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
        mining_manager: Option<Arc<crate::mining::MiningManager>>,
        expected_pubkey: &[u8; 32],
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let transport = QuicTransport::new(connection);
        Self::full_sync_impl(&transport, blockchain, mining_manager, expected_pubkey).await
    }

    /// Perform full sync with peer (secure) - legacy TCP version
    /// DEPRECATED: Use full_sync_quic instead
    ///
    /// # Arguments
    /// * `peer_addr` - The peer's socket address
    /// * `blockchain` - The blockchain to sync blocks into
    /// * `mining_manager` - Optional mining manager to pause during IBD (SYNC-001)
    /// * `expected_pubkey` - The peer's expected Ed25519 public key (32 bytes) for signature verification
    pub async fn full_sync(
        peer_addr: SocketAddr,
        blockchain: Arc<tokio::sync::RwLock<crate::blockchain::Blockchain>>,
        mining_manager: Option<Arc<crate::mining::MiningManager>>,
        expected_pubkey: &[u8; 32],
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let transport = TcpTransport::new(peer_addr);
        Self::full_sync_impl(&transport, blockchain, mining_manager, expected_pubkey).await
    }
}

/// Verify block's Proof of Work
fn verify_block_pow(block: &crate::blockchain::Block) -> bool {
    use crate::types::StreamType;

    // Genesis block doesn't need PoW verification
    if block.header.block_number == 0 {
        return true;
    }

    // Stream C doesn't use PoW
    if block.header.stream_type == StreamType::StreamC {
        return true;
    }

    // Check difficulty is non-zero
    if block.header.difficulty == 0 {
        return false;
    }

    // Count leading zeros in hash
    let hash_bytes = &block.hash;
    let mut leading_zeros = 0u64;

    for byte in hash_bytes.iter() {
        if *byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as u64;
            break;
        }
    }

    // Hash must have at least `difficulty` leading zero bits
    leading_zeros >= block.header.difficulty
}

/// Topological sort (Kahn's algorithm) for a batch of DAG blocks.
///
/// Returns the same blocks reordered so that every block appears after all of
/// its parents that are also in the batch. Blocks whose parents are already in
/// the chain (not in the batch) have in-degree zero and come first. Any blocks
/// that form a cycle or whose batch-parents are all missing end up appended at
/// the tail unchanged — the caller's existing retry logic handles them.
///
/// Complexity: O(N + E) where N = number of blocks, E = parent edges in batch.
fn topological_sort_blocks(blocks: Vec<crate::blockchain::Block>) -> Vec<crate::blockchain::Block> {
    let batch_hashes: HashSet<Hash> = blocks.iter().map(|b| b.hash).collect();

    // Move blocks into a hash-keyed map so we can extract them in order.
    let mut by_hash: HashMap<Hash, crate::blockchain::Block> =
        blocks.into_iter().map(|b| (b.hash, b)).collect();

    // in_degree = number of this block's parents that are also in the batch.
    // Parents already committed to the chain don't count.
    let mut in_degree: HashMap<Hash, usize> = by_hash
        .iter()
        .map(|(hash, block)| {
            let batch_parents = block
                .header
                .parent_hashes
                .iter()
                .filter(|p| batch_hashes.contains(*p))
                .count();
            (*hash, batch_parents)
        })
        .collect();

    // children[p] = list of batch blocks that have p as a parent.
    let mut children: HashMap<Hash, Vec<Hash>> = HashMap::new();
    for (hash, block) in &by_hash {
        for parent in &block.header.parent_hashes {
            if batch_hashes.contains(parent) {
                children.entry(*parent).or_default().push(*hash);
            }
        }
    }

    // Seed queue with roots (no batch parents — ready to insert immediately).
    let mut queue: VecDeque<Hash> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&h, _)| h)
        .collect();

    let mut sorted = Vec::with_capacity(by_hash.len());

    while let Some(h) = queue.pop_front() {
        if let Some(block) = by_hash.remove(&h) {
            if let Some(child_hashes) = children.get(&h) {
                for child_hash in child_hashes {
                    if let Some(deg) = in_degree.get_mut(child_hash) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(*child_hash);
                        }
                    }
                }
            }
            sorted.push(block);
        }
    }

    // Append any remaining blocks (genuine orphans or cycles) — handled by caller.
    sorted.extend(by_hash.into_values());
    sorted
}
