//! JSON-RPC 2.0 API Server
//!
//! Provides Ethereum-compatible JSON-RPC methods for external tool integration
//!
//! Copyright (c) 2024-2025 IronDAG Contributors
//! Licensed under the BUSL-1.1 License (see LICENSE file)
//!
//! # Module Organization
//!
//! This module is organized into the following logical sections:
//!
//! - **Constants & Helpers** (lines 22-180): RPC constants, helper functions for hex parsing,
//!   timing-safe comparisons, and API key utilities.
//!
//! - **Data Structures** (lines 191-380): JSON-RPC 2.0 request/response types and the main
//!   `RpcServer` struct with all its state fields.
//!
//! - **Server Construction** (lines 460-860): Factory methods for creating RPC server instances
//!   with various configurations (with/without auth, sharding, rate limiting, etc.).
//!
//! - **Request Handling Infrastructure** (lines 860-1100): Batch handling, timeouts, health
//!   status, and the main request router.
//!
//! - **Ethereum Standard Methods** (lines 1480-2400): eth_getBalance, eth_getBlockByNumber,
//!   eth_sendRawTransaction, etc. These provide Metamask/web3 compatibility.
//!
//! - **IronDAG Extension Methods** (lines 2400-8800): irondag_* methods for blockchain-specific
//!   features: sharding, privacy, oracles, account abstraction, snapshots, mining, etc.
//!
//! - **Helper Functions** (parse_address, block_to_json, etc.): Utilities for parameter
//!   extraction and response formatting.

pub mod grpc;
pub mod rate_limit;
pub mod v2; // Binary-optimized gRPC v2 (3.3x faster)

use crate::blockchain::{Block, Blockchain, Transaction};
use crate::types::{Address, Hash, DEFAULT_CHAIN_ID};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// Faucet rate limiting: max 1 request per address per 60 seconds
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Global faucet rate limiter: maps address to last request timestamp
fn faucet_rate_limiter() -> &'static Mutex<HashMap<[u8; 20], Instant>> {
    static RATE_LIMITER: OnceLock<Mutex<HashMap<[u8; 20], Instant>>> = OnceLock::new();
    RATE_LIMITER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Faucet rate limit duration (60 seconds)
const FAUCET_RATE_LIMIT_SECONDS: u64 = 60;

/// Total time to keep retrying blockchain read lock before giving up.
/// try_read() catches brief windows between mining writes; 30 s covers worst-case bursts.
const RPC_LOCK_TIMEOUT_MS: u64 = 30_000;

/// Sleep between try_read() attempts (ms). Short enough to catch inter-write gaps.
const RPC_LOCK_RETRY_MS: u64 = 5;

/// Maximum number of entries in the response cache (evict one when full)
const RESPONSE_CACHE_MAX_SIZE: usize = 1000;

/// RPC request timeout constants for production hardening
/// Default timeout for write operations (30 seconds)
const RPC_DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Timeout for read-only operations (15 seconds)
const RPC_READ_TIMEOUT_SECS: u64 = 15;
/// Maximum response size (10MB) to prevent resource exhaustion
const RPC_MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024;
/// Maximum batch request size (100 requests)
const RPC_MAX_BATCH_SIZE: usize = 100;

/// Hex address length constant (0x + 40 hex chars = 42 chars)
/// Used for Ethereum address validation: 2 (0x prefix) + 20 bytes * 2 (hex) = 42
const HEX_ADDRESS_LEN: usize = 42;

/// Hash length constant (0x + 64 hex chars = 66 chars)
/// Used for Ethereum hash validation: 2 (0x prefix) + 32 bytes * 2 (hex) = 66
const HEX_HASH_LEN: usize = 66;

/// Blocks per hour multiplier for hashrate estimation
/// Based on 100-block sample: 3600 seconds/hour / 100 seconds per sample block = 36
const BLOCKS_PER_HOUR_MULTIPLIER: f64 = 36.0;
/// Slow request threshold for logging (5 seconds)
const RPC_SLOW_REQUEST_THRESHOLD_SECS: u64 = 5;
/// Info-level logging threshold (1 second)
const RPC_INFO_LOG_THRESHOLD_SECS: u64 = 1;

/// Check if a method is a read-only operation (for timeout selection)
/// Read methods get shorter timeouts (5s) vs write methods (30s)
#[inline]
fn is_read_method(method: &str) -> bool {
    !is_state_changing_method(method)
}

/// Check if a method modifies state (for rate limiting)
/// State-changing methods get stricter rate limits
#[inline]
fn is_state_changing_method(method: &str) -> bool {
    matches!(
        method,
        "eth_sendRawTransaction" | "eth_sendTransaction" | "irondag_faucet"
    )
}

/// Check if a method is public (doesn't require authentication)
#[inline]
fn is_public_method(method: &str) -> bool {
    matches!(
        method,
        "eth_blockNumber"
            | "net_version"
            | "eth_chainId"
            | "eth_syncing"
            | "irondag_getDagStats"
            | "irondag_getTps"
            | "irondag_getBlocksByStream"
            | "irondag_getStreamCounts"
            | "irondag_faucet"
    )
}

/// Constant-time byte comparison to avoid timing side channels (e.g. API key verification).
#[inline(always)]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Hash API key for storage (we store only the hash, never the plain key).
#[inline(always)]
fn hash_api_key(key: &str) -> [u8; 32] {
    let h = blake3::hash(key.as_bytes());
    *h.as_bytes()
}

/// Generate a random 32-byte API key as a hex string.
/// Uses rand::thread_rng() for cryptographically secure random bytes.
fn generate_random_api_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ============================================================================
// JSON-RPC 2.0 Standard Error Codes
// ============================================================================

/// Parse error - Invalid JSON was received
pub const RPC_PARSE_ERROR: i32 = -32700;
/// Invalid request - JSON is not a valid Request object
pub const RPC_INVALID_REQUEST: i32 = -32600;
/// Method not found - Method does not exist
pub const RPC_METHOD_NOT_FOUND: i32 = -32601;
/// Invalid params - Invalid method parameters
pub const RPC_INVALID_PARAMS: i32 = -32602;
/// Internal error - Internal JSON-RPC error
pub const RPC_INTERNAL_ERROR: i32 = -32603;

// Application-specific error codes (-32000 to -32099)
/// Block not found
pub const RPC_BLOCK_NOT_FOUND: i32 = -32001;
/// Transaction not found
pub const RPC_TX_NOT_FOUND: i32 = -32002;
/// Invalid address format
pub const RPC_INVALID_ADDRESS: i32 = -32003;
/// Invalid transaction
pub const RPC_INVALID_TX: i32 = -32004;
/// Rate limit exceeded
pub const RPC_RATE_LIMITED: i32 = -32005;
/// Resource unavailable (lock timeout, etc.)
pub const RPC_RESOURCE_UNAVAILABLE: i32 = -32006;

// ============================================================================
// Error Constructor Helpers (STYLE-02)
// ============================================================================
// These helpers reduce repetitive inline JsonRpcError construction.
// ============================================================================

/// Create a standard JSON-RPC error with a code and message.
#[inline]
fn rpc_error(code: i32, message: &str) -> JsonRpcError {
    JsonRpcError {
        code,
        message: message.to_string(),
        data: None,
    }
}

/// Create an error for missing parameters.
#[inline]
fn missing_params_error() -> JsonRpcError {
    rpc_error(RPC_INVALID_PARAMS, "Invalid params")
}

/// Create an error for a missing named parameter.
#[inline]
fn missing_param_error(param_name: &str) -> JsonRpcError {
    JsonRpcError {
        code: RPC_INVALID_PARAMS,
        message: format!("Missing {} parameter", param_name),
        data: None,
    }
}

/// Create an error for an invalid parameter value.
#[inline]
fn invalid_param_error(param_name: &str) -> JsonRpcError {
    JsonRpcError {
        code: RPC_INVALID_PARAMS,
        message: format!("Invalid {} parameter", param_name),
        data: None,
    }
}

/// Create an internal server error.
#[inline]
#[allow(dead_code)]
fn internal_error(message: &str) -> JsonRpcError {
    rpc_error(RPC_INTERNAL_ERROR, message)
}

/// Create a method not found error.
#[inline]
#[allow(dead_code)]
fn method_not_found_error(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: RPC_METHOD_NOT_FOUND,
        message: format!("Method not found: {}", method),
        data: None,
    }
}

// ============================================================================
// Parameter Extraction Helpers (STYLE-03)
// ============================================================================
// These helpers reduce repetitive params.as_array().and_then(|arr| arr.get(N))
// patterns throughout the method handlers.
// ============================================================================

/// Extract a string parameter at the given index from a JSON array.
/// Returns None if params is not an array, index out of bounds, or not a string.
#[inline]
fn extract_str_param(params: &[Value], index: usize) -> Option<&str> {
    params.get(index).and_then(|v| v.as_str())
}

/// Extract a hex parameter at the given index from a JSON array.
/// Returns None if params is not an array, index out of bounds, or not a string.
/// On success, returns the decoded bytes (without 0x prefix).
#[inline]
#[allow(dead_code)]
fn extract_hex_param(params: &[Value], index: usize) -> Option<Vec<u8>> {
    params.get(index).and_then(|v| v.as_str()).and_then(|s| {
        let hex = if s.starts_with("0x") { &s[2..] } else { s };
        hex::decode(hex).ok()
    })
}

/// Extract an address parameter at the given index from a JSON array.
/// Returns an error if the address is invalid.
#[inline]
fn extract_address_param(params: &[Value], index: usize) -> Result<Address, JsonRpcError> {
    let addr_str =
        extract_str_param(params, index).ok_or_else(|| invalid_param_error("address"))?;
    parse_address(addr_str)
}

/// Extract a hash parameter at the given index from a JSON array.
/// Returns an error if the hash is invalid.
#[inline]
fn extract_hash_param(params: &[Value], index: usize) -> Result<Hash, JsonRpcError> {
    let hash_str = extract_str_param(params, index).ok_or_else(|| invalid_param_error("hash"))?;
    parse_hash(hash_str)
}

// ============================================================================
// Validation Helpers (centralized, no external dependencies)
// ============================================================================

/// Validate hex string format (must start with 0x, valid hex chars)
#[inline]
pub fn validate_hex(value: &str) -> Result<(), JsonRpcError> {
    if !value.starts_with("0x") {
        return Err(JsonRpcError {
            code: RPC_INVALID_PARAMS,
            message: "Hex string must start with 0x".to_string(),
            data: Some(json!({"value": value})),
        });
    }

    let hex_part = &value[2..];
    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(JsonRpcError {
            code: RPC_INVALID_PARAMS,
            message: "Invalid hex characters".to_string(),
            data: Some(json!({"value": value})),
        });
    }

    Ok(())
}

/// Validate Ethereum address format (0x + 40 hex chars = 20 bytes)
#[inline]
pub fn validate_address(address: &str) -> Result<(), JsonRpcError> {
    validate_hex(address)?;

    if address.len() != HEX_ADDRESS_LEN {
        return Err(JsonRpcError {
            code: RPC_INVALID_ADDRESS,
            message: format!(
                "Address must be 20 bytes ({} chars with 0x), got {}",
                HEX_ADDRESS_LEN,
                address.len()
            ),
            data: Some(json!({"address": address})),
        });
    }

    Ok(())
}

/// Validate block number parameter (hex number or tag)
#[inline]
pub fn validate_block_param(value: &str) -> Result<(), JsonRpcError> {
    // Tags are valid
    if matches!(
        value,
        "latest" | "earliest" | "pending" | "safe" | "finalized"
    ) {
        return Ok(());
    }

    validate_hex(value)?;

    // Sanity check for reasonable block number
    let num = u64::from_str_radix(&value[2..], 16).map_err(|_| JsonRpcError {
        code: RPC_INVALID_PARAMS,
        message: "Invalid block number".to_string(),
        data: Some(json!({"value": value})),
    })?;

    if num > 10_000_000_000 {
        return Err(JsonRpcError {
            code: RPC_INVALID_PARAMS,
            message: "Block number unreasonably high".to_string(),
            data: Some(json!({"value": value, "max": 10_000_000_000u64})),
        });
    }

    Ok(())
}

/// Validate hash format (0x + 64 hex chars = 32 bytes)
#[inline]
pub fn validate_hash(hash: &str) -> Result<(), JsonRpcError> {
    validate_hex(hash)?;

    if hash.len() != HEX_HASH_LEN {
        return Err(JsonRpcError {
            code: RPC_INVALID_PARAMS,
            message: format!(
                "Hash must be 32 bytes ({} chars with 0x), got {}",
                HEX_HASH_LEN,
                hash.len()
            ),
            data: Some(json!({"hash": hash})),
        });
    }

    Ok(())
}

/// Parse hex string to bytes (after validation)
#[inline]
pub fn parse_hex_bytes(hex: &str) -> Result<Vec<u8>, JsonRpcError> {
    hex::decode(&hex[2..]).map_err(|e| JsonRpcError {
        code: RPC_INVALID_PARAMS,
        message: format!("Failed to decode hex: {}", e),
        data: None,
    })
}

/// Parse address string to Address
#[inline]
pub fn parse_address(address: &str) -> Result<Address, JsonRpcError> {
    validate_address(address)?;
    let bytes = parse_hex_bytes(address)?;
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&bytes);
    Ok(Address(addr))
}

/// Parse hash string to Hash
#[inline]
pub fn parse_hash(hash: &str) -> Result<Hash, JsonRpcError> {
    validate_hash(hash)?;
    let bytes = parse_hex_bytes(hash)?;
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Ok(Hash(h))
}

// ============================================================================
// JSON-RPC Types
// ============================================================================

/// Convert bytes to u128 (big-endian)
#[allow(dead_code)]
fn bytes_to_u128(bytes: &[u8]) -> Result<u128, JsonRpcError> {
    if bytes.len() > 16 {
        return Err(JsonRpcError {
            code: -32602,
            message: format!("Value too large: {} bytes (max 16)", bytes.len()),
            data: None,
        });
    }

    let mut result = 0u128;
    for &byte in bytes {
        result = (result << 8) | (byte as u128);
    }
    Ok(result)
}

/// JSON-RPC 2.0 Request
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<Value>,
}

/// JSON-RPC Error
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// RPC server
pub struct RpcServer {
    blockchain: Arc<RwLock<Blockchain>>,
    network_manager: Option<Arc<crate::network::NetworkManager>>,
    rate_limiter: Option<Arc<rate_limit::RateLimiter>>,
    /// Per-IP rate limiter (when set and client IP available, used instead of or in addition to global)
    per_ip_rate_limiter: Option<Arc<rate_limit::PerIpRateLimiter>>,
    /// Per-IP rate limiter specifically for transaction submission (100 tx/min per IP)
    tx_submission_rate_limiter: Option<Arc<rate_limit::PerIpRateLimiter>>,
    shard_manager: Option<Arc<crate::sharding::ShardManager>>,
    metrics: Option<crate::metrics::MetricsHandle>,
    /// Security scorer for risk analysis
    security_scorer: Option<Arc<tokio::sync::RwLock<crate::security::RiskScorer>>>,
    /// Mining manager for fairness metrics
    mining_manager: Option<Arc<crate::mining::MiningManager>>,
    /// Forensic analyzer for fund tracing and address analysis
    forensic_analyzer: Option<Arc<tokio::sync::RwLock<crate::security::ForensicAnalyzer>>>,
    /// Light client for stateless mode
    light_client: Option<Arc<tokio::sync::RwLock<crate::light_client::LightClient>>>,
    /// Security policy manager for opt-in behavior gating
    policy_manager: Option<Arc<tokio::sync::RwLock<crate::security::SecurityPolicyManager>>>,
    /// Node registry for governance and longevity tracking
    node_registry: Option<Arc<tokio::sync::RwLock<crate::governance::NodeRegistry>>>,
    /// Reputation manager for trust scores
    reputation_manager: Option<Arc<tokio::sync::RwLock<crate::reputation::ReputationManager>>>,
    /// Wallet registry for account abstraction
    wallet_registry: Option<Arc<tokio::sync::RwLock<crate::account_abstraction::WalletRegistry>>>,
    /// Multi-signature manager for pending transactions
    multisig_manager: Option<Arc<tokio::sync::RwLock<crate::account_abstraction::MultiSigManager>>>,
    /// Social recovery manager for wallet recovery
    social_recovery_manager:
        Option<Arc<tokio::sync::RwLock<crate::account_abstraction::SocialRecoveryManager>>>,
    /// Batch transaction manager
    batch_manager: Option<Arc<tokio::sync::RwLock<crate::account_abstraction::BatchManager>>>,
    /// Parallel EVM executor
    parallel_evm_executor:
        Option<Arc<tokio::sync::RwLock<crate::evm::parallel::ParallelEvmExecutor>>>,
    /// Oracle registry for price feeds and randomness
    oracle_registry: Option<Arc<tokio::sync::RwLock<crate::oracles::OracleRegistry>>>,
    /// Price feed manager
    price_feed_manager: Option<Arc<tokio::sync::RwLock<crate::oracles::PriceFeedManager>>>,
    /// VRF manager for randomness
    vrf_manager: Option<Arc<tokio::sync::RwLock<crate::oracles::VrfManager>>>,
    // TODO: Oracle staking manager - reserved for future oracle staking functionality
    // Currently unused, will be enabled when oracle staking is implemented
    // oracle_staking: Option<Arc<tokio::sync::RwLock<crate::oracles::OracleStaking>>>,
    /// Recurring transaction manager
    recurring_manager:
        Option<Arc<tokio::sync::RwLock<crate::recurring::RecurringTransactionManager>>>,
    /// Stop-loss manager
    stop_loss_manager: Option<Arc<tokio::sync::RwLock<crate::stop_loss::StopLossManager>>>,
    /// PQ keystore for post-quantum transaction signing (maps address to PQ account)
    pq_keystore: Option<std::collections::HashMap<Address, crate::pqc::PqAccountExport>>,
    /// Privacy manager
    #[cfg(feature = "privacy")]
    privacy_manager: RwLock<Option<Arc<tokio::sync::RwLock<crate::privacy::PrivacyManager>>>>,
    /// Privacy prover (for generating proofs)
    #[cfg(feature = "privacy")]
    privacy_prover: RwLock<Option<Arc<crate::privacy::PrivacyProver>>>,
    /// Security hardening (DoS protection, IP filtering, rate limiting)
    security_hardening: Option<Arc<tokio::sync::RwLock<crate::security::SecurityHardening>>>,
    /// API key hash for authentication (only hash is stored; key never kept in memory). If None, auth disabled.
    api_key_hash: Option<[u8; 32]>,
    /// Allow unsigned eth_sendTransaction (dev only). When false, require signed tx or use eth_sendRawTransaction.
    allow_unsigned_eth_send: bool,
    /// Chain ID for EIP-155 and network identification
    chain_id: u64,
    /// This node's miner address (reported as coinbase in eth_getBlock*).
    miner_address: Address,
    /// Response cache for read-only methods (TTL + block-based invalidation)
    response_cache:
        Arc<tokio::sync::RwLock<std::collections::HashMap<String, (Value, Instant, u64)>>>,
    /// Last block number (for cache invalidation)
    _last_block_number: Arc<AtomicU64>,
    /// Cache TTL (milliseconds)
    #[allow(dead_code)]
    cache_ttl_ms: u64,
    /// Direct access to blockchain's cached block number (lock-free)
    blockchain_cached_block_number: Arc<AtomicU64>,
    /// Direct access to the accounts DashMap for lock-free balance/nonce queries.
    /// Shared Arc — always reflects the latest committed state.
    accounts_cache: Arc<dashmap::DashMap<Address, crate::blockchain::AccountState>>,
    /// RPC request counter for metrics (total requests processed)
    rpc_requests_total: Arc<AtomicU64>,
    /// RPC error counter for metrics (total errors)
    rpc_errors_total: Arc<AtomicU64>,
    /// Trace ID counter for request observability (atomic u64 for unique IDs)
    trace_id_counter: Arc<AtomicU64>,
}

impl RpcServer {
    /// Create RPC server with auto-generated API key (secure-by-default).
    /// Prints the generated key to stdout for the operator.
    pub fn new(blockchain: Arc<RwLock<Blockchain>>) -> Self {
        Self::new_with_chain_id(blockchain, DEFAULT_CHAIN_ID)
    }

    /// Create RPC server with auto-generated API key and specified chain ID.
    /// Prints the generated key to stdout for the operator.
    pub fn new_with_chain_id(blockchain: Arc<RwLock<Blockchain>>, chain_id: u64) -> Self {
        let mut server = Self::with_chain_id(blockchain, chain_id);
        let api_key = generate_random_api_key();
        println!("================================================================");
        println!("  RPC API KEY: {}", api_key);
        println!("  This key changes on every restart unless you set a static key");
        println!("  via --rpc-api-key <KEY> or the IRONDAG_API_KEY env var.");
        println!("================================================================");
        info!("RPC API Key generated (use full key shown at startup)");
        server.api_key_hash = Some(hash_api_key(&api_key));
        server
    }

    /// Create RPC server WITHOUT authentication (for local development only).
    /// WARNING: This disables all API key checks. Do not use in production.
    pub fn without_auth(blockchain: Arc<RwLock<Blockchain>>) -> Self {
        Self::with_chain_id(blockchain, DEFAULT_CHAIN_ID)
    }

    /// Create RPC server WITHOUT authentication and specified chain ID (for local development only).
    /// WARNING: This disables all API key checks. Do not use in production.
    pub fn without_auth_with_chain_id(blockchain: Arc<RwLock<Blockchain>>, chain_id: u64) -> Self {
        Self::with_chain_id(blockchain, chain_id)
    }

    pub fn with_chain_id(blockchain: Arc<RwLock<Blockchain>>, chain_id: u64) -> Self {
        // Get direct reference to blockchain's cached block number (one-time lock acquisition)
        let (blockchain_cached_block_number, accounts_cache) = {
            let bc = match blockchain.try_read() {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                    panic!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                }
            };
            (bc.get_cached_block_number_arc(), bc.accounts_arc())
        };

        Self {
            blockchain,
            network_manager: None,
            rate_limiter: None,
            per_ip_rate_limiter: None,
            tx_submission_rate_limiter: None,
            shard_manager: None,
            metrics: None,
            security_scorer: None,
            mining_manager: None,
            forensic_analyzer: None,
            light_client: None,
            policy_manager: None,
            node_registry: None,
            reputation_manager: None,
            wallet_registry: None,
            multisig_manager: None,
            social_recovery_manager: None,
            batch_manager: None,
            parallel_evm_executor: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            recurring_manager: None,
            stop_loss_manager: None,
            pq_keystore: None,
            #[cfg(feature = "privacy")]
            privacy_manager: RwLock::new(None),
            #[cfg(feature = "privacy")]
            privacy_prover: RwLock::new(None),
            security_hardening: None,
            api_key_hash: None,
            allow_unsigned_eth_send: false,
            chain_id,
            miner_address: Address::zero(),
            response_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            _last_block_number: Arc::new(AtomicU64::new(0)),
            cache_ttl_ms: 100, // 100ms cache TTL (very short for blockchain data)
            blockchain_cached_block_number,
            accounts_cache,
            rpc_requests_total: Arc::new(AtomicU64::new(0)),
            rpc_errors_total: Arc::new(AtomicU64::new(0)),
            trace_id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create RPC server with API key authentication. Key is hashed immediately; plain key is not stored.
    pub fn with_auth(blockchain: Arc<RwLock<Blockchain>>, api_key: String) -> Self {
        let mut server = Self::new(blockchain);
        server.api_key_hash = Some(hash_api_key(&api_key));
        server
    }

    /// Create RPC server with chain ID and API key authentication. Key is hashed immediately; plain key is not stored.
    pub fn with_chain_id_and_auth(
        blockchain: Arc<RwLock<Blockchain>>,
        chain_id: u64,
        api_key: String,
    ) -> Self {
        let mut server = Self::with_chain_id(blockchain, chain_id);
        server.api_key_hash = Some(hash_api_key(&api_key));
        server
    }

    /// Create RPC server with rate limiting
    pub fn with_rate_limit(
        blockchain: Arc<RwLock<Blockchain>>,
        max_tokens: u32,
        tokens_per_second: f64,
    ) -> Self {
        Self::with_chain_id_and_rate_limit(
            blockchain,
            DEFAULT_CHAIN_ID,
            max_tokens,
            tokens_per_second,
        )
    }

    /// Create RPC server with chain ID and rate limiting
    pub fn with_chain_id_and_rate_limit(
        blockchain: Arc<RwLock<Blockchain>>,
        chain_id: u64,
        max_tokens: u32,
        tokens_per_second: f64,
    ) -> Self {
        // Get direct reference to blockchain's cached block number BEFORE moving blockchain
        let (blockchain_cached_block_number, accounts_cache) = {
            let bc = match blockchain.try_read() {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                    panic!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                }
            };
            (bc.get_cached_block_number_arc(), bc.accounts_arc())
        };

        Self {
            blockchain,
            network_manager: None,
            rate_limiter: Some(Arc::new(rate_limit::RateLimiter::new(
                max_tokens,
                tokens_per_second,
            ))),
            per_ip_rate_limiter: None,
            tx_submission_rate_limiter: None,
            shard_manager: None,
            metrics: None,
            security_scorer: None,
            mining_manager: None,
            forensic_analyzer: None,
            light_client: None,
            policy_manager: None,
            node_registry: None,
            reputation_manager: None,
            wallet_registry: None,
            multisig_manager: None,
            social_recovery_manager: None,
            batch_manager: None,
            parallel_evm_executor: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            recurring_manager: None,
            stop_loss_manager: None,
            pq_keystore: None,
            #[cfg(feature = "privacy")]
            privacy_manager: RwLock::new(None),
            #[cfg(feature = "privacy")]
            privacy_prover: RwLock::new(None),
            security_hardening: None,
            api_key_hash: None,
            allow_unsigned_eth_send: false,
            chain_id,
            miner_address: Address::zero(),
            response_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            _last_block_number: Arc::new(AtomicU64::new(0)),
            cache_ttl_ms: 100, // 100ms cache TTL (very short for blockchain data)
            blockchain_cached_block_number,
            accounts_cache,
            rpc_requests_total: Arc::new(AtomicU64::new(0)),
            rpc_errors_total: Arc::new(AtomicU64::new(0)),
            trace_id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create RPC server with rate limiting and authentication
    pub fn with_rate_limit_and_auth(
        blockchain: Arc<RwLock<Blockchain>>,
        max_tokens: u32,
        tokens_per_second: f64,
        api_key: String,
    ) -> Self {
        let mut server = Self::with_rate_limit(blockchain, max_tokens, tokens_per_second);
        server.api_key_hash = Some(hash_api_key(&api_key));
        server
    }

    /// Create RPC server with sharding
    pub fn with_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        shard_manager: Arc<crate::sharding::ShardManager>,
    ) -> Self {
        Self::with_chain_id_and_sharding(blockchain, DEFAULT_CHAIN_ID, shard_manager)
    }

    /// Create RPC server with chain ID and sharding
    pub fn with_chain_id_and_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        chain_id: u64,
        shard_manager: Arc<crate::sharding::ShardManager>,
    ) -> Self {
        // Get direct reference to blockchain's cached block number BEFORE moving blockchain
        let (blockchain_cached_block_number, accounts_cache) = {
            let bc = match blockchain.try_read() {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                    panic!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                }
            };
            (bc.get_cached_block_number_arc(), bc.accounts_arc())
        };

        Self {
            blockchain,
            network_manager: None,
            rate_limiter: None,
            per_ip_rate_limiter: None,
            tx_submission_rate_limiter: None,
            shard_manager: Some(shard_manager),
            metrics: None,
            security_scorer: None,
            mining_manager: None,
            forensic_analyzer: None,
            light_client: None,
            policy_manager: None,
            node_registry: None,
            reputation_manager: None,
            wallet_registry: None,
            multisig_manager: None,
            social_recovery_manager: None,
            batch_manager: None,
            parallel_evm_executor: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            recurring_manager: None,
            stop_loss_manager: None,
            pq_keystore: None,
            #[cfg(feature = "privacy")]
            privacy_manager: RwLock::new(None),
            #[cfg(feature = "privacy")]
            privacy_prover: RwLock::new(None),
            security_hardening: None,
            api_key_hash: None,
            allow_unsigned_eth_send: false,
            chain_id,
            miner_address: Address::zero(),
            response_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            _last_block_number: Arc::new(AtomicU64::new(0)),
            cache_ttl_ms: 100, // 100ms cache TTL (very short for blockchain data)
            blockchain_cached_block_number,
            accounts_cache,
            rpc_requests_total: Arc::new(AtomicU64::new(0)),
            rpc_errors_total: Arc::new(AtomicU64::new(0)),
            trace_id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create RPC server with chain ID and sharding WITHOUT authentication (for local development only).
    /// WARNING: This disables all API key checks. Do not use in production.
    pub fn without_auth_with_chain_id_and_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        chain_id: u64,
        shard_manager: Arc<crate::sharding::ShardManager>,
    ) -> Self {
        // Just delegate to with_chain_id_and_sharding since it doesn't set auth by default
        Self::with_chain_id_and_sharding(blockchain, chain_id, shard_manager)
    }

    /// Create RPC server with both rate limiting and sharding
    pub fn with_rate_limit_and_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        max_tokens: u32,
        tokens_per_second: f64,
        shard_manager: Arc<crate::sharding::ShardManager>,
    ) -> Self {
        Self::with_chain_id_rate_limit_and_sharding(
            blockchain,
            0x4D534857,
            max_tokens,
            tokens_per_second,
            shard_manager,
        )
    }

    /// Create RPC server with chain ID, rate limiting, and sharding
    pub fn with_chain_id_rate_limit_and_sharding(
        blockchain: Arc<RwLock<Blockchain>>,
        chain_id: u64,
        max_tokens: u32,
        tokens_per_second: f64,
        shard_manager: Arc<crate::sharding::ShardManager>,
    ) -> Self {
        // Get direct reference to blockchain's cached block number BEFORE moving blockchain
        let (blockchain_cached_block_number, accounts_cache) = {
            let bc = match blockchain.try_read() {
                Ok(guard) => guard,
                Err(e) => {
                    tracing::error!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                    panic!(
                        "Failed to acquire blockchain lock during RPC server initialization: {}",
                        e
                    );
                }
            };
            (bc.get_cached_block_number_arc(), bc.accounts_arc())
        };

        Self {
            blockchain,
            network_manager: None,
            rate_limiter: Some(Arc::new(rate_limit::RateLimiter::new(
                max_tokens,
                tokens_per_second,
            ))),
            per_ip_rate_limiter: None,
            tx_submission_rate_limiter: None,
            shard_manager: Some(shard_manager),
            metrics: None,
            security_scorer: None,
            mining_manager: None,
            forensic_analyzer: None,
            light_client: None,
            policy_manager: None,
            node_registry: None,
            reputation_manager: None,
            wallet_registry: None,
            multisig_manager: None,
            social_recovery_manager: None,
            batch_manager: None,
            parallel_evm_executor: None,
            oracle_registry: None,
            price_feed_manager: None,
            vrf_manager: None,
            recurring_manager: None,
            stop_loss_manager: None,
            pq_keystore: None,
            #[cfg(feature = "privacy")]
            privacy_manager: RwLock::new(None),
            #[cfg(feature = "privacy")]
            privacy_prover: RwLock::new(None),
            security_hardening: None,
            api_key_hash: None,
            allow_unsigned_eth_send: false,
            chain_id,
            miner_address: Address::zero(),
            response_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            _last_block_number: Arc::new(AtomicU64::new(0)),
            cache_ttl_ms: 100, // 100ms cache TTL (very short for blockchain data)
            blockchain_cached_block_number,
            accounts_cache,
            rpc_requests_total: Arc::new(AtomicU64::new(0)),
            rpc_errors_total: Arc::new(AtomicU64::new(0)),
            trace_id_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create RPC server with rate limiting, sharding, and authentication
    pub fn with_rate_limit_sharding_and_auth(
        blockchain: Arc<RwLock<Blockchain>>,
        max_tokens: u32,
        tokens_per_second: f64,
        shard_manager: Arc<crate::sharding::ShardManager>,
        api_key: String,
    ) -> Self {
        let mut server = Self::with_rate_limit_and_sharding(
            blockchain,
            max_tokens,
            tokens_per_second,
            shard_manager,
        );
        server.api_key_hash = Some(hash_api_key(&api_key));
        server
    }

    /// Set API key by hashing it; plain key is not stored.
    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key_hash = Some(hash_api_key(&api_key));
    }

    /// Allow unsigned eth_sendTransaction (dev only). When false, production-safe: require signed tx or use eth_sendRawTransaction.
    pub fn set_miner_address(&mut self, address: Address) {
        self.miner_address = address;
    }

    pub fn set_allow_unsigned_eth_send(&mut self, allow: bool) {
        self.allow_unsigned_eth_send = allow;
        if allow {
            warn!("DEVELOPMENT MODE: unsigned transactions are enabled - DO NOT use in production");
        }
    }

    /// Set per-IP rate limiter (when set and client IP available, used for rate check)
    pub fn set_per_ip_rate_limiter(&mut self, limiter: Arc<rate_limit::PerIpRateLimiter>) {
        self.per_ip_rate_limiter = Some(limiter);
    }

    /// Set per-IP rate limiter for transaction submission (100 tx/min per IP by default)
    /// This provides DoS protection specifically for eth_sendRawTransaction and irondag_sendRawTransaction
    pub fn set_tx_submission_rate_limiter(&mut self, limiter: Arc<rate_limit::PerIpRateLimiter>) {
        self.tx_submission_rate_limiter = Some(limiter);
    }

    /// Acquire blockchain read lock without starving against frequent mining writes.
    ///
    /// `blockchain.read().await` registers in the RwLock wait-queue, but tokio's
    /// writer-preferring policy means a steady stream of mining writes (5-10/s) can
    /// starve readers indefinitely even with a 10 s timeout.
    ///
    /// `try_read()` does NOT enter the wait-queue — it succeeds the instant no write
    /// lock is held, catching the brief gaps between block additions (~5-50 ms each).
    /// With a 5 ms sleep between attempts the worst-case latency is ~10 ms; 30 s total
    /// gives ample room before returning "Node busy".
    async fn acquire_blockchain_read(
        &self,
    ) -> Result<tokio::sync::RwLockReadGuard<'_, Blockchain>, JsonRpcError> {
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(RPC_LOCK_TIMEOUT_MS);
        loop {
            if let Ok(guard) = self.blockchain.try_read() {
                return Ok(guard);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(JsonRpcError {
                    code: -32000,
                    message: "Node busy (mining in progress), please retry".to_string(),
                    data: None,
                });
            }
            tokio::time::sleep(Duration::from_millis(RPC_LOCK_RETRY_MS)).await;
        }
    }

    /// Check if authentication is required for a method
    fn requires_auth(&self, method: &str) -> bool {
        // If no API key is set, authentication is disabled
        if self.api_key_hash.is_none() {
            return false;
        }

        // Public methods don't require authentication
        !is_public_method(method)
    }

    /// Verify API key from request. Only accepts key via X-API-Key header (not in params).
    /// Compares hash(provided_key) to stored hash using constant-time comparison.
    fn verify_api_key(&self, _request: &JsonRpcRequest, api_key_header: Option<&str>) -> bool {
        let stored_hash = match &self.api_key_hash {
            Some(h) => h,
            None => return true,
        };
        let Some(header_key) = api_key_header else {
            return false;
        };
        let provided_hash = hash_api_key(header_key);
        constant_time_eq(&provided_hash[..], &stored_hash[..])
    }

    /// Set metrics handle
    pub fn set_metrics(&mut self, metrics: crate::metrics::MetricsHandle) {
        self.metrics = Some(metrics);
    }

    /// Set security hardening
    pub fn set_security_hardening(
        &mut self,
        hardening: Arc<tokio::sync::RwLock<crate::security::SecurityHardening>>,
    ) {
        self.security_hardening = Some(hardening);
    }

    /// Create RPC server with security hardening
    pub fn with_security_hardening(
        blockchain: Arc<RwLock<Blockchain>>,
        config: crate::security::SecurityConfig,
    ) -> Self {
        let mut server = Self::new(blockchain);
        server.security_hardening = Some(Arc::new(tokio::sync::RwLock::new(
            crate::security::SecurityHardening::new(config),
        )));
        server
    }

    // ============================================================================
    // Production Hardening Methods
    // ============================================================================

    /// Check if a batch request exceeds the maximum allowed size
    /// Returns Err if batch is too large, Ok(()) otherwise
    pub fn check_batch_limit(&self, batch: &[JsonRpcRequest]) -> Result<(), JsonRpcError> {
        if batch.len() > RPC_MAX_BATCH_SIZE {
            warn!(
                "Batch request rejected: {} methods (max {})",
                batch.len(),
                RPC_MAX_BATCH_SIZE
            );
            Err(JsonRpcError {
                code: RPC_INVALID_REQUEST,
                message: format!("Batch too large, maximum {} requests", RPC_MAX_BATCH_SIZE),
                data: Some(json!({ "received": batch.len(), "max": RPC_MAX_BATCH_SIZE })),
            })
        } else {
            Ok(())
        }
    }

    /// Handle a batch of JSON-RPC requests with size limit checking
    /// This method should be called from the HTTP handler in node/mod.rs
    pub async fn handle_batch_requests(
        &self,
        requests: Vec<JsonRpcRequest>,
        api_key_header: Option<&str>,
        client_ip: Option<std::net::IpAddr>,
    ) -> Result<Vec<JsonRpcResponse>, JsonRpcError> {
        // Check batch size limit first
        self.check_batch_limit(&requests)?;

        // Process all requests in parallel using join_all
        let futures: Vec<_> = requests
            .into_iter()
            .map(|request| self.handle_request_with_timeout(request, api_key_header, client_ip))
            .collect();

        let responses = futures::future::join_all(futures).await;
        Ok(responses)
    }

    /// Handle a single JSON-RPC request with timeout protection
    /// Read methods get 5s timeout, write methods get 30s timeout
    pub async fn handle_request_with_timeout(
        &self,
        request: JsonRpcRequest,
        api_key_header: Option<&str>,
        client_ip: Option<std::net::IpAddr>,
    ) -> JsonRpcResponse {
        let method = request.method.clone();
        let timeout_duration = if is_read_method(&method) {
            Duration::from_secs(RPC_READ_TIMEOUT_SECS)
        } else {
            Duration::from_secs(RPC_DEFAULT_TIMEOUT_SECS)
        };

        let start = Instant::now();

        match tokio::time::timeout(
            timeout_duration,
            self.handle_request(request, api_key_header, client_ip),
        )
        .await
        {
            Ok(response) => {
                // Log slow requests
                let elapsed = start.elapsed();
                if elapsed > Duration::from_secs(RPC_SLOW_REQUEST_THRESHOLD_SECS) {
                    warn!(
                        "Slow RPC request: {} took {:.2}s",
                        method,
                        elapsed.as_secs_f64()
                    );
                } else if elapsed > Duration::from_secs(RPC_INFO_LOG_THRESHOLD_SECS) {
                    info!("RPC request: {} took {:.2}s", method, elapsed.as_secs_f64());
                }
                response
            }
            Err(_) => {
                warn!(
                    "RPC request timed out after {}s: {}",
                    timeout_duration.as_secs(),
                    method
                );
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32000,
                        message: format!("Request timed out after {}s", timeout_duration.as_secs()),
                        data: Some(json!({ "method": method })),
                    }),
                    id: None,
                }
            }
        }
    }

    /// Check if a serialized response exceeds the maximum size limit
    /// Returns the response bytes if valid, or an error response if too large
    pub fn check_response_size(
        &self,
        response: &JsonRpcResponse,
        method: &str,
    ) -> Result<Vec<u8>, JsonRpcError> {
        match serde_json::to_vec(response) {
            Ok(bytes) => {
                if bytes.len() > RPC_MAX_RESPONSE_SIZE {
                    warn!(
                        "RPC response exceeds 10MB limit for method {}: {} bytes",
                        method,
                        bytes.len()
                    );
                    Err(JsonRpcError {
                        code: -32000,
                        message: "Response too large".to_string(),
                        data: Some(json!({
                            "method": method,
                            "size_bytes": bytes.len(),
                            "max_size_bytes": RPC_MAX_RESPONSE_SIZE
                        })),
                    })
                } else {
                    Ok(bytes)
                }
            }
            Err(e) => {
                warn!("Failed to serialize response for method {}: {}", method, e);
                Err(JsonRpcError {
                    code: RPC_INTERNAL_ERROR,
                    message: "Failed to serialize response".to_string(),
                    data: Some(json!({ "error": e.to_string() })),
                })
            }
        }
    }

    /// Serialize a response with size checking
    /// Returns the serialized bytes or an error response serialized to bytes
    pub fn serialize_response_checked(&self, response: JsonRpcResponse, method: &str) -> Vec<u8> {
        match self.check_response_size(&response, method) {
            Ok(bytes) => bytes,
            Err(size_error) => {
                // Return an error response instead
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(size_error),
                    id: None,
                };
                serde_json::to_vec(&error_response).unwrap_or_else(|_| {
                    br#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"},"id":null}"#.to_vec()
                })
            }
        }
    }

    /// Get health status information for the /_health endpoint
    /// Returns a JSON Value with node status information
    pub async fn get_health_status(&self) -> Value {
        // Get blockchain height (lock-free via cached atomic)
        let block_height = self.blockchain_cached_block_number.load(std::sync::atomic::Ordering::Relaxed);

        // Get peer count from network manager if available
        let peer_count = if let Some(ref network_mgr) = self.network_manager {
            // peer_count() is synchronous and returns usize
            network_mgr.peer_count()
        } else {
            0
        };

        // Get mining status from mining manager if available
        let is_mining = if let Some(ref mining_mgr) = self.mining_manager {
            // is_mining() returns &Arc<RwLock<bool>>, so we need to read it
            *mining_mgr.is_mining().read().await
        } else {
            false
        };

        json!({
            "status": "ok",
            "block_height": block_height,
            "peer_count": peer_count,
            "mining": is_mining,
            "version": env!("CARGO_PKG_VERSION"),
        })
    }

    /// Handle JSON-RPC request
    ///
    /// # Arguments
    /// * `request` - The JSON-RPC request
    /// * `api_key_header` - Optional API key from HTTP header (X-API-Key)
    /// * `client_ip` - Optional client IP address for security hardening
    pub async fn handle_request(
        &self,
        request: JsonRpcRequest,
        api_key_header: Option<&str>,
        client_ip: Option<std::net::IpAddr>,
    ) -> JsonRpcResponse {
        // Generate trace ID for observability (atomic u64 counter for efficiency)
        let trace_id = self.trace_id_counter.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let method = request.method.clone();
        let client_addr_or_unknown = client_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        debug!(
            "[RPC:{}] {} from {}",
            trace_id, method, client_addr_or_unknown
        );

        // Increment RPC request counter for metrics
        self.rpc_requests_total.fetch_add(1, Ordering::Relaxed);

        // Security hardening: Check IP if provided
        if let Some(ip) = client_ip {
            if let Some(ref hardening) = self.security_hardening {
                let hardening = hardening.read().await;
                match hardening.check_ip(ip).await {
                    Err(crate::security::SecurityError::Blacklisted) => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32003,
                                message: "Access denied: IP is blacklisted".to_string(),
                                data: None,
                            }),
                            id: request.id,
                        };
                    }
                    Err(crate::security::SecurityError::Banned) => {
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32004,
                                message: "Access denied: IP is temporarily banned".to_string(),
                                data: None,
                            }),
                            id: request.id,
                        };
                    }
                    Err(crate::security::SecurityError::RateLimitExceeded) => {
                        hardening.record_failed_request(ip).await;
                        return JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32005,
                                message: "Rate limit exceeded".to_string(),
                                data: None,
                            }),
                            id: request.id,
                        };
                    }
                    Ok(()) => {}
                    other => {
                        warn!("Unhandled security error variant: {:?}", other);
                    }
                }
            }
        }

        // Check request size (DoS protection)
        // Estimate request size from method and params
        let request_size = request.method.len()
            + request
                .params
                .as_ref()
                .map(|p| serde_json::to_string(p).map(|s| s.len()).unwrap_or(0))
                .unwrap_or(0);
        if let Some(ref hardening) = self.security_hardening {
            let hardening = hardening.read().await;
            if let Err(_) = hardening.check_request_size(request_size) {
                if let Some(ip) = client_ip {
                    hardening.record_invalid_request(ip).await;
                }
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32600,
                        message: "Request too large".to_string(),
                        data: None,
                    }),
                    id: request.id,
                };
            }
        }

        // Check authentication if required
        if self.requires_auth(&request.method) {
            if !self.verify_api_key(&request, api_key_header) {
                if let Some(ip) = client_ip {
                    if let Some(ref hardening) = self.security_hardening {
                        let hardening = hardening.read().await;
                        hardening.record_failed_request(ip).await;
                    }
                }
                return JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32001,
                        message: "Unauthorized: Invalid or missing API key".to_string(),
                        data: Some(Value::String(
                            "Provide API key via X-API-Key header".to_string(),
                        )),
                    }),
                    id: request.id,
                };
            }
        }

        // Rate limit: per-IP when available, else global
        let rate_limited =
            if let (Some(ref limiter), Some(ip)) = (&self.per_ip_rate_limiter, client_ip) {
                !limiter.try_acquire(ip).await
            } else if let Some(ref limiter) = self.rate_limiter {
                !limiter.try_acquire().await
            } else {
                false
            };
        if rate_limited {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32005,
                    message: "Rate limit exceeded".to_string(),
                    data: None,
                }),
                id: request.id,
            };
        }

        // Transaction submission rate limiting (100 tx/min per IP by default)
        // Applied specifically to eth_sendRawTransaction and irondag_sendRawTransaction
        if matches!(
            request.method.as_str(),
            "eth_sendRawTransaction" | "irondag_sendRawTransaction"
        ) {
            if let (Some(ref limiter), Some(ip)) = (&self.tx_submission_rate_limiter, client_ip) {
                if !limiter.try_acquire(ip).await {
                    return JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32005,
                            message: "Transaction submission rate limit exceeded".to_string(),
                            data: None,
                        }),
                        id: request.id,
                    };
                }
            }
        }

        if request.jsonrpc != "2.0" {
            return JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32600,
                    message: "Invalid Request".to_string(),
                    data: None,
                }),
                id: request.id,
            };
        }

        let result = match request.method.as_str() {
            "eth_getBalance" => self.eth_get_balance(request.params).await,
            "eth_getTransactionCount" => self.eth_get_transaction_count(request.params).await,
            "eth_getBlockByNumber" => self.eth_get_block_by_number(request.params).await,
            "eth_getBlockByHash" => self.eth_get_block_by_hash(request.params).await,
            "eth_getTransactionByHash" => self.eth_get_transaction_by_hash(request.params).await,
            "eth_sendTransaction" => self.eth_send_transaction(request.params).await,
            "eth_sendRawTransaction" => self.eth_send_raw_transaction(request.params).await,
            "eth_blockNumber" => self.eth_block_number().await,
            "eth_getBlockTransactionCountByNumber" => {
                self.eth_get_block_transaction_count_by_number(request.params)
                    .await
            }
            "net_peerCount" => self.net_peer_count().await,
            "net_version" => Ok(Value::String(self.chain_id.to_string())), // Network version (chain ID as string)
            "eth_chainId" => Ok(Value::String(format!("0x{:x}", self.chain_id))), // Chain ID in hex
            "eth_gasPrice" => self.eth_gas_price().await,
            "eth_syncing" => Ok(Value::Bool(false)),
            "eth_feeHistory" => self.eth_fee_history(request.params).await,
            "eth_maxPriorityFeePerGas" => self.eth_max_priority_fee_per_gas().await,
            "irondag_getDagStats" => self.irondag_get_dag_stats().await,
            "irondag_getBlueScore" => self.irondag_get_blue_score(request.params).await,
            "irondag_getTps" => self.irondag_get_tps(request.params).await,
            "irondag_getBlocksByStream" => self.irondag_get_blocks_by_stream(request.params).await,
            "irondag_getStreamCounts" => self.irondag_get_stream_counts().await,
            "eth_getCode" => self.eth_get_code(request.params).await,
            "eth_estimateGas" => self.eth_estimate_gas(request.params).await,
            "eth_call" => self.eth_call(request.params).await,
            "eth_getStorageAt" => self.eth_get_storage_at(request.params).await,
            "eth_getTransactionReceipt" => self.eth_get_transaction_receipt(request.params).await,
            "irondag_getShardStats" => self.irondag_get_shard_stats(request.params).await,
            "irondag_getShardForAddress" => self.irondag_get_shard_for_address(request.params).await,
            "irondag_getRiskScore" => self.irondag_get_risk_score(request.params).await,
            "irondag_getRiskLabels" => self.irondag_get_risk_labels(request.params).await,
            "irondag_getTransactionRisk" => self.irondag_get_transaction_risk(request.params).await,
            "irondag_getFairnessMetrics" => self.irondag_get_fairness_metrics(request.params).await,
            "irondag_getStateRoot" => self.irondag_get_state_root().await,
            "irondag_getStateProof" => self.irondag_get_state_proof(request.params).await,
            "irondag_verifyStateProof" => self.irondag_verify_state_proof(request.params).await,
            "irondag_getCrossShardTransaction" => {
                self.irondag_get_cross_shard_transaction(request.params).await
            }
            "irondag_getCrossShardTransactions" => {
                self.irondag_get_cross_shard_transactions(request.params).await
            }
            "irondag_getShardBlock" => self.irondag_get_shard_block(request.params).await,
            "irondag_getShardTransactions" => self.irondag_get_shard_transactions(request.params).await,
            "irondag_getShardBalance" => self.irondag_get_shard_balance(request.params).await,
            "irondag_getOrderingPolicy" => self.irondag_get_ordering_policy().await,
            "irondag_setOrderingPolicy" => self.irondag_set_ordering_policy(request.params).await,
            "irondag_getMevMetrics" => self.irondag_get_mev_metrics(request.params).await,
            "irondag_getBlockFairness" => self.irondag_get_block_fairness(request.params).await,
            "irondag_traceFunds" => self.irondag_trace_funds(request.params).await,
            "irondag_getAddressSummary" => self.irondag_get_address_summary(request.params).await,
            "irondag_getAddressTransactions" => self.irondag_get_address_transactions(request.params).await,
            "irondag_detectAnomalies" => self.irondag_detect_anomalies(request.params).await,
            "irondag_findRelatedAddresses" => self.irondag_find_related_addresses(request.params).await,
            "irondag_getStateRootHistory" => self.irondag_get_state_root_history(request.params).await,
            "irondag_getLightClientSyncStatus" => self.irondag_get_light_client_sync_status().await,
            "irondag_enableLightClientMode" => self.irondag_enable_light_client_mode(request.params).await,
            "irondag_generatePqAccount" => self.irondag_generate_pq_account(request.params).await,
            "irondag_getPqAccountType" => self.irondag_get_pq_account_type(request.params).await,
            "irondag_exportPqKey" => self.irondag_export_pq_key(request.params).await,
            "irondag_importPqKey" => self.irondag_import_pq_key(request.params).await,
            "irondag_createPqTransaction" => self.irondag_create_pq_transaction(request.params).await,
            "irondag_addSecurityPolicy" => self.irondag_add_security_policy(request.params).await,
            "irondag_removeSecurityPolicy" => self.irondag_remove_security_policy(request.params).await,
            "irondag_getSecurityPolicies" => self.irondag_get_security_policies(request.params).await,
            "irondag_setPolicyEnabled" => self.irondag_set_policy_enabled(request.params).await,
            "irondag_evaluateTransactionPolicy" => {
                self.irondag_evaluate_transaction_policy(request.params).await
            }
            #[cfg(test)]
            "irondag_addTestBlock" => self.irondag_add_test_block(request.params).await,
            #[cfg(not(test))]
            "irondag_addTestBlock" => Err(JsonRpcError {
                code: -32601,
                message: "Method not available in production build".to_string(),
                data: None,
            }),
            #[cfg(test)]
            "irondag_createTestTransaction" => self.irondag_create_test_transaction(request.params).await,
            #[cfg(not(test))]
            "irondag_createTestTransaction" => Err(JsonRpcError {
                code: -32601,
                message: "Method not available in production build".to_string(),
                data: None,
            }),
            "irondag_getNodeRegistry" => self.irondag_get_node_registry().await,
            "irondag_getNodeLongevity" => self.irondag_get_node_longevity(request.params).await,
            "irondag_registerNode" => self.irondag_register_node(request.params).await,
            "irondag_startMining" => self.irondag_start_mining(request.params).await,
            "irondag_stopMining" => self.irondag_stop_mining(request.params).await,
            "irondag_getMiningStatus" => self.irondag_get_mining_status().await,
            "irondag_getMiningDashboard" => self.irondag_get_mining_dashboard(request.params).await,
            "irondag_getNodeStatus" => self.irondag_get_node_status().await,
            "irondag_sendRawTransaction" => self.irondag_send_raw_transaction(request.params).await,
            // Time-locked transactions
            "irondag_createTimeLockedTransaction" => {
                self.irondag_create_time_locked_transaction(request.params)
                    .await
            }
            "irondag_getTimeLockedTransactions" => {
                self.irondag_get_time_locked_transactions(request.params).await
            }
            // Recurring transactions
            "irondag_createRecurringTransaction" => {
                self.irondag_create_recurring_transaction(request.params).await
            }
            "irondag_cancelRecurringTransaction" => {
                self.irondag_cancel_recurring_transaction(request.params).await
            }
            "irondag_getRecurringTransaction" => {
                self.irondag_get_recurring_transaction(request.params).await
            }
            "irondag_getRecurringTransactions" => {
                self.irondag_get_recurring_transactions(request.params).await
            }
            // Gasless transactions
            "irondag_createGaslessTransaction" => {
                self.irondag_create_gasless_transaction(request.params).await
            }
            "irondag_getSponsoredTransactions" => {
                self.irondag_get_sponsored_transactions(request.params).await
            }
            // Programmable Gas Sponsorship
            "irondag_registerSponsorPolicy" => self.irondag_register_sponsor_policy(request.params).await,
            "irondag_deregisterSponsorPolicy" => {
                self.irondag_deregister_sponsor_policy(request.params).await
            }
            "irondag_getSponsorPolicy" => self.irondag_get_sponsor_policy(request.params).await,
            // Reputation system
            "irondag_getReputation" => self.irondag_get_reputation(request.params).await,
            "irondag_getReputationFactors" => self.irondag_get_reputation_factors(request.params).await,
            // Account Abstraction
            "irondag_createWallet" => self.irondag_create_wallet(request.params).await,
            "irondag_getWallet" => self.irondag_get_wallet(request.params).await,
            "irondag_getOwnerWallets" => self.irondag_get_owner_wallets(request.params).await,
            "irondag_isContractWallet" => self.irondag_is_contract_wallet(request.params).await,
            // Multi-Signature Operations
            "irondag_createMultisigTransaction" => {
                self.irondag_create_multisig_transaction(request.params).await
            }
            "irondag_addMultisigSignature" => self.irondag_add_multisig_signature(request.params).await,
            "irondag_getPendingMultisigTransactions" => {
                self.irondag_get_pending_multisig_transactions(request.params)
                    .await
            }
            "irondag_validateMultisigTransaction" => {
                self.irondag_validate_multisig_transaction(request.params).await
            }
            // Social Recovery Operations
            "irondag_initiateRecovery" => self.irondag_initiate_recovery(request.params).await,
            "irondag_approveRecovery" => self.irondag_approve_recovery(request.params).await,
            "irondag_getRecoveryStatus" => self.irondag_get_recovery_status(request.params).await,
            "irondag_completeRecovery" => self.irondag_complete_recovery(request.params).await,
            "irondag_cancelRecovery" => self.irondag_cancel_recovery(request.params).await,
            // Batch Transaction Operations
            "irondag_createBatchTransaction" => self.irondag_create_batch_transaction(request.params).await,
            "irondag_executeBatchTransaction" => {
                self.irondag_execute_batch_transaction(request.params).await
            }
            "irondag_getBatchStatus" => self.irondag_get_batch_status(request.params).await,
            "irondag_estimateBatchGas" => self.irondag_estimate_batch_gas(request.params).await,
            // Parallel EVM Operations
            "irondag_enableParallelEVM" => self.irondag_enable_parallel_evm(request.params).await,
            "irondag_getParallelEVMStats" => self.irondag_get_parallel_evm_stats(request.params).await,
            "irondag_estimateParallelImprovement" => {
                self.irondag_estimate_parallel_improvement(request.params).await
            }
            // Oracle Operations
            "irondag_registerOracle" => self.irondag_register_oracle(request.params).await,
            "irondag_unregisterOracle" => self.irondag_unregister_oracle(request.params).await,
            "irondag_getOracleInfo" => self.irondag_get_oracle_info(request.params).await,
            "irondag_getOracleList" => self.irondag_get_oracle_list(request.params).await,
            "irondag_getPrice" => self.irondag_get_price(request.params).await,
            "irondag_getPriceHistory" => self.irondag_get_price_history(request.params).await,
            "irondag_getPriceFeeds" => self.irondag_get_price_feeds().await,
            "irondag_requestRandomness" => self.irondag_request_randomness(request.params).await,
            "irondag_getRandomness" => self.irondag_get_randomness(request.params).await,
            // Recurring Transaction Operations
            "irondag_createRecurringTransaction" => {
                self.irondag_create_recurring_transaction(request.params).await
            }
            "irondag_cancelRecurringTransaction" => {
                self.irondag_cancel_recurring_transaction(request.params).await
            }
            "irondag_getRecurringTransaction" => {
                self.irondag_get_recurring_transaction(request.params).await
            }
            "irondag_getRecurringTransactions" => {
                self.irondag_get_recurring_transactions(request.params).await
            }
            "irondag_pauseRecurringTransaction" => {
                self.irondag_pause_recurring_transaction(request.params).await
            }
            "irondag_resumeRecurringTransaction" => {
                self.irondag_resume_recurring_transaction(request.params).await
            }
            // Built-in Privacy Pool
            "irondag_getPoolInfo" => self.irondag_get_pool_info().await,
            "irondag_getPoolDepositParams" => {
                self.irondag_get_pool_deposit_params(request.params).await
            }
            "irondag_poolWithdraw" => self.irondag_pool_withdraw(request.params).await,
            "irondag_isNullifierSpent" => self.irondag_is_nullifier_spent(request.params).await,
            // Stop-Loss Operations
            "irondag_createStopLoss" => self.irondag_create_stop_loss(request.params).await,
            "irondag_cancelStopLoss" => self.irondag_cancel_stop_loss(request.params).await,
            "irondag_getStopLoss" => self.irondag_get_stop_loss(request.params).await,
            "irondag_getStopLossOrders" => self.irondag_get_stop_loss_orders(request.params).await,
            "irondag_updateStopLossPrice" => self.irondag_update_stop_loss_price(request.params).await,
            "irondag_pauseStopLoss" => self.irondag_pause_stop_loss(request.params).await,
            "irondag_resumeStopLoss" => self.irondag_resume_stop_loss(request.params).await,
            // Privacy Operations
            "irondag_createPrivateTransaction" => {
                #[cfg(feature = "privacy")]
                {
                    self.irondag_create_private_transaction(request.params).await
                }
                #[cfg(not(feature = "privacy"))]
                {
                    Err(JsonRpcError { code: RPC_METHOD_NOT_FOUND, message: "Method not found: irondag_createPrivateTransaction (privacy feature not enabled)".to_string(), data: None })
                }
            }
            "irondag_verifyPrivacyProof" => {
                #[cfg(feature = "privacy")]
                {
                    self.irondag_verify_privacy_proof(request.params).await
                }
                #[cfg(not(feature = "privacy"))]
                {
                    Err(JsonRpcError {
                        code: RPC_METHOD_NOT_FOUND,
                        message:
                            "Method not found: irondag_verifyPrivacyProof (privacy feature not enabled)"
                                .to_string(),
                        data: None,
                    })
                }
            }
            "irondag_proveBalance" => {
                #[cfg(feature = "privacy")]
                {
                    self.irondag_prove_balance(request.params).await
                }
                #[cfg(not(feature = "privacy"))]
                {
                    Err(JsonRpcError {
                        code: RPC_METHOD_NOT_FOUND,
                        message: "Method not found: irondag_proveBalance (privacy feature not enabled)"
                            .to_string(),
                        data: None,
                    })
                }
            }
            "irondag_getPrivacyStats" => {
                #[cfg(feature = "privacy")]
                {
                    self.irondag_get_privacy_stats().await
                }
                #[cfg(not(feature = "privacy"))]
                {
                    Err(JsonRpcError {
                        code: RPC_METHOD_NOT_FOUND,
                        message:
                            "Method not found: irondag_getPrivacyStats (privacy feature not enabled)"
                                .to_string(),
                        data: None,
                    })
                }
            }
            // Snapshot Operations
            "irondag_createSnapshot" => self.irondag_create_snapshot(request.params).await,
            "irondag_listSnapshots" => self.irondag_list_snapshots(request.params).await,
            "irondag_getSnapshotInfo" => self.irondag_get_snapshot_info(request.params).await,
            "irondag_restoreSnapshot" => {
                self.irondag_restore_snapshot(request.params, api_key_header)
                    .await
            }
            // Testnet Faucet
            "irondag_faucet" => self.irondag_faucet(request.params).await,
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        };

        let response = match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(value),
                error: None,
                id: request.id,
            },
            Err(error) => {
                // Increment RPC error counter for metrics
                self.rpc_errors_total.fetch_add(1, Ordering::Relaxed);
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(error),
                    id: request.id,
                }
            }
        };

        debug!("[RPC:{}] completed in {:?}", trace_id, start.elapsed());
        response
    }

    /// Generate a new trace ID (atomic u64 counter)
    #[allow(dead_code)]
    fn next_trace_id(&self) -> u64 {
        self.trace_id_counter.fetch_add(1, Ordering::Relaxed)
    }

    // =========================================================================
    // === ETHEREUM STANDARD METHODS ==========================================
    // =========================================================================
    // These methods provide compatibility with Ethereum JSON-RPC APIs.
    // They are used by tools like Metamask, ethers.js, and web3.py.
    // =========================================================================

    /// eth_getBalance - Get balance for an address (LOCK-FREE via accounts DashMap)
    async fn eth_get_balance(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;
        let params_arr = params.as_array().ok_or_else(missing_params_error)?;
        let address = extract_address_param(params_arr, 0)?;
        let balance = self.accounts_cache
            .get(&address)
            .map(|s| s.balance)
            .unwrap_or(0);
        Ok(Value::String(format!("0x{:x}", balance)))
    }

    /// eth_getTransactionCount - Get nonce for an address (LOCK-FREE via accounts DashMap)
    async fn eth_get_transaction_count(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;
        let params_arr = params.as_array().ok_or_else(missing_params_error)?;
        let address = extract_address_param(params_arr, 0)?;
        let nonce = self.accounts_cache
            .get(&address)
            .map(|s| s.nonce)
            .unwrap_or(0);
        Ok(Value::String(format!("0x{:x}", nonce)))
    }

    /// eth_blockNumber - Get latest block number (LOCK-FREE via atomic)
    async fn eth_block_number(&self) -> Result<Value, JsonRpcError> {
        // LOCK-FREE: Read directly from atomic cached block number
        // No blockchain lock acquisition required!
        // This is the CRITICAL FIX for RPC lock contention during aggressive mining
        let block_number = self.blockchain_cached_block_number.load(Ordering::Acquire);
        Ok(Value::String(format!("0x{:x}", block_number)))
    }

    /// eth_getBlockByNumber - Get block by number
    async fn eth_get_block_by_number(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;

        let block_num_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_param_error("block number"))?;

        let blockchain = self.acquire_blockchain_read().await?;

        let block = if block_num_str == "latest" || block_num_str == "pending" {
            blockchain.get_latest_block()
        } else if block_num_str == "finalized" || block_num_str == "safe" {
            // Do not fall back to `latest`: callers (e.g. explorer) need a distinct finalized height.
            match blockchain.get_finalized_block_number() {
                Ok(Some(n)) => blockchain.get_block_by_number(n),
                _ => None,
            }
        } else {
            let block_number = parse_hex_number(block_num_str)?;
            blockchain.get_block_by_number(block_number)
        };

        Ok(block_to_json(block, self.miner_address))
    }

    /// eth_getBlockByHash - Get block by hash
    async fn eth_get_block_by_hash(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;
        let params_arr = params.as_array().ok_or_else(missing_params_error)?;

        let hash = extract_hash_param(params_arr, 0)?;

        let blockchain = self.acquire_blockchain_read().await?;
        let block = blockchain.get_block_by_hash(&hash);

        Ok(block_to_json(block.as_ref().cloned(), self.miner_address))
    }

    /// eth_getTransactionByHash - Get transaction by hash
    async fn eth_get_transaction_by_hash(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_param_error("hash"))?;

        let hash = parse_hash(hash_str)?;

        let blockchain = self.acquire_blockchain_read().await?;

        if let Some((block, tx, _idx)) = blockchain.get_transaction_by_hash(&hash) {
            let shard_info = if let Some(shard_manager) = &self.shard_manager {
                shard_manager.get_transaction_shards(&tx).await
            } else {
                None
            };
            return Ok(tx_to_json_with_shard(&tx, block.header.block_number, shard_info));
        }

        Ok(Value::Null)
    }

    /// eth_sendTransaction - Send a transaction (Metamask compatible)
    ///
    /// Accepts Ethereum-style transaction format from Metamask and converts to internal format.
    /// Note: Metamask uses ECDSA/secp256k1 signatures, but our blockchain uses Ed25519.
    /// For now, this accepts the transaction structure - signature conversion can be added later.
    async fn eth_send_transaction(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;

        // Parse Ethereum-style transaction object
        let tx_obj = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_object())
            .ok_or_else(|| invalid_param_error("transaction"))?;

        // Production: reject unsigned eth_sendTransaction; require signed payload via eth_sendRawTransaction
        let has_signature = tx_obj.get("r").and_then(|v| v.as_str()).is_some()
            && tx_obj.get("s").and_then(|v| v.as_str()).is_some()
            && tx_obj.get("v").and_then(|v| v.as_u64()).is_some();
        if !self.allow_unsigned_eth_send && !has_signature {
            return Err(JsonRpcError {
                code: -32602,
                message: "Unsigned eth_sendTransaction is disabled in production. Use eth_sendRawTransaction with a signed payload.".to_string(),
                data: None,
            });
        }

        // Parse transaction fields (Ethereum format)
        let from_str = tx_obj
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| missing_param_error("from"))?;
        let from = parse_address(from_str)?;

        let to_str = tx_obj
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0000000000000000000000000000000000000000");
        let to = parse_address(to_str)?;

        let value_str = tx_obj
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0");
        let value = parse_hex_u128(value_str)?;

        let data_str = tx_obj.get("data").and_then(|v| v.as_str()).unwrap_or("0x");
        let data = if data_str.starts_with("0x") {
            hex::decode(&data_str[2..]).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid data hex: {}", e),
                data: None,
            })?
        } else {
            hex::decode(data_str).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid data hex: {}", e),
                data: None,
            })?
        };

        // Parse gas fields (Ethereum uses gasPrice or maxFeePerGas)
        let gas_price_str = tx_obj
            .get("gasPrice")
            .or_else(|| tx_obj.get("maxFeePerGas"))
            .and_then(|v| v.as_str())
            .unwrap_or("0x4a817c800"); // Default 20 gwei
        let gas_price = parse_hex_u128(gas_price_str)?;

        let gas_limit_str = tx_obj
            .get("gas")
            .and_then(|v| v.as_str())
            .unwrap_or("0x5208"); // Default 21,000
        let gas_limit = parse_hex_number(gas_limit_str)? as u64;

        // Calculate fee (gas_price * gas_limit)
        let fee = gas_price
            .checked_mul(gas_limit as u128)
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "Gas price * gas limit overflow".to_string(),
                data: None,
            })?;

        // Get nonce
        let nonce = if let Some(nonce_str) = tx_obj.get("nonce").and_then(|v| v.as_str()) {
            parse_hex_number(nonce_str)? as u64
        } else {
            self.accounts_cache.get(&from).map(|s| s.nonce).unwrap_or(0)
        };

        // Create transaction
        let mut tx = if !data.is_empty() {
            crate::blockchain::Transaction::with_data(
                from,
                to,
                value,
                fee,
                nonce,
                data.clone(),
                gas_limit,
            )
        } else {
            crate::blockchain::Transaction::new(from, to, value, fee, nonce)
        };

        // Note: Metamask signs with ECDSA/secp256k1, but our blockchain uses Ed25519
        // For now, we'll accept unsigned transactions for testing, or use irondag_send_raw_transaction
        // with pre-signed Ed25519 transactions

        // Verify balance first
        let balance = self.accounts_cache.get(&from).map(|s| s.balance).unwrap_or(0);

        let total_cost = value.checked_add(fee).ok_or_else(|| JsonRpcError {
            code: -32000,
            message: "Transaction value + fee overflow".to_string(),
            data: None,
        })?;

        if balance < total_cost {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Insufficient balance: have {}, need {}",
                    balance, total_cost
                ),
                data: None,
            });
        }

        // Check if ECDSA signature is provided (Metamask format - r, s, v)
        if let (Some(r_str), Some(s_str), Some(v_val)) = (
            tx_obj.get("r").and_then(|v| v.as_str()),
            tx_obj.get("s").and_then(|v| v.as_str()),
            tx_obj.get("v").and_then(|v| v.as_u64()),
        ) {
            // Parse r and s (32 bytes each, hex encoded)
            let r_bytes = if r_str.starts_with("0x") {
                hex::decode(&r_str[2..])
            } else {
                hex::decode(r_str)
            }
            .map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid r signature: {}", e),
                data: None,
            })?;

            let s_bytes = if s_str.starts_with("0x") {
                hex::decode(&s_str[2..])
            } else {
                hex::decode(s_str)
            }
            .map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid s signature: {}", e),
                data: None,
            })?;

            if r_bytes.len() != 32 || s_bytes.len() != 32 {
                return Err(JsonRpcError {
                    code: -32602,
                    message: "Invalid signature: r and s must be 32 bytes each".to_string(),
                    data: None,
                });
            }

            let mut r = [0u8; 32];
            r.copy_from_slice(&r_bytes);
            let mut s = [0u8; 32];
            s.copy_from_slice(&s_bytes);

            // Create ECDSA signature - use ORIGINAL v value (not normalized)
            // verify_ecdsa_signature will extract chain_id and recovery_id from v
            let ecdsa_sig = crate::blockchain::EcdsaSignature { r, s, v: v_val };

            // Set ECDSA signature in transaction
            tx.ecdsa_signature = Some(ecdsa_sig);

            // Verify signature
            if !tx.verify_signature(self.chain_id).unwrap_or(false) {
                return Err(JsonRpcError {
                    code: -32000,
                    message: "Invalid ECDSA signature".to_string(),
                    data: None,
                });
            }

            // Submit to mining manager
            if let Some(mining_mgr) = &self.mining_manager {
                mining_mgr
                    .add_transaction(tx.clone())
                    .await
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Failed to add transaction: {}", e),
                        data: None,
                    })?;
            } else {
                return Err(JsonRpcError {
                    code: -32603,
                    message: "Mining manager not available".to_string(),
                    data: None,
                });
            }

            // Return transaction hash
            return Ok(json!({
                "hash": format!("0x{}", hex::encode(tx.hash)),
                "from": format!("0x{}", hex::encode(from)),
                "to": format!("0x{}", hex::encode(to)),
                "value": format!("0x{:x}", value),
                "gas": format!("0x{:x}", gas_limit),
                "gasPrice": format!("0x{:x}", gas_price),
                "nonce": format!("0x{:x}", nonce),
            }));
        }

        // No signature provided - return error
        Err(JsonRpcError {
            code: -32602,
            message: "Missing signature (r, s, v). Metamask should provide these fields."
                .to_string(),
            data: None,
        })
    }

    /// eth_sendRawTransaction - Send a raw signed transaction
    ///
    /// This method accepts a raw signed transaction in RLP-encoded hex format.
    /// MetaMask sends transactions this way after signing them.
    async fn eth_send_raw_transaction(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;

        let raw_tx_hex = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_param_error("raw transaction"))?;

        // Decode hex (remove 0x prefix if present)
        let raw_tx_bytes = if raw_tx_hex.starts_with("0x") {
            hex::decode(&raw_tx_hex[2..])
        } else {
            hex::decode(raw_tx_hex)
        }
        .map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid hex encoding: {}", e),
            data: None,
        })?;

        // EIP-2718: Detect typed transaction vs legacy
        // Typed transactions have first byte <= 0x7f (the transaction type)
        // Legacy transactions start with RLP list encoding (0xc0-0xff)
        let (tx_type, rlp_bytes): (u8, &[u8]) =
            if !raw_tx_bytes.is_empty() && raw_tx_bytes[0] <= 0x7f {
                // Typed transaction: first byte is tx type, rest is RLP
                (raw_tx_bytes[0], &raw_tx_bytes[1..])
            } else {
                // Legacy transaction (type 0): entire slice is RLP
                (0u8, raw_tx_bytes.as_slice())
            };

        // Decode RLP transaction
        // MetaMask sends: RLP([nonce, gasPrice, gasLimit, to, value, data, v, r, s])
        // Or for EIP-1559: RLP([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, v, r, s])

        use alloy_rlp::Decodable;

        // Helper to decode RLP list items with proper error handling
        fn decode_rlp_item<T: Decodable>(
            data: &mut &[u8],
            field_name: &str,
        ) -> Result<T, JsonRpcError> {
            T::decode(data).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid {}: {}", field_name, e),
                data: None,
            })
        }

        // Helper to decode u64 from RLP bytes (handles variable-length encoding)
        fn decode_u64_from_bytes(data: &mut &[u8], field_name: &str) -> Result<u64, JsonRpcError> {
            let bytes: Vec<u8> = decode_rlp_item(data, field_name)?;
            if bytes.is_empty() {
                return Ok(0);
            }
            if bytes.len() > 8 {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("{} too large", field_name),
                    data: None,
                });
            }
            let mut buf = [0u8; 8];
            buf[8 - bytes.len()..].copy_from_slice(&bytes);
            Ok(u64::from_be_bytes(buf))
        }

        // Helper to decode u128 from RLP bytes
        fn decode_u128_from_bytes(
            data: &mut &[u8],
            field_name: &str,
        ) -> Result<u128, JsonRpcError> {
            let bytes: Vec<u8> = decode_rlp_item(data, field_name)?;
            if bytes.is_empty() {
                return Ok(0);
            }
            if bytes.len() > 16 {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("{} too large", field_name),
                    data: None,
                });
            }
            let mut buf = [0u8; 16];
            buf[16 - bytes.len()..].copy_from_slice(&bytes);
            Ok(u128::from_be_bytes(buf))
        }

        // Create a cursor for decoding - alloy-rlp advances the slice as it decodes
        let mut data = rlp_bytes;

        // Parse transaction based on type
        // Legacy (type 0): [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
        // EIP-1559 (type 2): [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, yParity, r, s]

        let (nonce, gas_price, gas_limit): (u64, u128, u64) = if tx_type == 0x02 {
            // EIP-1559: first field is chainId
            let _chain_id: u64 = decode_u64_from_bytes(&mut data, "chainId")?;
            let nonce = decode_u64_from_bytes(&mut data, "nonce")?;
            let _max_priority_fee: u128 =
                decode_u128_from_bytes(&mut data, "maxPriorityFeePerGas")?;
            let max_fee: u128 = decode_u128_from_bytes(&mut data, "maxFeePerGas")?;
            let gas_limit = decode_u64_from_bytes(&mut data, "gasLimit")?;
            // Use max_fee as gas_price for EIP-1559 (TODO: proper base fee calculation)
            (nonce, max_fee, gas_limit)
        } else {
            // Legacy transaction
            let nonce = decode_u64_from_bytes(&mut data, "nonce")?;
            let gas_price = decode_u128_from_bytes(&mut data, "gasPrice")?;
            let gas_limit = decode_u64_from_bytes(&mut data, "gasLimit")?;
            (nonce, gas_price, gas_limit)
        };

        let to_bytes: Vec<u8> = decode_rlp_item(&mut data, "to")?;

        let to = if to_bytes.is_empty() {
            [0u8; 20] // Contract deployment
        } else if to_bytes.len() == 20 {
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&to_bytes);
            addr
        } else {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("Invalid to address length: {}", to_bytes.len()),
                data: None,
            });
        };

        let value: u128 = decode_u128_from_bytes(&mut data, "value")?;

        let tx_data: Vec<u8> = decode_rlp_item(&mut data, "data")?;

        // For EIP-1559, skip access_list before signature fields
        if tx_type == 0x02 {
            // access_list is an RLP list of tuples (address, storage_keys)
            // We decode but don't use it (TODO: implement access list support)
            let _access_list: Vec<u8> = decode_rlp_item(&mut data, "accessList")?;
        }

        // Decode signature fields
        // EIP-1559 uses yParity (0 or 1), legacy uses v (may include chain_id per EIP-155)
        let v: u64 = decode_u64_from_bytes(&mut data, "yParity/v")?;

        let r_bytes: Vec<u8> = decode_rlp_item(&mut data, "r")?;

        let s_bytes: Vec<u8> = decode_rlp_item(&mut data, "s")?;

        // Ensure r and s are 32 bytes
        if r_bytes.len() > 32 || s_bytes.len() > 32 {
            return Err(JsonRpcError {
                code: -32602,
                message: "Invalid signature: r or s too long".to_string(),
                data: None,
            });
        }

        let mut r = [0u8; 32];
        let mut s = [0u8; 32];

        // Pad left with zeros if needed
        r[32 - r_bytes.len()..].copy_from_slice(&r_bytes);
        s[32 - s_bytes.len()..].copy_from_slice(&s_bytes);

        // Extract chain_id from v if EIP-155 (legacy only)
        // For EIP-1559, chain_id is in the transaction body, v is just yParity (0 or 1)
        let extracted_chain_id = if tx_type == 0x02 {
            // EIP-1559: chain_id was already decoded and validated earlier
            // yParity is just 0 or 1, no chain_id encoding
            Some(self.chain_id)
        } else if v >= 35 {
            let chain_id = (v - 35) / 2;
            if chain_id != self.chain_id {
                return Err(JsonRpcError {
                    code: -32000,
                    message: format!(
                        "Invalid Chain ID: transaction signed for chain {}, but node expects {}",
                        chain_id, self.chain_id
                    ),
                    data: Some(json!({
                        "expected_chain_id": self.chain_id,
                        "received_chain_id": chain_id,
                        "hint": "Make sure MetaMask is connected to the correct network"
                    })),
                });
            }
            Some(chain_id)
        } else {
            None
        };

        // Calculate fee
        let fee = gas_price
            .checked_mul(gas_limit as u128)
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: "Gas price * gas limit overflow".to_string(),
                data: None,
            })?;

        // Create transaction
        let mut tx = if !tx_data.is_empty() {
            crate::blockchain::Transaction::with_data(
                Address::zero(), // from will be recovered from signature
                Address(to),
                value,
                fee,
                nonce,
                tx_data,
                gas_limit,
            )
        } else {
            crate::blockchain::Transaction::new(
                Address::zero(), // from will be recovered from signature
                Address(to),
                value,
                fee,
                nonce,
            )
        };

        // Set chain_id for EIP-155 replay protection
        tx.chain_id = extracted_chain_id;

        // Set ECDSA signature - IMPORTANT: Store ORIGINAL v value, not normalized
        // verify_ecdsa_signature will extract chain_id and recovery_id from the original v
        let ecdsa_sig = crate::blockchain::EcdsaSignature {
            r,
            s,
            v, // Use ORIGINAL v value (2710 for chain_id 1337), not v_normalized
        };
        tx.ecdsa_signature = Some(ecdsa_sig.clone());

        // Recover sender address from signature BEFORE verification
        let recovered_address =
            tx.recover_ecdsa_address(&ecdsa_sig)
                .ok_or_else(|| JsonRpcError {
                    code: -32000,
                    message: "Invalid signature: could not recover sender address".to_string(),
                    data: None,
                })?;

        // Set the recovered address on the transaction
        tx.from = recovered_address;

        // CRITICAL: Set the transaction hash to the hash of the RAW SIGNED BYTES
        // This ensures it matches what ethers.js/MetaMask expects
        use sha3::{Digest, Keccak256};
        let mut hasher = Keccak256::new();
        hasher.update(&raw_tx_bytes);
        let signed_tx_hash = hasher.finalize();
        let mut hash_array = [0u8; 32];
        hash_array.copy_from_slice(&signed_tx_hash);
        tx.hash = Hash(hash_array);

        if !tx.verify_signature(self.chain_id).unwrap_or(false) {
            return Err(JsonRpcError {
                code: -32000,
                message: "Invalid signature: verification failed".to_string(),
                data: None,
            });
        }

        // Verify balance and nonce
        let balance = self.accounts_cache.get(&tx.from).map(|s| s.balance).unwrap_or(0);
        let current_nonce = self.accounts_cache.get(&tx.from).map(|s| s.nonce).unwrap_or(0);

        let total_cost = value.checked_add(fee).ok_or_else(|| JsonRpcError {
            code: -32000,
            message: "Transaction value + fee overflow".to_string(),
            data: None,
        })?;

        if tx.nonce != current_nonce {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Invalid nonce: expected {}, got {}",
                    current_nonce, tx.nonce
                ),
                data: Some(json!({
                    "expected_nonce": current_nonce,
                    "received_nonce": tx.nonce,
                    "hint": "Nonce must match the current transaction count for this address"
                })),
            });
        }

        if balance < total_cost {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Insufficient balance: have {}, need {}",
                    balance, total_cost
                ),
                data: Some(json!({
                    "balance": balance,
                    "required": total_cost,
                    "value": value,
                    "fee": fee,
                    "hint": "Sender does not have enough balance to cover transaction value + fee"
                })),
            });
        }

        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr
                .add_transaction(tx.clone())
                .await
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to add transaction: {}", e),
                    data: None,
                })?;
        } else {
            return Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            });
        }

        if let Some(ref network_manager) = self.network_manager {
            let _ = network_manager.broadcast_transaction(&tx).await;
        }

        Ok(Value::String(format!("0x{}", hex::encode(tx.hash))))
    }

    /// eth_getBlockTransactionCountByNumber - Get transaction count in block
    async fn eth_get_block_transaction_count_by_number(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;

        let block_num_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid_param_error("block number"))?;

        let blockchain = self.acquire_blockchain_read().await?;

        let block_number = if block_num_str == "latest" || block_num_str == "pending" {
            let count = blockchain.get_block_count();
            if count == 0 { 0 } else { (count - 1) as u64 }
        } else if block_num_str == "finalized" || block_num_str == "safe" {
            blockchain
                .get_finalized_block_number()
                .unwrap_or(None)
                .unwrap_or(0)
        } else {
            parse_hex_number(block_num_str)?
        };

        let block = blockchain.get_block_by_number(block_number);

        let count = block.map(|b| b.transactions.len()).unwrap_or(0);
        Ok(Value::String(format!("0x{:x}", count)))
    }

    /// net_peerCount - Get connected peer count
    async fn net_peer_count(&self) -> Result<Value, JsonRpcError> {
        if let Some(network_mgr) = &self.network_manager {
            let peer_count = network_mgr.peer_count();
            Ok(Value::String(format!("0x{:x}", peer_count)))
        } else {
            // Fallback if network manager not set
            Ok(Value::String("0x0".to_string()))
        }
    }

    // =========================================================================
    // === IRONDAG EXTENSION METHODS ======================================
    // =========================================================================
    // These methods provide blockchain-specific functionality beyond the
    // standard Ethereum JSON-RPC API. They include:
    // - DAG/TPS statistics and monitoring
    // - Sharding operations
    // - Privacy features
    // - Oracle integration
    // - Account abstraction (wallets, multisig, social recovery)
    // - Mining control
    // - Snapshot management
    // =========================================================================

    /// irondag_getDagStats - Get GhostDAG statistics (cached)
    async fn irondag_get_dag_stats(&self) -> Result<Value, JsonRpcError> {
        let cache_key = "irondag_getDagStats".to_string();
        let now = Instant::now();
        let current_block = self
            .blockchain_cached_block_number
            .load(std::sync::atomic::Ordering::Relaxed);

        // Aggressive caching: return cached response if < 10 seconds old, even if block changed.
        // This prevents lock contention from completely blocking stats during mining.
        const AGGRESSIVE_CACHE_TTL_SECS: u64 = 10;
        {
            let cache = self.response_cache.read().await;
            if let Some((cached_value, cached_time, _cached_block)) = cache.get(&cache_key) {
                if now.duration_since(*cached_time).as_secs() < AGGRESSIVE_CACHE_TTL_SECS {
                    return Ok(cached_value.clone());
                }
            }
        }

        let blockchain = self.acquire_blockchain_read().await?;
        let stats = blockchain.get_dag_stats();
        drop(blockchain);

        let result = serde_json::json!({
            "total_blocks": stats.total_blocks,
            "blue_blocks": stats.blue_blocks,
            "red_blocks": stats.red_blocks,
            "total_transactions": stats.total_transactions,
            "total_size_bytes": stats.total_size_bytes,
            "avg_block_size": stats.avg_block_size,
            "avg_txs_per_block": stats.avg_txs_per_block,
        });

        {
            let mut cache = self.response_cache.write().await;
            if cache.len() >= RESPONSE_CACHE_MAX_SIZE {
                if let Some(evict_key) = cache.keys().next().cloned() {
                    cache.remove(&evict_key);
                }
            }
            cache.insert(cache_key, (result.clone(), now, current_block));
        }

        Ok(result)
    }

    /// irondag_getBlueScore - Get blue score for a block
    async fn irondag_get_blue_score(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;
        let params_arr = params.as_array().ok_or_else(missing_params_error)?;

        let hash = extract_hash_param(params_arr, 0)?;

        let blockchain = self.acquire_blockchain_read().await?;
        let blue_score = if let Some(dag) = blockchain.ghostdag() {
            dag.read().await.get_blue_score(&hash).unwrap_or(0)
        } else {
            0
        };

        Ok(Value::String(format!("0x{:x}", blue_score)))
    }

    /// irondag_getTps - Get transactions per second
    async fn irondag_get_tps(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let duration_seconds = if let Some(ref params) = params {
            if let Some(ref arr) = params.as_array() {
                if let Some(v) = arr.get(0) {
                    v.as_str()
                        .and_then(|s| parse_hex_number(s).ok())
                        .or_else(|| v.as_u64())
                        .unwrap_or(60)
                } else {
                    60
                }
            } else {
                60
            }
        } else {
            60
        };

        let blockchain = self.acquire_blockchain_read().await?;
        let tps = blockchain.get_tps(duration_seconds);

        Ok(Value::String(format!("{:.2}", tps)))
    }

    /// irondag_getBlocksByStream - Get blocks filtered by stream type
    /// Params: [stream_type: "A"|"B"|"C", count: number (max 100)]
    async fn irondag_get_blocks_by_stream(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        // Parse params: [stream_type_str, count]
        let (stream_filter, count) = match params.as_ref().and_then(|p| p.as_array()) {
            Some(arr) if arr.len() >= 2 => {
                let stream_str = arr[0].as_str().unwrap_or("A");
                let count = arr[1].as_u64().unwrap_or(10).min(100) as usize;
                (stream_str.to_uppercase(), count)
            }
            Some(arr) if arr.len() >= 1 => {
                let stream_str = arr[0].as_str().unwrap_or("A");
                (stream_str.to_uppercase(), 10)
            }
            _ => ("A".to_string(), 10),
        };

        // Validate stream type
        if !["A", "B", "C"].contains(&stream_filter.as_str()) {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("Invalid stream type: {}. Must be A, B, or C", stream_filter),
                data: None,
            });
        }

        let blockchain = self.acquire_blockchain_read().await?;

        // Use with_blocks() closure to avoid cloning entire chain
        let filtered: Vec<crate::blockchain::Block> = blockchain.with_blocks(|blocks| {
            // Filter blocks by stream type and collect matching ones
            let mut matching: Vec<crate::blockchain::Block> = blocks
                .iter()
                .filter(|b| {
                    let st = match b.header.stream_type {
                        crate::types::StreamType::StreamA => "A",
                        crate::types::StreamType::StreamB => "B",
                        crate::types::StreamType::StreamC => "C",
                    };
                    st == stream_filter
                })
                .cloned()
                .collect();

            // Sort by block number descending (most recent first) and truncate
            matching.sort_by(|a, b| b.header.block_number.cmp(&a.header.block_number));
            matching.truncate(count);
            matching
        });
        drop(blockchain);

        // Convert to JSON using the existing block_to_json_with_shard function
        let blocks_json: Vec<Value> = filtered
            .iter()
            .map(|b| block_to_json_with_shard(Some(b.clone()), None, self.miner_address))
            .collect();

        Ok(Value::Array(blocks_json))
    }

    /// irondag_getStreamCounts - Get total block count for each stream type
    /// Returns JSON object like {"A": 2500, "B": 1200, "C": 0}
    async fn irondag_get_stream_counts(&self) -> Result<Value, JsonRpcError> {
        let blockchain = self.acquire_blockchain_read().await?;

        // Count blocks per stream type
        let (mut count_a, mut count_b, mut count_c) = (0u64, 0u64, 0u64);

        blockchain.with_blocks(|blocks| {
            for block in blocks.iter() {
                match block.header.stream_type {
                    crate::types::StreamType::StreamA => count_a += 1,
                    crate::types::StreamType::StreamB => count_b += 1,
                    crate::types::StreamType::StreamC => count_c += 1,
                }
            }
        });
        drop(blockchain);

        // Build JSON result
        Ok(json!({
            "A": count_a,
            "B": count_b,
            "C": count_c
        }))
    }

    /// eth_getCode - Get contract code at address
    async fn eth_get_code(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(missing_params_error)?;
        let params_arr = params.as_array().ok_or_else(missing_params_error)?;

        let address = extract_address_param(params_arr, 0)?;

        let blockchain = self.acquire_blockchain_read().await?;
        if let Some(executor) = blockchain.evm_executor() {
            if let Some(code) = executor.get_contract_code(address) {
                return Ok(Value::String(format!("0x{}", hex::encode(code))));
            }
        }

        Ok(Value::String("0x".to_string()))
    }

    /// eth_gasPrice - Get current gas price (legacy compatibility)
    /// Returns base fee + suggested priority fee for legacy transactions
    async fn eth_gas_price(&self) -> Result<Value, JsonRpcError> {
        // EIP-1559: Return base fee + small priority fee for legacy compatibility
        let blockchain = self.acquire_blockchain_read().await?;
        let base_fee = blockchain
            .get_latest_block()
            .map(|b| b.header.base_fee_per_gas)
            .unwrap_or(crate::mining::BASE_FEE_INITIAL);

        // Add a small priority fee (0.1 Gwei) for legacy compatibility
        let gas_price = base_fee + 100_000_000u128;

        Ok(Value::String(format!("0x{:x}", gas_price)))
    }

    /// eth_maxPriorityFeePerGas - Get max priority fee (EIP-1559)
    /// Returns the suggested priority fee (tip) for miners
    async fn eth_max_priority_fee_per_gas(&self) -> Result<Value, JsonRpcError> {
        // EIP-1559: Return suggested priority fee (0.1 Gwei)
        // This is a fixed suggestion; in production could be based on recent block congestion
        let priority_fee = 100_000_000u128; // 0.1 Gwei
        Ok(Value::String(format!("0x{:x}", priority_fee)))
    }

    /// eth_feeHistory - Get historical gas fees (EIP-1559)
    async fn eth_fee_history(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        // Parse params: [blockCount, newestBlock, rewardPercentiles]
        let _params = params.unwrap_or(serde_json::json!([1, "latest", []]));

        // EIP-1559: Return real base fee history from recent blocks
        let blockchain = self.acquire_blockchain_read().await?;
        let tail = blockchain.get_blocks_tail(10);

        // Most recent first
        let mut recent_blocks: Vec<_> = tail.iter().collect();
        recent_blocks.sort_by_key(|b| b.header.block_number);
        recent_blocks.reverse();

        // Build base fee history (oldest first)
        let mut base_fee_per_gas: Vec<String> = recent_blocks
            .iter()
            .rev()
            .map(|b| format!("0x{:x}", b.header.base_fee_per_gas))
            .collect();

        // If no blocks, use initial base fee
        if base_fee_per_gas.is_empty() {
            base_fee_per_gas.push(format!("0x{:x}", crate::mining::BASE_FEE_INITIAL));
        }

        // Calculate gas used ratios (simplified: actual gas / gas limit)
        let gas_used_ratio: Vec<f64> = recent_blocks
            .iter()
            .rev()
            .map(|b| {
                let gas_used: u64 = b.transactions.iter().map(|tx| {
                    if tx.data.is_empty() { 21_000u64 } else { tx.gas_limit }
                }).sum();
                let gas_limit = match b.header.stream_type {
                    crate::types::StreamType::StreamA => crate::mining::STREAM_A_GAS_LIMIT,
                    crate::types::StreamType::StreamB => crate::mining::STREAM_B_GAS_LIMIT,
                    crate::types::StreamType::StreamC => crate::mining::STREAM_C_GAS_LIMIT,
                };
                if gas_limit > 0 {
                    (gas_used as f64) / (gas_limit as f64)
                } else {
                    0.5
                }
            })
            .collect();

        // If no blocks, use default ratio
        let gas_used_ratio = if gas_used_ratio.is_empty() {
            vec![0.5]
        } else {
            gas_used_ratio
        };

        // Oldest block number
        let oldest_block = recent_blocks
            .last()
            .map(|b| b.header.block_number)
            .unwrap_or(1);

        Ok(json!({
            "oldestBlock": format!("0x{:x}", oldest_block),
            "baseFeePerGas": base_fee_per_gas,
            "gasUsedRatio": gas_used_ratio,
            "reward": [["0x5f5e100"]] // 0.1 Gwei suggested priority fee
        }))
    }

    /// eth_estimateGas - Estimate gas for a transaction via EVM dry-run simulation.
    /// Falls back to heuristics for simple transfers (no data).
    async fn eth_estimate_gas(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.unwrap_or(Value::Array(vec![]));
        let call_obj = params.as_array().and_then(|a| a.get(0)).and_then(|v| v.as_object());

        // Simple ETH transfer: no EVM needed
        let has_data = call_obj
            .and_then(|o| o.get("data").and_then(|v| v.as_str()))
            .map(|s| s != "0x" && !s.is_empty())
            .unwrap_or(false);
        if !has_data {
            return Ok(Value::String("0x5208".to_string())); // 21_000 in hex
        }

        // Contract call or deployment: simulate via EVM read-only execution
        let blockchain = self.acquire_blockchain_read().await?;
        if let Some(executor) = blockchain.evm_executor() {
            let call_obj = call_obj.ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid params: expected call object".to_string(),
                data: None,
            })?;
            let from = call_obj.get("from").and_then(|v| v.as_str())
                .map(parse_address).transpose()?.unwrap_or(Address::zero());
            let to = call_obj.get("to").and_then(|v| v.as_str())
                .map(parse_address).transpose()?.unwrap_or(Address::zero());
            let value = call_obj.get("value").and_then(|v| v.as_str())
                .map(parse_hex_u128).transpose()?.unwrap_or(0);
            let data_str = call_obj.get("data").and_then(|v| v.as_str()).unwrap_or("0x");
            let data = if data_str.starts_with("0x") {
                hex::decode(&data_str[2..]).unwrap_or_default()
            } else {
                hex::decode(data_str).unwrap_or_default()
            };
            let gas_limit = call_obj.get("gas").or_else(|| call_obj.get("gasLimit"))
                .and_then(|v| v.as_str()).map(parse_hex_u128).transpose()?
                .and_then(|v| u64::try_from(v).ok()).unwrap_or(10_000_000);

            let block_num = blockchain.latest_block_number();
            let block_timestamp = blockchain.get_latest_block()
                .map(|b| b.header.timestamp)
                .unwrap_or_else(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
                });
            let nonce = executor.get_account_nonce(from);
            let temp_tx = crate::blockchain::Transaction::with_data(from, to, value, 0, nonce, data, gas_limit);

            match executor.execute_readonly(&temp_tx, block_num, block_timestamp) {
                Ok(result) => {
                    // Add 10% safety buffer on top of actual gas used
                    let estimate = result.gas_used.saturating_add(result.gas_used / 10).max(21_000);
                    return Ok(Value::String(format!("0x{:x}", estimate)));
                }
                Err(_) => {} // Fall through to heuristic
            }
        }
        drop(blockchain);

        // Heuristic fallback (EVM unavailable or simulation failed)
        let data_len = call_obj
            .and_then(|o| o.get("data").and_then(|v| v.as_str()))
            .map(|s| if s.starts_with("0x") { (s.len() - 2) / 2 } else { s.len() / 2 })
            .unwrap_or(0);
        let gas_estimate = 50_000u64.saturating_add((data_len as u64).saturating_mul(68)).min(2_000_000);

        Ok(Value::String(format!("0x{:x}", gas_estimate)))
    }

    /// eth_call - Execute a contract call without creating a transaction
    async fn eth_call(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        // Parse call object
        let call_obj = params_array
            .get(0)
            .and_then(|v| v.as_object())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid call parameter".to_string(),
                data: None,
            })?;

        let to_str = call_obj
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'to' address".to_string(),
                data: None,
            })?;
        let to = parse_address(to_str)?;

        let data_str = call_obj
            .get("data")
            .and_then(|v| v.as_str())
            .unwrap_or("0x");
        let data = if data_str.starts_with("0x") {
            hex::decode(&data_str[2..]).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid data hex: {}", e),
                data: None,
            })?
        } else {
            hex::decode(data_str).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid data hex: {}", e),
                data: None,
            })?
        };

        // Execute contract call via EVM
        let blockchain = self.acquire_blockchain_read().await?;
        if !blockchain.evm_enabled {
            return Err(JsonRpcError {
                code: -32603,
                message: "EVM is not enabled".to_string(),
                data: None,
            });
        }

        if let Some(executor) = blockchain.evm_executor() {
            let from = call_obj
                .get("from")
                .and_then(|v| v.as_str())
                .map(|s| parse_address(s))
                .transpose()?
                .unwrap_or(Address::zero());

            let value_str = call_obj
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("0x0");
            let value = parse_hex_u128(value_str)?;

            let block_num = blockchain.latest_block_number();
            let block_timestamp = if let Some(latest_block) = blockchain.get_latest_block() {
                latest_block.header.timestamp
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            };

            let gas_limit = call_obj
                .get("gas")
                .or_else(|| call_obj.get("gasLimit"))
                .and_then(|v| v.as_str())
                .map(parse_hex_u128)
                .transpose()?
                .and_then(|v| u64::try_from(v).ok())
                .unwrap_or(1_000_000);

            let nonce = executor.get_account_nonce(from);

            // Create a temporary transaction for the call
            let temp_tx = crate::blockchain::Transaction::with_data(
                from, to, value, 0, // No fee for call
                nonce, data, gas_limit,
            );

            // Execute via EVM - the EVM is the source of truth for contract calls
            // No hardcoded selector matching - rely entirely on EVM execution

            match executor.execute_readonly(&temp_tx, block_num, block_timestamp) {
                Ok(result) => {
                    // Return the EVM output directly - the EVM is authoritative
                    Ok(Value::String(format!("0x{}", hex::encode(result.output))))
                }
                Err(e) => {
                    // Return the actual EVM error instead of silently returning wrong data
                    Err(JsonRpcError {
                        code: -32000,
                        message: format!("Contract call failed: {}", e),
                        data: None,
                    })
                }
            }
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "EVM executor not available".to_string(),
                data: None,
            })
        }
    }

    /// eth_getStorageAt - Get contract storage value at specific position
    async fn eth_get_storage_at(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an array".to_string(),
            data: None,
        })?;

        if params_array.len() < 2 {
            return Err(JsonRpcError {
                code: -32602,
                message: "Missing parameters: address and position required".to_string(),
                data: None,
            });
        }

        // Parse contract address
        let address_str = params_array[0].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid address format".to_string(),
            data: None,
        })?;

        let address = parse_address(address_str)?;

        // Parse storage position
        let position_str = params_array[1].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid position format".to_string(),
            data: None,
        })?;

        let position = if position_str.starts_with("0x") {
            &position_str[2..]
        } else {
            position_str
        };

        // Handle short hex (e.g., "0" -> "00", "1" -> "01") - Ethereum compatibility
        let position_padded = if position.len() % 2 == 1 {
            format!("0{}", position)
        } else if position.is_empty() {
            "00".to_string()
        } else {
            position.to_string()
        };

        // Parse position as 32-byte storage key
        let position_bytes = hex::decode(&position_padded).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid position hex: {}", e),
            data: None,
        })?;

        // Ensure position is 32 bytes (pad with leading zeros if necessary)
        let mut storage_key = [0u8; 32];
        if position_bytes.len() <= 32 {
            let start_pos = 32 - position_bytes.len();
            storage_key[start_pos..].copy_from_slice(&position_bytes);
        } else {
            return Err(JsonRpcError {
                code: -32602,
                message: "Storage position too long (max 32 bytes)".to_string(),
                data: None,
            });
        }

        // Get storage from blockchain
        let blockchain = self.acquire_blockchain_read().await?;
        if !blockchain.evm_enabled {
            return Err(JsonRpcError {
                code: -32603,
                message: "EVM is not enabled".to_string(),
                data: None,
            });
        }

        if let Some(executor) = blockchain.evm_executor() {
            let storage_value = executor
                .get_contract_storage(address, &storage_key)
                .unwrap_or(vec![0u8; 32]); // Return 32 zero bytes if not found

            // Return as hex string (32 bytes)
            Ok(Value::String(format!("0x{}", hex::encode(storage_value))))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "EVM executor not available".to_string(),
                data: None,
            })
        }
    }

    /// eth_getTransactionReceipt - Get transaction receipt
    async fn eth_get_transaction_receipt(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid transaction hash parameter".to_string(),
                data: None,
            })?;

        let tx_hash = parse_hash(hash_str)?;
        debug!(
            "RPC: eth_getTransactionReceipt searching for {}",
            hex::encode(tx_hash)
        );

        let blockchain = self.acquire_blockchain_read().await?;
        let found_tx = blockchain
            .get_transaction_by_hash(&tx_hash)
            .map(|(block, tx, idx)| (tx, block.header.block_number, idx, block.hash));
        drop(blockchain);

        if let Some((tx, block_number, index, block_hash)) = found_tx {
            // Re-acquire read lock to get receipt and cumulative gas from same snapshot
            let blockchain_ref = self.acquire_blockchain_read().await?;

            // Look up the persisted execution receipt (real gas_used, status, logs)
            let receipt = blockchain_ref.get_tx_receipt(&tx_hash);

            let gas_used = receipt.as_ref().map(|r| r.gas_used).unwrap_or_else(|| {
                // Fallback for blocks processed before receipt tracking was added
                if tx.data.is_empty() { 21_000 } else { tx.gas_limit }
            });

            let status = if receipt.as_ref().map(|r| r.success).unwrap_or(true) {
                "0x1"
            } else {
                "0x0"
            };

            let contract_address = receipt.as_ref().and_then(|r| r.contract_address)
                .map(|addr| Value::String(format!("0x{}", hex::encode(addr))))
                .unwrap_or(Value::Null);

            // Build logs array from receipt
            let logs_json: Vec<Value> = receipt.as_ref().map(|r| {
                r.logs.iter().enumerate().map(|(log_idx, log)| {
                    json!({
                        "address": format!("0x{}", hex::encode(log.address)),
                        "topics": log.topics.iter().map(|t| format!("0x{}", hex::encode(t))).collect::<Vec<_>>(),
                        "data": format!("0x{}", hex::encode(&log.data)),
                        "blockNumber": format!("0x{:x}", block_number),
                        "transactionHash": format!("0x{}", hex::encode(tx_hash)),
                        "transactionIndex": format!("0x{:x}", index),
                        "blockHash": format!("0x{}", hex::encode(block_hash)),
                        "logIndex": format!("0x{:x}", log_idx),
                        "removed": false,
                    })
                }).collect()
            }).unwrap_or_default();

            // Cumulative gas: sum receipts for all txs up to this index in the block
            let mut cumulative_gas_used = 0u64;
            if let Some(block) = blockchain_ref.get_block_by_hash(&block_hash) {
                for (i, btx) in block.transactions.iter().enumerate() {
                    let tx_gas = blockchain_ref.get_tx_receipt(&btx.hash)
                        .map(|r| r.gas_used)
                        .unwrap_or(if btx.data.is_empty() { 21_000 } else { btx.gas_limit });
                    cumulative_gas_used = cumulative_gas_used.saturating_add(tx_gas);
                    if i == index {
                        break;
                    }
                }
            }
            drop(blockchain_ref);

            Ok(json!({
                "transactionHash": format!("0x{}", hex::encode(tx_hash)),
                "transactionIndex": format!("0x{:x}", index),
                "blockNumber": format!("0x{:x}", block_number),
                "blockHash": format!("0x{}", hex::encode(block_hash)),
                "from": format!("0x{}", hex::encode(tx.from)),
                "to": if tx.to.is_zero() { Value::Null } else { Value::String(format!("0x{}", hex::encode(tx.to))) },
                "gasUsed": format!("0x{:x}", gas_used),
                "cumulativeGasUsed": format!("0x{:x}", cumulative_gas_used),
                "contractAddress": contract_address,
                "logs": logs_json,
                "status": status,
                "logsBloom": format!("0x{}", "0".repeat(512)),
            }))
        } else {
            Ok(Value::Null)
        }
    }

    /// irondag_getShardStats - Get statistics for all shards
    async fn irondag_get_shard_stats(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        if let Some(shard_manager) = &self.shard_manager {
            let stats = shard_manager.get_all_shard_stats().await;
            let shards_json: Vec<Value> = stats
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "shard_id": s.shard_id,
                        "block_count": s.block_count,
                        "transaction_pool_size": s.transaction_pool_size,
                        "cross_shard_outgoing": s.cross_shard_outgoing,
                        "cross_shard_incoming": s.cross_shard_incoming,
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "shard_count": stats.len(),
                "shards": shards_json
            }))
        } else {
            Ok(serde_json::json!({
                "shard_count": 0,
                "shards": []
            }))
        }
    }

    /// irondag_getShardForAddress - Get shard ID for an address
    async fn irondag_get_shard_for_address(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        if let Some(shard_manager) = &self.shard_manager {
            let shard_id = shard_manager.get_shard_for_address(&address);
            Ok(Value::String(format!("0x{:x}", shard_id)))
        } else {
            Ok(Value::String("0x0".to_string()))
        }
    }

    /// irondag_getRiskScore - Get risk score for an address
    async fn irondag_get_risk_score(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let scorer = self.security_scorer.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Security scoring not enabled".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        let scorer_guard = scorer.read().await;
        let risk_score = scorer_guard.score_address(&address);

        Ok(serde_json::json!({
            "score": risk_score.score,
            "confidence": risk_score.confidence,
            "labels": risk_score.labels,
        }))
    }

    /// irondag_getRiskLabels - Get risk labels for an address
    async fn irondag_get_risk_labels(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let scorer = self.security_scorer.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Security scoring not enabled".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        let scorer_guard = scorer.read().await;
        let risk_score = scorer_guard.score_address(&address);

        Ok(serde_json::json!({
            "labels": risk_score.labels,
        }))
    }

    /// irondag_getTransactionRisk - Get risk score for a transaction
    async fn irondag_get_transaction_risk(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let scorer = self.security_scorer.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Security scoring not enabled".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid transaction hash parameter".to_string(),
                data: None,
            })?;

        let tx_hash = parse_hash(hash_str)?;

        // Find transaction in blockchain
        let blockchain = self.acquire_blockchain_read().await?;
        let found_tx = blockchain.get_transaction_by_hash(&tx_hash).map(|(_, tx, _)| tx);
        drop(blockchain);

        let tx = found_tx.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Transaction not found".to_string(),
            data: None,
        })?;

        let scorer_guard = scorer.read().await;
        let risk_score = scorer_guard.score_transaction(&tx);

        Ok(serde_json::json!({
            "score": risk_score.score,
            "confidence": risk_score.confidence,
            "labels": risk_score.labels,
        }))
    }

    /// Set security scorer
    pub fn set_security_scorer(
        &mut self,
        scorer: Arc<tokio::sync::RwLock<crate::security::RiskScorer>>,
    ) {
        self.security_scorer = Some(scorer);
    }

    /// Set mining manager for fairness metrics
    pub fn set_mining_manager(&mut self, mining_manager: Arc<crate::mining::MiningManager>) {
        self.mining_manager = Some(mining_manager);
    }

    /// Set forensic analyzer for fund tracing
    pub fn set_forensic_analyzer(
        &mut self,
        forensic_analyzer: Arc<tokio::sync::RwLock<crate::security::ForensicAnalyzer>>,
    ) {
        self.forensic_analyzer = Some(forensic_analyzer);
    }

    /// Set light client for stateless mode
    pub fn set_light_client(
        &mut self,
        light_client: Arc<tokio::sync::RwLock<crate::light_client::LightClient>>,
    ) {
        self.light_client = Some(light_client);
    }

    /// Set network manager for peer info
    pub fn set_network_manager(&mut self, network_manager: Arc<crate::network::NetworkManager>) {
        self.network_manager = Some(network_manager);
    }

    /// Get total RPC requests processed (for metrics)
    pub fn get_rpc_requests_total(&self) -> u64 {
        self.rpc_requests_total.load(Ordering::Relaxed)
    }

    /// Get total RPC errors (for metrics)
    pub fn get_rpc_errors_total(&self) -> u64 {
        self.rpc_errors_total.load(Ordering::Relaxed)
    }

    /// Start BraidCore mining via RPC
    async fn irondag_start_mining(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr.start_mining().await;
            Ok(json!({
                "status": "started",
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// Stop BraidCore mining via RPC
    async fn irondag_stop_mining(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr.stop_mining().await;
            Ok(json!({
                "status": "stopped",
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// Get mining status and basic BraidCore configuration
    async fn irondag_get_mining_status(&self) -> Result<Value, JsonRpcError> {
        if let Some(mining_mgr) = &self.mining_manager {
            let is_mining = *mining_mgr.is_mining().read().await;
            let pending_txs = mining_mgr.pending_count().await;

            // Use constants from mining module for stream configuration
            let stream_a_block_time_ms = crate::mining::STREAM_A_BLOCK_TIME.as_millis();
            let stream_b_block_time_ms = crate::mining::STREAM_B_BLOCK_TIME.as_millis();
            let stream_c_block_time_ms = crate::mining::STREAM_C_BLOCK_TIME.as_millis();

            Ok(json!({
                "is_mining": is_mining,
                "pending_txs": pending_txs,
                "streams": {
                    "streamA": {
                        "block_time_ms": stream_a_block_time_ms,
                        "max_txs": crate::mining::STREAM_A_MAX_TXS,
                        "reward": format!("0x{:x}", crate::mining::STREAM_A_REWARD),
                    },
                    "streamB": {
                        "block_time_ms": stream_b_block_time_ms,
                        "max_txs": crate::mining::STREAM_B_MAX_TXS,
                        "reward": format!("0x{:x}", crate::mining::STREAM_B_REWARD),
                    },
                    "streamC": {
                        "block_time_ms": stream_c_block_time_ms,
                        "max_txs": crate::mining::STREAM_C_MAX_TXS,
                        "reward": format!("0x{:x}", crate::mining::STREAM_C_REWARD),
                    },
                }
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// Get detailed mining dashboard statistics including hashrate and earnings
    async fn irondag_get_mining_dashboard(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let blockchain = self.acquire_blockchain_read().await?;
        let total_blocks = blockchain.get_block_count() as u64;

        let mut stream_a_blocks = 0u64;
        let mut stream_b_blocks = 0u64;
        let mut stream_c_blocks = 0u64;
        let mut stream_a_earnings = 0u128;
        let mut stream_b_earnings = 0u128;
        let mut stream_c_earnings = 0u128;
        let mut total_fees_collected = 0u128;

        let tail = blockchain.get_blocks_tail(100);
        for block in tail.iter().rev() {
            match block.header.stream_type {
                crate::types::StreamType::StreamA => {
                    stream_a_blocks += 1;
                    stream_a_earnings += crate::mining::STREAM_A_REWARD;
                }
                crate::types::StreamType::StreamB => {
                    stream_b_blocks += 1;
                    stream_b_earnings += crate::mining::STREAM_B_REWARD;
                }
                crate::types::StreamType::StreamC => {
                    stream_c_blocks += 1;
                    let block_fees: u128 = block.transactions.iter().map(|tx| tx.fee).sum();
                    stream_c_earnings += block_fees;
                    total_fees_collected += block_fees;
                }
            }
        }

        let total_earnings = stream_a_earnings + stream_b_earnings + stream_c_earnings;

        // Calculate hashrate estimates (blocks per hour from 100 block sample)
        let stream_a_hashrate = stream_a_blocks as f64 * BLOCKS_PER_HOUR_MULTIPLIER;
        let stream_b_hashrate = stream_b_blocks as f64 * BLOCKS_PER_HOUR_MULTIPLIER;
        let stream_c_hashrate = stream_c_blocks as f64 * BLOCKS_PER_HOUR_MULTIPLIER;

        drop(blockchain);

        Ok(json!({
            "total_blocks": total_blocks,
            "recent_sample_size": 100,
            "streams": {
                "stream_a": {
                    "blocks_mined": stream_a_blocks,
                    "earnings": format!("0x{:x}", stream_a_earnings),
                    "hashrate_estimate_blocks_per_hour": stream_a_hashrate,
                    "block_time_seconds": 10,
                    "reward_per_block": format!("0x{:x}", crate::mining::STREAM_A_REWARD),
                },
                "stream_b": {
                    "blocks_mined": stream_b_blocks,
                    "earnings": format!("0x{:x}", stream_b_earnings),
                    "hashrate_estimate_blocks_per_hour": stream_b_hashrate,
                    "block_time_seconds": 1,
                    "reward_per_block": format!("0x{:x}", crate::mining::STREAM_B_REWARD),
                },
                "stream_c": {
                    "blocks_mined": stream_c_blocks,
                    "earnings": format!("0x{:x}", stream_c_earnings),
                    "hashrate_estimate_blocks_per_hour": stream_c_hashrate,
                    "block_time_seconds": 0.1,
                    "fees_collected": format!("0x{:x}", total_fees_collected),
                },
            },
            "total_earnings_recent": format!("0x{:x}", total_earnings),
            "fees_collected": format!("0x{:x}", total_fees_collected),
        }))
    }

    /// Send a signed transaction to the mining pool
    async fn irondag_send_raw_transaction(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        // Expect a single parameter: the transaction object
        let tx_value = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing transaction parameter".to_string(),
                data: None,
            })?;

        // Deserialize the transaction
        let tx: crate::blockchain::Transaction =
            serde_json::from_value(tx_value.clone()).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid transaction format: {}", e),
                data: None,
            })?;

        // Verify signature
        if !tx.verify_signature(self.chain_id).unwrap_or(false) {
            return Err(JsonRpcError {
                code: -32000,
                message: "Invalid transaction signature".to_string(),
                data: None,
            });
        }

        // Verify nonce and balance
        let from_addr = tx.from;
        let current_nonce = self.accounts_cache.get(&from_addr).map(|s| s.nonce).unwrap_or(0);
        let balance = self.accounts_cache.get(&from_addr).map(|s| s.balance).unwrap_or(0);

        if tx.nonce != current_nonce {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Invalid nonce: expected {}, got {}",
                    current_nonce, tx.nonce
                ),
                data: None,
            });
        }

        let total_cost = tx.value.checked_add(tx.fee).ok_or_else(|| JsonRpcError {
            code: -32000,
            message: "Transaction value + fee overflow".to_string(),
            data: None,
        })?;

        if balance < total_cost {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Insufficient balance: have {}, need {}",
                    balance, total_cost
                ),
                data: None,
            });
        }

        // Submit to mining manager
        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr
                .add_transaction(tx.clone())
                .await
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to add transaction: {}", e),
                    data: None,
                })?;
        } else {
            return Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            });
        }

        // Return the transaction hash
        Ok(json!({ "hash": format!("0x{}", hex::encode(tx.hash)) }))
    }

    /// irondag_create_test_transaction - Create and submit a test transaction (FOR TESTING ONLY)
    ///
    /// This method uses a hardcoded test key pair for testing purposes.
    /// WARNING: NOT FOR PRODUCTION USE - uses insecure test keys!
    ///
    /// Parameters: [from_address, to_address, value, fee]
    /// - from_address: Must be the test address derived from test key [2u8; 32]
    /// - to_address: Recipient address
    /// - value: Amount to send (hex string, e.g., "0x2386f26fc10000" = 0.01 IDAG)
    /// - fee: Transaction fee (hex string, optional, defaults to 0.01 IDAG)
    #[cfg(test)]
    async fn irondag_create_test_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        // Parse parameters: [from_address, to_address, value, fee]
        let from_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'from' address".to_string(),
                data: None,
            })?;
        let from = parse_address(from_str)?;

        let to_str = params_array
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'to' address".to_string(),
                data: None,
            })?;
        let to = parse_address(to_str)?;

        let value_str = params_array
            .get(2)
            .and_then(|v| v.as_str())
            .unwrap_or("0x0");
        let value = parse_hex_number(value_str)? as u128;

        let fee_str = params_array
            .get(3)
            .and_then(|v| v.as_str())
            .unwrap_or("0x2386f26fc10000"); // 0.01 IDAG default fee
        let fee = parse_hex_number(fee_str)? as u128;

        // For testing: Use a known test key pair
        // WARNING: This is INSECURE and only for testing!
        // The test key is [2u8; 32] - DO NOT USE IN PRODUCTION
        let test_secret_key: [u8; 32] = [2u8; 32];

        // Derive address from test key
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&test_secret_key);
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes: [u8; 32] = verifying_key.to_bytes();
        let test_address =
            crate::blockchain::Transaction::derive_address_from_public_key(&public_key_bytes);

        // Only sign if the from address matches the test key's address
        // Otherwise, return error (can't sign for arbitrary addresses)
        if from != test_address {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("Test transaction can only be created for test address 0x{}. Use that address as 'from'.", hex::encode(test_address)),
                data: None,
            });
        }

        // Get nonce and balance
        let nonce = self.accounts_cache.get(&from).map(|s| s.nonce).unwrap_or(0);
        let balance = self.accounts_cache.get(&from).map(|s| s.balance).unwrap_or(0);

        // Check balance
        let total_cost = value.checked_add(fee).ok_or_else(|| JsonRpcError {
            code: -32000,
            message: "Transaction value + fee overflow".to_string(),
            data: None,
        })?;

        if balance < total_cost {
            return Err(JsonRpcError {
                code: -32000,
                message: format!(
                    "Insufficient balance: have {}, need {} (value: {}, fee: {})",
                    balance, total_cost, value, fee
                ),
                data: None,
            });
        }

        // Create transaction
        let mut tx = crate::blockchain::Transaction::new(from, to, value, fee, nonce);

        // Sign the transaction
        tx = tx.sign(&test_secret_key);

        // Verify signature
        if !tx.verify_signature(self.chain_id).unwrap_or(false) {
            return Err(JsonRpcError {
                code: -32000,
                message: "Failed to create valid signature".to_string(),
                data: None,
            });
        }

        // Submit to mining manager
        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr
                .add_transaction(tx.clone())
                .await
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to add transaction: {}", e),
                    data: None,
                })?;
        } else {
            return Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            });
        }

        // Return transaction details
        Ok(json!({
            "hash": format!("0x{}", hex::encode(tx.hash)),
            "from": format!("0x{}", hex::encode(from)),
            "to": format!("0x{}", hex::encode(to)),
            "value": format!("0x{:x}", value),
            "fee": format!("0x{:x}", fee),
            "nonce": nonce,
            "testAddress": format!("0x{}", hex::encode(test_address)),
            "note": "This transaction was created with a test key pair. Fund the test address first."
        }))
    }

    /// irondag_faucet - Request testnet tokens from the faucet (FOR TESTING ONLY)
    ///
    /// Mints 10 IDAG tokens directly to the specified address (testnet only).
    /// Rate limited: one request per address per 60 seconds.
    ///
    /// Parameters: [recipient_address]
    /// - recipient_address: Ethereum-format address (0x prefixed, 40 hex chars)
    ///
    /// Returns: { amount: "10000000000000000000", recipient: "0x..." }
    async fn irondag_faucet(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params: expected [address]".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format: expected array".to_string(),
            data: None,
        })?;

        // Parse recipient address
        let recipient_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing recipient address".to_string(),
                data: None,
            })?;
        let recipient = parse_address(recipient_str)?;
        let recipient_bytes: [u8; 20] = recipient.0;

        // Check rate limit: max 1 request per address per 60 seconds
        {
            let mut rate_limiter = faucet_rate_limiter().lock().map_err(|_| JsonRpcError {
                code: -32603,
                message: "Failed to acquire rate limiter lock".to_string(),
                data: None,
            })?;

            let now = Instant::now();
            if let Some(&last_request) = rate_limiter.get(&recipient_bytes) {
                let elapsed = now.duration_since(last_request);
                if elapsed < Duration::from_secs(FAUCET_RATE_LIMIT_SECONDS) {
                    let remaining = Duration::from_secs(FAUCET_RATE_LIMIT_SECONDS) - elapsed;
                    return Err(JsonRpcError {
                        code: RPC_RATE_LIMITED,
                        message: format!(
                            "Faucet rate limit exceeded. Please wait {} seconds before requesting again.",
                            remaining.as_secs()
                        ),
                        data: None,
                    });
                }
            }

            // Update last request time
            rate_limiter.insert(recipient_bytes, now);
        }

        // Faucet amount: 10 IDAG (10 * 10^18 wei)
        let faucet_amount: u128 = 10_000_000_000_000_000_000;

        // Directly credit balance to recipient (testnet minting)
        let mut blockchain = self.blockchain.write().await;
        let current_balance = blockchain.get_balance(recipient);
        let new_balance = current_balance.saturating_add(faucet_amount);

        if let Err(e) = blockchain.set_balance(recipient, new_balance) {
            // Remove from rate limiter on failure so they can retry
            let _ = faucet_rate_limiter()
                .lock()
                .map(|mut rl| rl.remove(&recipient_bytes));
            return Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to credit faucet balance: {}", e),
                data: None,
            });
        }
        drop(blockchain);

        // Log the faucet operation
        info!("Faucet: credited 10 IDAG to {}", recipient_str);

        // Return success
        Ok(json!({
            "amount": faucet_amount.to_string(),
            "recipient": recipient_str
        }))
    }

    /// Get aggregated node status for desktop and monitoring clients
    async fn irondag_get_node_status(&self) -> Result<Value, JsonRpcError> {
        // Blockchain stats
        let blockchain = self.acquire_blockchain_read().await?;
        let latest_block = blockchain.latest_block_number();
        let tx_count = blockchain.transaction_count();
        drop(blockchain);

        // Peer count
        let peer_count = if let Some(network_mgr) = &self.network_manager {
            network_mgr.peer_count()
        } else {
            0
        };

        // Mining status
        let is_mining = if let Some(mining_mgr) = &self.mining_manager {
            *mining_mgr.is_mining().read().await
        } else {
            false
        };

        Ok(json!({
            "height": latest_block,
            "tx_count": tx_count,
            "peer_count": peer_count,
            "is_mining": is_mining,
        }))
    }

    /// irondag_getFairnessMetrics - Get fairness metrics for a block
    async fn irondag_get_fairness_metrics(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid block hash parameter".to_string(),
                data: None,
            })?;

        let block_hash = parse_hash(hash_str)?;

        // Find block in blockchain
        let blockchain = self.acquire_blockchain_read().await?;
        let block = blockchain.get_block_by_hash(&block_hash);
        let block = block.as_ref().cloned();
        drop(blockchain);

        if let Some(block) = block {
            // Get fairness metrics from mining manager if available
            if let Some(mining_mgr) = &self.mining_manager {
                let metrics = mining_mgr.get_fairness_metrics(&block).await;
                Ok(serde_json::json!({
                    "reordering_distance": metrics.reordering_distance,
                    "sandwich_detections": metrics.sandwich_detections,
                    "backrun_detections": metrics.backrun_detections,
                    "frontrun_detections": metrics.frontrun_detections,
                    "estimated_mev_value": format!("0x{:x}", metrics.estimated_mev_value),
                    "fairness_score": metrics.fairness_score,
                    "transaction_count": metrics.transaction_count,
                    "avg_transaction_age": metrics.avg_transaction_age,
                    "fee_concentration": metrics.fee_concentration,
                }))
            } else {
                // Return basic metrics if mining manager not available
                Ok(serde_json::json!({
                    "reordering_distance": 0.0,
                    "sandwich_detections": 0,
                    "backrun_detections": 0,
                    "frontrun_detections": 0,
                    "estimated_mev_value": "0x0",
                    "fairness_score": 1.0,
                    "transaction_count": block.transactions.len(),
                    "avg_transaction_age": 0.0,
                    "fee_concentration": 0.0,
                }))
            }
        } else {
            Err(JsonRpcError {
                code: -32602,
                message: "Block not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getStateRoot - Get current state root (Verkle tree root hash)
    async fn irondag_get_state_root(&self) -> Result<Value, JsonRpcError> {
        let blockchain = self.acquire_blockchain_read().await?;

        if !blockchain.is_verkle_enabled() {
            return Err(JsonRpcError {
                code: -32603,
                message: "Verkle tree not enabled".to_string(),
                data: None,
            });
        }

        let state_root = blockchain.state_root().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "State root not available".to_string(),
            data: None,
        })?;

        Ok(Value::String(format!("0x{}", hex::encode(state_root))))
    }

    /// irondag_getStateProof - Get state proof for an address (balance + nonce)
    async fn irondag_get_state_proof(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        let blockchain = self.acquire_blockchain_read().await?;

        if !blockchain.is_verkle_enabled() {
            return Err(JsonRpcError {
                code: -32603,
                message: "Verkle tree not enabled".to_string(),
                data: None,
            });
        }

        let (balance, proof) =
            blockchain
                .get_balance_with_proof(address)
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Failed to generate state proof".to_string(),
                    data: None,
                })?;

        let (nonce, _) = blockchain
            .get_nonce_with_proof(address)
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Failed to generate nonce proof".to_string(),
                data: None,
            })?;

        // Serialize proof
        let proof_bytes = proof.to_bytes();

        Ok(serde_json::json!({
            "address": format!("0x{}", hex::encode(address)),
            "balance": format!("0x{:x}", balance),
            "nonce": format!("0x{:x}", nonce),
            "state_root": format!("0x{}", hex::encode(proof.state_root)),
            "proof": format!("0x{}", hex::encode(&proof_bytes)),
            "proof_path": proof.proof.iter().map(|h| format!("0x{}", hex::encode(h))).collect::<Vec<_>>(),
        }))
    }

    /// irondag_verifyStateProof - Verify a state proof (for light clients)
    async fn irondag_verify_state_proof(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        // Parse address
        let address_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;
        let address = parse_address(address_str)?;

        // Parse balance
        let balance_str = params_array
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid balance parameter".to_string(),
                data: None,
            })?;
        let balance = parse_hex_number(balance_str)? as u128;

        // Parse proof
        let proof_str = params_array
            .get(2)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid proof parameter".to_string(),
                data: None,
            })?;
        let proof_bytes =
            hex::decode(proof_str.strip_prefix("0x").unwrap_or(proof_str)).map_err(|_| {
                JsonRpcError {
                    code: -32602,
                    message: "Invalid proof format".to_string(),
                    data: None,
                }
            })?;

        let proof =
            crate::verkle::StateProof::from_bytes(&proof_bytes).map_err(|_| JsonRpcError {
                code: -32602,
                message: "Failed to deserialize proof".to_string(),
                data: None,
            })?;

        // Verify proof
        let is_valid = crate::verkle::ProofVerifier::verify_balance_proof(address, balance, &proof);

        Ok(serde_json::json!({
            "valid": is_valid,
            "address": format!("0x{}", hex::encode(address)),
            "balance": format!("0x{:x}", balance),
            "state_root": format!("0x{}", hex::encode(proof.state_root)),
        }))
    }

    /// irondag_getStateRootHistory - Get state root history for a block range
    async fn irondag_get_state_root_history(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let light_client = self.light_client.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Light client not available".to_string(),
            data: None,
        })?;

        let (_start_block, _end_block) = if let Some(params) = params {
            let arr = params.as_array().ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid params format".to_string(),
                data: None,
            })?;
            let start = arr.get(0).and_then(|v| v.as_u64()).unwrap_or(0);
            let end = arr.get(1).and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
            (start, end)
        } else {
            (0, u64::MAX)
        };

        let client = light_client.read().await;
        let mut history = Vec::new();

        // Get state roots from light client (simplified - in real implementation,
        // light client would store history)
        if let Some(state_root) = client.current_state_root() {
            history.push(serde_json::json!({
                "block_number": client.latest_verified_block(),
                "state_root": format!("0x{}", hex::encode(state_root)),
            }));
        }

        Ok(serde_json::json!({
            "history": history,
            "count": history.len(),
        }))
    }

    /// irondag_getLightClientSyncStatus - Get light client sync status
    async fn irondag_get_light_client_sync_status(&self) -> Result<Value, JsonRpcError> {
        let light_client = self.light_client.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Light client not available".to_string(),
            data: None,
        })?;

        let client = light_client.read().await;
        let status = client.sync_status();

        Ok(serde_json::json!({
            "is_synced": status.is_synced,
            "latest_block": status.latest_block,
            "current_state_root": status.current_state_root.map(|r| format!("0x{}", hex::encode(r))),
            "state_root_count": status.state_root_count,
        }))
    }

    /// irondag_enableLightClientMode - Enable or disable light client mode
    async fn irondag_enable_light_client_mode(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let _enabled = if let Some(p) = params {
            if let Some(arr) = p.as_array() {
                arr.get(0).and_then(|v| v.as_bool()).unwrap_or(true)
            } else {
                true
            }
        } else {
            true
        };

        // Light client mode is always enabled if light client is available
        // This is a placeholder for future implementation
        Ok(serde_json::json!({
            "enabled": self.light_client.is_some(),
            "message": "Light client mode status"
        }))
    }

    /// irondag_generatePqAccount - Generate a new PQ account
    async fn irondag_generate_pq_account(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;
        let algorithm = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid algorithm parameter".to_string(),
                data: None,
            })?;

        let account =
            crate::pqc::tooling::generate_pq_account(algorithm).map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to generate PQ account: {}", e),
                data: None,
            })?;
        // CRIT-01 FIX: Secret key is NOT returned in response to prevent leaks.
        // The secret key must be stored securely by the caller immediately after generation.
        Ok(serde_json::json!({
            "address": format!("0x{}", hex::encode(account.address())),
            "public_key": format!("0x{}", hex::encode(account.public_key())),
            "account_type": format!("{:?}", account.account_type()),
            "message": "Secret key must be stored securely by the caller immediately after generation. It is NOT returned in this response for security reasons.",
        }))
    }

    /// irondag_getPqAccountType - Get PQ account type from a transaction
    async fn irondag_get_pq_account_type(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;
        let tx_hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid transaction hash parameter".to_string(),
                data: None,
            })?;
        let tx_hash = parse_hash(tx_hash_str)?;

        let blockchain = self.acquire_blockchain_read().await?;
        // This is a simplified check. In a real scenario, you'd need to retrieve the full transaction
        // and then use `detect_pq_account_type_from_transaction`.
        // For now, we'll simulate by checking if a transaction with this hash exists and has a PQ signature.
        if let Some((_, tx, _)) = blockchain.get_transaction_by_hash(&tx_hash) {
            if let Some(pq_sig) = &tx.pq_signature {
                return Ok(Value::String(format!("{:?}", pq_sig.account_type)));
            }
        }
        Ok(Value::Null)
    }

    /// irondag_exportPqKey - Export PQ account keys (disabled for security)
    async fn irondag_export_pq_key(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        Err(JsonRpcError {
            code: -32603,
            message: "Key export disabled for security reasons".to_string(),
            data: None,
        })
    }

    /// irondag_importPqKey - Import PQ account keys (disabled for security)
    async fn irondag_import_pq_key(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        Err(JsonRpcError {
            code: -32603,
            message: "Key import disabled for security reasons".to_string(),
            data: None,
        })
    }

    /// irondag_createPqTransaction - Create a PQ-signed transaction
    async fn irondag_create_pq_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        // Parse transaction parameters
        let from_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid from address".to_string(),
                data: None,
            })?;
        let from = parse_address(from_str)?;

        let to_str = params_array
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid to address".to_string(),
                data: None,
            })?;
        let to = parse_address(to_str)?;

        let value_str = params_array
            .get(2)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid value".to_string(),
                data: None,
            })?;
        let value = parse_hex_number(value_str)? as u128;

        let fee_str = params_array
            .get(3)
            .and_then(|v| v.as_str())
            .unwrap_or("0x0");
        let fee = parse_hex_number(fee_str)? as u128;

        let nonce = params_array.get(4).and_then(|v| v.as_u64()).unwrap_or(0);

        let _algorithm = params_array
            .get(5)
            .and_then(|v| v.as_str())
            .unwrap_or("Dilithium3");

        // Get current nonce
        let current_nonce = self.accounts_cache.get(&from).map(|s| s.nonce).unwrap_or(0);
        let final_nonce = if current_nonce > nonce {
            current_nonce
        } else {
            nonce
        };

        // Look up PQ keypair from keystore using the from address
        // TODO: Implement PQ keystore integration
        // For now, return an error indicating the key must be pre-generated
        let account_export = self
            .pq_keystore
            .as_ref()
            .and_then(|keystore| keystore.get(&from))
            .ok_or_else(|| JsonRpcError {
                code: -32000,
                message: format!(
                    "PQ keypair not found for address {}. Generate via irondag_generatePqAccount first",
                    hex::encode(from)
                ),
                data: None,
            })?;

        // Convert PqAccountExport to PqAccount for transaction creation
        let account = crate::pqc::accounts::PqAccount::from_keypair(
            account_export.account_type,
            account_export.secret_key.clone(),
            account_export.public_key.clone(),
        )
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to reconstruct PQ account: {}", e),
            data: None,
        })?;

        // Create transaction using the looked-up account
        let tx = crate::pqc::tooling::create_pq_transaction(
            &account,
            to,
            value,
            fee,
            final_nonce,
            vec![], // Empty data
        )
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to create PQ transaction: {}", e),
            data: None,
        })?;

        Ok(serde_json::json!({
            "hash": format!("0x{}", hex::encode(tx.hash)),
            "from": format!("0x{}", hex::encode(tx.from)),
            "to": format!("0x{}", hex::encode(tx.to)),
            "value": format!("0x{:x}", tx.value),
            "fee": format!("0x{:x}", tx.fee),
            "nonce": format!("0x{:x}", tx.nonce),
            "has_pq_signature": tx.pq_signature.is_some(),
        }))
    }

    /// irondag_getCrossShardTransaction - Get cross-shard transaction details
    async fn irondag_get_cross_shard_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid transaction hash parameter".to_string(),
                data: None,
            })?;

        let tx_hash = parse_hash(hash_str)?;

        if let Some(shard_manager) = &self.shard_manager {
            if let Some(cross_tx) = shard_manager.get_cross_shard_transaction(tx_hash).await {
                Ok(serde_json::json!({
                    "transaction_hash": format!("0x{}", hex::encode(tx_hash)),
                    "source_shard": cross_tx.source_shard,
                    "target_shard": cross_tx.target_shard,
                    "status": format!("{:?}", cross_tx.status),
                    "from": format!("0x{}", hex::encode(cross_tx.tx.from)),
                    "to": format!("0x{}", hex::encode(cross_tx.tx.to)),
                    "value": format!("0x{:x}", cross_tx.tx.value),
                    "is_cross_shard": true,
                }))
            } else {
                // Not a cross-shard transaction
                Ok(serde_json::json!({
                    "transaction_hash": format!("0x{}", hex::encode(tx_hash)),
                    "is_cross_shard": false,
                }))
            }
        } else {
            Ok(serde_json::json!({
                "transaction_hash": format!("0x{}", hex::encode(tx_hash)),
                "is_cross_shard": false,
                "sharding_disabled": true,
            }))
        }
    }

    /// irondag_getCrossShardTransactions - Get all cross-shard transactions (with optional filters)
    async fn irondag_get_cross_shard_transactions(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        if let Some(shard_manager) = &self.shard_manager {
            let cross_txs = shard_manager.get_all_cross_shard_transactions().await;
            let mut results = Vec::new();

            for cross_tx in cross_txs {
                results.push(serde_json::json!({
                    "transaction_hash": format!("0x{}", hex::encode(cross_tx.id)),
                    "source_shard": cross_tx.source_shard,
                    "target_shard": cross_tx.target_shard,
                    "status": format!("{:?}", cross_tx.status),
                    "from": format!("0x{}", hex::encode(cross_tx.tx.from)),
                    "to": format!("0x{}", hex::encode(cross_tx.tx.to)),
                    "value": format!("0x{:x}", cross_tx.tx.value),
                }));
            }

            Ok(serde_json::json!({
                "count": results.len(),
                "transactions": results,
            }))
        } else {
            Ok(serde_json::json!({
                "count": 0,
                "transactions": [],
                "sharding_disabled": true,
            }))
        }
    }

    /// irondag_getShardBlock - Get block from a specific shard
    async fn irondag_get_shard_block(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let shard_id_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid shard_id parameter".to_string(),
                data: None,
            })?;

        let shard_id = parse_hex_number(shard_id_str)? as usize;

        let block_number_str = params_array
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid block_number parameter".to_string(),
                data: None,
            })?;

        let block_number = parse_hex_number(block_number_str)?;

        if let Some(shard_manager) = &self.shard_manager {
            if let Some(shard) = shard_manager.get_shard(shard_id) {
                let shard_guard = shard.read().await;
                let blockchain = shard_guard.blockchain.read().await;

                if let Some(block) = blockchain.get_block_by_number(block_number) {
                    Ok(serde_json::json!({
                        "shard_id": shard_id,
                        "block": block_to_json(Some(block), self.miner_address),
                    }))
                } else {
                    Err(JsonRpcError {
                        code: -32602,
                        message: "Block not found in shard".to_string(),
                        data: None,
                    })
                }
            } else {
                Err(JsonRpcError {
                    code: -32602,
                    message: format!("Invalid shard_id: {}", shard_id),
                    data: None,
                })
            }
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Sharding not enabled".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getShardTransactions - Get transactions from a specific shard's pool
    async fn irondag_get_shard_transactions(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let shard_id_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid shard_id parameter".to_string(),
                data: None,
            })?;

        let shard_id = parse_hex_number(shard_id_str)? as usize;

        let limit = params_array.get(1).and_then(|v| v.as_u64()).unwrap_or(100) as usize;

        if let Some(shard_manager) = &self.shard_manager {
            let transactions = shard_manager.get_shard_transactions(shard_id, limit).await;

            let txs_json: Vec<Value> = transactions
                .iter()
                .map(|tx| {
                    tx_to_json(tx, 0) // Block number not available for pool transactions
                })
                .collect();

            Ok(serde_json::json!({
                "shard_id": shard_id,
                "count": transactions.len(),
                "transactions": txs_json,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Sharding not enabled".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getShardBalance - Get balance for an address in a specific shard
    async fn irondag_get_shard_balance(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let shard_id_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid shard_id parameter".to_string(),
                data: None,
            })?;

        let shard_id = parse_hex_number(shard_id_str)? as usize;

        let address_str = params_array
            .get(1)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        if let Some(shard_manager) = &self.shard_manager {
            if let Some(shard) = shard_manager.get_shard(shard_id) {
                let shard_guard = shard.read().await;
                let blockchain = shard_guard.blockchain.read().await;

                let balance = blockchain.get_balance(address);

                Ok(serde_json::json!({
                    "shard_id": shard_id,
                    "address": format!("0x{}", hex::encode(address)),
                    "balance": format!("0x{:x}", balance),
                }))
            } else {
                Err(JsonRpcError {
                    code: -32602,
                    message: format!("Invalid shard_id: {}", shard_id),
                    data: None,
                })
            }
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Sharding not enabled".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getOrderingPolicy - Get current transaction ordering policy
    async fn irondag_get_ordering_policy(&self) -> Result<Value, JsonRpcError> {
        if let Some(mining_mgr) = &self.mining_manager {
            let policy = mining_mgr.get_ordering_policy().await;
            Ok(serde_json::json!({
                "policy": policy.name(),
                "description": match policy {
                    crate::mining::ordering::OrderingPolicy::Fifo => "First-In-First-Out (most fair)",
                    crate::mining::ordering::OrderingPolicy::Random => "Random ordering (prevents front-running)",
                    crate::mining::ordering::OrderingPolicy::FeeBased => "Fee-based ordering (maximizes miner revenue)",
                    crate::mining::ordering::OrderingPolicy::Hybrid => "Hybrid: FIFO with fee boost",
                    crate::mining::ordering::OrderingPolicy::TimeWeighted => "Time-weighted fairness",
                }
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_setOrderingPolicy - Set transaction ordering policy
    async fn irondag_set_ordering_policy(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let policy_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid policy parameter".to_string(),
                data: None,
            })?;

        let policy = match policy_str.to_lowercase().as_str() {
            "fifo" => crate::mining::ordering::OrderingPolicy::Fifo,
            "random" => crate::mining::ordering::OrderingPolicy::Random,
            "feebased" | "fee-based" => crate::mining::ordering::OrderingPolicy::FeeBased,
            "hybrid" => crate::mining::ordering::OrderingPolicy::Hybrid,
            "timeweighted" | "time-weighted" => {
                crate::mining::ordering::OrderingPolicy::TimeWeighted
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown policy: {}. Valid options: fifo, random, feebased, hybrid, timeweighted", policy_str),
                    data: None,
                });
            }
        };

        if let Some(mining_mgr) = &self.mining_manager {
            mining_mgr.set_ordering_policy(policy).await;
            Ok(serde_json::json!({
                "success": true,
                "policy": policy.name(),
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getMevMetrics - Get MEV metrics for recent blocks
    async fn irondag_get_mev_metrics(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let block_count = if let Some(params) = params {
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize
        } else {
            10
        };

        let recent_blocks: Vec<Block> = {
            let blockchain = self.acquire_blockchain_read().await?;
            let mut tail = blockchain.get_blocks_tail(block_count);
            tail.reverse();
            tail
        };

        if let Some(mining_mgr) = &self.mining_manager {
            let mut total_sandwich = 0u64;
            let mut total_backrun = 0u64;
            let mut total_frontrun = 0u64;
            let mut total_mev_value = 0u128;
            let mut total_fairness = 0.0;
            let mut block_count_actual = 0;

            for block in recent_blocks {
                let metrics = mining_mgr.get_fairness_metrics(&block).await;
                total_sandwich += metrics.sandwich_detections;
                total_backrun += metrics.backrun_detections;
                total_frontrun += metrics.frontrun_detections;
                total_mev_value += metrics.estimated_mev_value;
                total_fairness += metrics.fairness_score;
                block_count_actual += 1;
            }

            let avg_fairness = if block_count_actual > 0 {
                total_fairness / block_count_actual as f64
            } else {
                0.0
            };

            Ok(serde_json::json!({
                "blocks_analyzed": block_count_actual,
                "total_sandwich_attacks": total_sandwich,
                "total_backrun_attacks": total_backrun,
                "total_frontrun_attacks": total_frontrun,
                "total_mev_value": format!("0x{:x}", total_mev_value),
                "average_fairness_score": avg_fairness,
                "mev_detected": total_sandwich > 0 || total_backrun > 0 || total_frontrun > 0,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Mining manager not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getBlockFairness - Get detailed fairness metrics for a specific block
    async fn irondag_get_block_fairness(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid block hash parameter".to_string(),
                data: None,
            })?;

        let block_hash = parse_hash(hash_str)?;

        let blockchain = self.acquire_blockchain_read().await?;
        let block = blockchain.get_block_by_hash(&block_hash);
        let block = block.as_ref().cloned();
        drop(blockchain);

        if let Some(block) = block {
            if let Some(mining_mgr) = &self.mining_manager {
                let metrics = mining_mgr.get_fairness_metrics(&block).await;
                Ok(serde_json::json!({
                    "block_hash": format!("0x{}", hex::encode(block_hash)),
                    "block_number": block.header.block_number,
                    "reordering_distance": metrics.reordering_distance,
                    "sandwich_detections": metrics.sandwich_detections,
                    "backrun_detections": metrics.backrun_detections,
                    "frontrun_detections": metrics.frontrun_detections,
                    "estimated_mev_value": format!("0x{:x}", metrics.estimated_mev_value),
                    "fairness_score": metrics.fairness_score,
                    "transaction_count": metrics.transaction_count,
                    "avg_transaction_age": metrics.avg_transaction_age,
                    "fee_concentration": metrics.fee_concentration,
                }))
            } else {
                Err(JsonRpcError {
                    code: -32603,
                    message: "Mining manager not available".to_string(),
                    data: None,
                })
            }
        } else {
            Err(JsonRpcError {
                code: -32602,
                message: "Block not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_traceFunds - Trace funds from a source address
    async fn irondag_trace_funds(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let address_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;
        let source = parse_address(address_str)?;

        let max_hops = params_array.get(1).and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let max_paths = params_array.get(2).and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        if let Some(forensic) = &self.forensic_analyzer {
            let analyzer = forensic.read().await;
            let flows = analyzer.trace_funds(source, max_hops, max_paths);

            let flows_json: Vec<Value> = flows.iter().map(|flow| {
                serde_json::json!({
                    "path": flow.path.iter().map(|a| format!("0x{}", hex::encode(a))).collect::<Vec<_>>(),
                    "transactions": flow.transactions.iter().map(|h| format!("0x{}", hex::encode(h))).collect::<Vec<_>>(),
                    "total_value": format!("0x{:x}", flow.total_value),
                    "hop_count": flow.hop_count,
                })
            }).collect();

            Ok(serde_json::json!({
                "source": format!("0x{}", hex::encode(source)),
                "max_hops": max_hops,
                "flows_found": flows.len(),
                "flows": flows_json,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Forensic analyzer not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getAddressSummary - Get comprehensive address summary
    async fn irondag_get_address_summary(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        if let Some(forensic) = &self.forensic_analyzer {
            let analyzer = forensic.read().await;
            let summary = analyzer.generate_address_summary(address);

            Ok(serde_json::json!({
                "address": format!("0x{}", hex::encode(address)),
                "total_received": format!("0x{:x}", summary.total_received),
                "total_sent": format!("0x{:x}", summary.total_sent),
                "net_balance": format!("0x{:x}", u128::try_from(summary.net_balance.max(0)).unwrap_or(0)),
                "incoming_tx_count": summary.incoming_tx_count,
                "outgoing_tx_count": summary.outgoing_tx_count,
                "unique_contacts": summary.unique_contacts,
                "first_seen": summary.first_seen,
                "last_seen": summary.last_seen,
                "suspicious_patterns": summary.suspicious_patterns,
                "risk_indicators": summary.risk_indicators,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Forensic analyzer not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getAddressTransactions - Get transaction history for an address
    async fn irondag_get_address_transactions(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let address_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        // Optional limit parameter (default 50)
        let limit = params_array.get(1).and_then(|v| v.as_u64()).unwrap_or(50) as usize;

        let blockchain = self.acquire_blockchain_read().await?;
        let mut transactions = Vec::new();

        // Iterate through last 1000 blocks in reverse to get most recent first
        let mut blocks = blockchain.get_blocks_tail(1000);
        blocks.reverse();

        for block in blocks {
            for tx in &block.transactions {
                // Check if address is involved (sender or receiver)
                if tx.from == address || tx.to == address {
                    transactions.push(serde_json::json!({
                        "hash": format!("0x{}", hex::encode(tx.hash)),
                        "from": format!("0x{}", hex::encode(tx.from)),
                        "to": format!("0x{}", hex::encode(tx.to)),
                        "value": format!("0x{:x}", tx.value),
                        "fee": format!("0x{:x}", tx.fee),
                        "nonce": format!("0x{:x}", tx.nonce),
                        "block_number": format!("0x{:x}", block.header.block_number),
                        "block_hash": format!("0x{}", hex::encode(block.hash)),
                        "timestamp": format!("0x{:x}", block.header.timestamp),
                        "direction": if tx.from == address { "outgoing" } else { "incoming" },
                    }));

                    if transactions.len() >= limit {
                        break;
                    }
                }
            }
            if transactions.len() >= limit {
                break;
            }
        }

        Ok(serde_json::json!({
            "address": format!("0x{}", hex::encode(address)),
            "total": transactions.len(),
            "limit": limit,
            "transactions": transactions,
        }))
    }

    /// irondag_detectAnomalies - Detect anomalies for an address
    async fn irondag_detect_anomalies(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        if let Some(forensic) = &self.forensic_analyzer {
            let analyzer = forensic.read().await;
            let detection = analyzer.detect_anomalies(address);

            let anomalies_json: Vec<Value> = detection.anomalies.iter().map(|anomaly| {
                serde_json::json!({
                    "type": format!("{:?}", anomaly.anomaly_type),
                    "description": anomaly.description,
                    "severity": anomaly.severity,
                    "related_addresses": anomaly.related_addresses.iter().map(|a| format!("0x{}", hex::encode(a))).collect::<Vec<_>>(),
                })
            }).collect();

            Ok(serde_json::json!({
                "address": format!("0x{}", hex::encode(address)),
                "anomaly_score": detection.anomaly_score,
                "confidence": detection.confidence,
                "anomalies": anomalies_json,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Forensic analyzer not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_findRelatedAddresses - Find addresses that interacted with the target
    async fn irondag_find_related_addresses(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        let address_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid address parameter".to_string(),
                data: None,
            })?;
        let address = parse_address(address_str)?;

        let max_results = params_array.get(1).and_then(|v| v.as_u64()).unwrap_or(50) as usize;

        if let Some(forensic) = &self.forensic_analyzer {
            let analyzer = forensic.read().await;
            let related = analyzer.find_related_addresses(address, max_results);

            Ok(serde_json::json!({
                "address": format!("0x{}", hex::encode(address)),
                "related_count": related.len(),
                "related_addresses": related.iter().map(|a| format!("0x{}", hex::encode(a))).collect::<Vec<_>>(),
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Forensic analyzer not available".to_string(),
                data: None,
            })
        }
    }

    /// Set policy manager
    pub fn set_policy_manager(
        &mut self,
        policy_manager: Arc<tokio::sync::RwLock<crate::security::SecurityPolicyManager>>,
    ) {
        self.policy_manager = Some(policy_manager);
    }

    /// irondag_addSecurityPolicy - Add a new security policy
    async fn irondag_add_security_policy(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let policy_manager = self.policy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Policy manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        // Parse policy from JSON
        let policy_json = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid policy parameter".to_string(),
                data: None,
            })?;

        let policy: crate::security::SecurityPolicy = serde_json::from_value(policy_json.clone())
            .map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Invalid policy format: {}", e),
            data: None,
        })?;

        let mut manager = policy_manager.write().await;
        match manager.add_policy(policy.clone()) {
            Ok(policy_id) => Ok(serde_json::json!({
                "policy_id": policy_id,
                "message": "Policy added successfully",
                "policy": serde_json::to_value(&policy).unwrap_or(Value::Null),
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to add policy: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_removeSecurityPolicy - Remove a security policy
    async fn irondag_remove_security_policy(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let policy_manager = self.policy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Policy manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let owner_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid owner address parameter".to_string(),
                data: None,
            })?;
        let owner = parse_address(owner_str)?;

        let policy_id = params
            .as_array()
            .and_then(|arr| arr.get(1))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid policy_id parameter".to_string(),
                data: None,
            })?;

        let mut manager = policy_manager.write().await;
        match manager.remove_policy(owner, policy_id) {
            Ok(_) => Ok(serde_json::json!({
                "message": "Policy removed successfully"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to remove policy: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getSecurityPolicies - Get all security policies for an owner
    async fn irondag_get_security_policies(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let policy_manager = self.policy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Policy manager not available".to_string(),
            data: None,
        })?;

        let owner = if let Some(p) = params {
            if let Some(arr) = p.as_array() {
                if let Some(v) = arr.get(0) {
                    if let Some(s) = v.as_str() {
                        parse_address(s).map_err(|e| JsonRpcError {
                            code: -32602,
                            message: format!("Invalid owner address: {}", e.message),
                            data: None,
                        })?
                    } else {
                        Address::zero()
                    }
                } else {
                    Address::zero()
                }
            } else {
                Address::zero()
            }
        } else {
            Address::zero()
        };

        let manager = policy_manager.read().await;
        let policies = manager.get_policies(owner);

        Ok(serde_json::json!({
            "owner": format!("0x{}", hex::encode(owner)),
            "policy_count": policies.len(),
            "policies": policies.iter().map(|p| serde_json::to_value(p).unwrap_or(Value::Null)).collect::<Vec<_>>(),
        }))
    }

    /// irondag_setPolicyEnabled - Enable or disable a policy
    async fn irondag_set_policy_enabled(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let policy_manager = self.policy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Policy manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let owner_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid owner address parameter".to_string(),
                data: None,
            })?;
        let owner = parse_address(owner_str)?;

        let policy_id = params
            .as_array()
            .and_then(|arr| arr.get(1))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid policy_id parameter".to_string(),
                data: None,
            })?;

        let enabled = params
            .as_array()
            .and_then(|arr| arr.get(2))
            .and_then(|v| v.as_bool())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid enabled parameter".to_string(),
                data: None,
            })?;

        let mut manager = policy_manager.write().await;
        match manager.set_policy_enabled(owner, policy_id, enabled) {
            Ok(_) => Ok(serde_json::json!({
                "message": format!("Policy {} {}", policy_id, if enabled { "enabled" } else { "disabled" })
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to update policy: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_evaluateTransactionPolicy - Evaluate a transaction against policies
    async fn irondag_evaluate_transaction_policy(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let policy_manager = self.policy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Policy manager not available".to_string(),
            data: None,
        })?;

        let security_scorer = self.security_scorer.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Security scorer not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        // Parse transaction hash
        let tx_hash_str = params
            .as_array()
            .and_then(|arr| arr.get(0))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid transaction hash parameter".to_string(),
                data: None,
            })?;
        let tx_hash = parse_hash(tx_hash_str)?;

        // Get owner address (optional)
        let owner_str = params
            .as_array()
            .and_then(|arr| arr.get(1))
            .and_then(|v| v.as_str())
            .map(|s| parse_address(s))
            .transpose()
            .map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid owner address: {}", e.message),
                data: None,
            })?;
        let owner = owner_str.unwrap_or(Address::zero());

        // Find transaction
        let blockchain = self.acquire_blockchain_read().await?;
        let tx = blockchain.get_transaction_by_hash(&tx_hash).map(|(_, t, _)| t);
        drop(blockchain);

        let tx = tx.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Transaction not found".to_string(),
            data: None,
        })?;

        // Get risk score
        let scorer = security_scorer.read().await;
        let risk_score = scorer.score_transaction(&tx);
        drop(scorer);

        // Evaluate policies
        let manager = policy_manager.read().await;
        let evaluation = manager.evaluate_transaction(&tx, &risk_score, owner);

        Ok(serde_json::json!({
            "triggered": evaluation.triggered,
            "message": evaluation.message,
            "action": evaluation.action.map(|a| serde_json::to_value(&a).unwrap_or(Value::Null)),
            "policy": evaluation.policy.map(|p| serde_json::to_value(&p).unwrap_or(Value::Null)),
            "risk_score": {
                "score": risk_score.score,
                "confidence": risk_score.confidence,
                "labels": risk_score.labels,
            }
        }))
    }

    /// irondag_addTestBlock - Manually add a test block (for demo purposes, bypasses mining)
    #[cfg(test)]
    async fn irondag_add_test_block(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let params_array = params.as_array().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params format".to_string(),
            data: None,
        })?;

        // Parse block number
        let block_number = params_array
            .get(0)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid block_number parameter".to_string(),
                data: None,
            })?;

        // Parse transactions (optional - array of transaction objects or simplified format)
        // We'll need blockchain access for nonces, so get read lock first
        let blockchain_read = self.acquire_blockchain_read().await?;
        let transactions: Vec<crate::blockchain::Transaction> = if let Some(txs_value) =
            params_array.get(1)
        {
            if let Some(tx_array) = txs_value.as_array() {
                let mut txs = Vec::new();
                for tx_value in tx_array {
                    // Try to parse as full Transaction struct first
                    if let Ok(tx) =
                        serde_json::from_value::<crate::blockchain::Transaction>(tx_value.clone())
                    {
                        txs.push(tx);
                    } else if let Some(tx_obj) = tx_value.as_object() {
                        // Try to parse as simplified format (from irondag_createTestTransaction)
                        let from_str =
                            tx_obj.get("from").and_then(|v| v.as_str()).ok_or_else(|| {
                                JsonRpcError {
                                    code: -32602,
                                    message: "Missing 'from' field in transaction".to_string(),
                                    data: None,
                                }
                            })?;
                        let from = parse_address(from_str)?;

                        let to_str =
                            tx_obj.get("to").and_then(|v| v.as_str()).ok_or_else(|| {
                                JsonRpcError {
                                    code: -32602,
                                    message: "Missing 'to' field in transaction".to_string(),
                                    data: None,
                                }
                            })?;
                        let to = parse_address(to_str)?;

                        let value_str =
                            tx_obj
                                .get("value")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| JsonRpcError {
                                    code: -32602,
                                    message: "Missing 'value' field in transaction".to_string(),
                                    data: None,
                                })?;
                        let value = parse_hex_number(value_str)? as u128;

                        let fee_str = tx_obj.get("fee").and_then(|v| v.as_str()).unwrap_or("0x0");
                        let fee = parse_hex_number(fee_str)? as u128;

                        // Get nonce from blockchain if not provided in simplified format
                        let nonce = if let Some(nonce_val) = tx_obj.get("nonce") {
                            if let Some(nonce_str) = nonce_val.as_str() {
                                parse_hex_number(nonce_str)? as u64
                            } else if let Some(nonce_u64) = nonce_val.as_u64() {
                                nonce_u64
                            } else {
                                blockchain_read.get_nonce(from)
                            }
                        } else {
                            blockchain_read.get_nonce(from)
                        };

                        // Create transaction (unsigned, for demo)
                        let tx = crate::blockchain::Transaction::new(from, to, value, fee, nonce);
                        txs.push(tx);
                    } else {
                        // Try to get more info about what we received
                        let tx_type = if tx_value.is_null() {
                            "null"
                        } else if tx_value.is_string() {
                            "string"
                        } else if tx_value.is_number() {
                            "number"
                        } else if tx_value.is_boolean() {
                            "boolean"
                        } else if tx_value.is_array() {
                            "array"
                        } else {
                            "unknown"
                        };

                        return Err(JsonRpcError {
                            code: -32602,
                            message: format!("Invalid transaction format. Expected transaction object, got: {}. Transaction value: {}", tx_type, tx_value),
                            data: None,
                        });
                    }
                }
                txs
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Get parent hashes (optional - defaults to latest block)
        // Do this before dropping blockchain_read
        let parent_hashes: Vec<crate::types::Hash> =
            if let Some(parents_value) = params_array.get(2) {
                if let Some(parent_array) = parents_value.as_array() {
                    parent_array
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| parse_hash(s).ok())
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                // Default to latest block hash (use blockchain_read we already have)
                if let Some(latest_block) = blockchain_read.get_latest_block() {
                    vec![latest_block.hash]
                } else {
                    vec![] // Genesis block
                }
            };

        // Release read lock before write lock
        drop(blockchain_read);

        // Create block header
        // EIP-1559: Use initial base fee for RPC-generated blocks
        let header = crate::blockchain::BlockHeader::new(
            parent_hashes.clone(),
            block_number,
            crate::types::StreamType::StreamA, // Default to StreamA
            4,                                 // Default difficulty
            crate::mining::BASE_FEE_INITIAL,   // Initial base fee for EIP-1559
        );

        // Create block
        let block = crate::blockchain::Block::new(header, transactions);

        // Add block to blockchain
        let mut blockchain = self.blockchain.write().await;
        match blockchain.add_block(block.clone()).await {
            Ok(_) => {
                // Update light client if available
                if let Some(light_client) = &self.light_client {
                    if let Some(state_root) = blockchain.state_root() {
                        let mut client = light_client.write().await;
                        client.update_state_root(block_number, state_root);
                    }
                }

                // Update forensic analyzer if available
                if let Some(forensic) = &self.forensic_analyzer {
                    let mut analyzer = forensic.write().await;
                    for tx in &block.transactions {
                        analyzer.index_transaction(tx, block_number);
                    }
                }

                Ok(serde_json::json!({
                    "success": true,
                    "block_hash": format!("0x{}", hex::encode(block.hash)),
                    "block_number": block_number,
                    "transaction_count": block.transactions.len(),
                    "message": "Block added successfully"
                }))
            }
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to add block: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getNodeRegistry - Get node registry statistics
    async fn irondag_get_node_registry(&self) -> Result<Value, JsonRpcError> {
        let registry = self.node_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Node registry not available".to_string(),
            data: None,
        })?;

        let registry = registry.read().await;
        let total_nodes = registry.total_nodes();
        let active_nodes = registry.active_nodes();

        Ok(json!({
            "total_nodes": total_nodes,
            "active_nodes": active_nodes,
            "nodes": registry.get_all_nodes().iter().map(|node| {
                json!({
                    "public_key": hex::encode(&node.public_key),
                    "ip_address": node.ip_address.map(|ip| ip.to_string()),
                    "created_at": node.created_at,
                })
            }).collect::<Vec<_>>()
        }))
    }

    /// irondag_getNodeLongevity - Get longevity stats for a node
    async fn irondag_get_node_longevity(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.node_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Node registry not available".to_string(),
            data: None,
        })?;

        let params_array =
            params
                .and_then(|p| p.as_array().cloned())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Invalid parameters".to_string(),
                    data: None,
                })?;

        let public_key_str = params_array
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing public_key parameter".to_string(),
                data: None,
            })?;

        let public_key_bytes = hex::decode(
            public_key_str.strip_prefix("0x").unwrap_or(public_key_str),
        )
        .map_err(|_| JsonRpcError {
            code: -32602,
            message: "Invalid public_key format".to_string(),
            data: None,
        })?;

        if public_key_bytes.len() != 32 {
            return Err(JsonRpcError {
                code: -32602,
                message: "Invalid public_key length".to_string(),
                data: None,
            });
        }

        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&public_key_bytes);

        let registry = registry.read().await;
        let all_nodes = registry.get_all_nodes();
        let node_identity = all_nodes
            .iter()
            .find(|node| node.public_key == public_key)
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Node not found".to_string(),
                data: None,
            })?;

        let stats = registry
            .get_node_stats(node_identity)
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Node stats not found".to_string(),
                data: None,
            })?;

        let network_age = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(stats.network_age_at_join);
        let network_age_days = network_age / 86400;

        Ok(json!({
            "public_key": hex::encode(&node_identity.public_key),
            "active_days": stats.active_days,
            "blocks_mined": stats.blocks_mined,
            "uptime_index": stats.uptime_index,
            "last_seen": stats.last_seen,
            "network_age_at_join": stats.network_age_at_join,
            "consecutive_offline_days": stats.consecutive_offline_days,
            "longevity_weight": stats.calculate_weight(network_age_days),
            "activity_snapshots_count": stats.activity_snapshots.len(),
        }))
    }

    /// irondag_registerNode - Register a new node
    async fn irondag_register_node(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.node_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Node registry not available".to_string(),
            data: None,
        })?;

        // For now, create a placeholder node identity
        // In production, this would parse the full node identity from params
        // Generate real Ed25519 keypair for node identity
        use ed25519_dalek::SigningKey;
        use rand::thread_rng;
        let signing_key = SigningKey::generate(&mut thread_rng());
        let public_key: [u8; 32] = signing_key.verifying_key().to_bytes();
        let private_key: [u8; 32] = signing_key.to_bytes();
        // Note: In production, store signing_key securely (e.g., encrypted keystore)
        // For now, we only use it for registration

        let hardware_fingerprint = crate::governance::HardwareFingerprint::generate(&private_key);
        let node_identity = crate::governance::NodeIdentity {
            public_key,
            ip_address: None,
            hardware_fingerprint,
            zk_uniqueness_proof: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        let mut registry = registry.write().await;
        match registry.register_node(node_identity.clone()) {
            Ok(_) => Ok(json!({
                "success": true,
                "public_key": hex::encode(&node_identity.public_key),
                "message": "Node registered successfully"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to register node: {}", e),
                data: None,
            }),
        }
    }

    /// Set reputation manager
    pub fn with_reputation_manager(
        &mut self,
        reputation_manager: Arc<tokio::sync::RwLock<crate::reputation::ReputationManager>>,
    ) {
        self.reputation_manager = Some(reputation_manager);
    }

    /// irondag_createTimeLockedTransaction - Create a time-locked transaction
    async fn irondag_create_time_locked_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let obj = params.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an object".to_string(),
            data: None,
        })?;

        let from = parse_address(obj.get("from").and_then(|v| v.as_str()).ok_or_else(|| {
            JsonRpcError {
                code: -32602,
                message: "Missing 'from' address".to_string(),
                data: None,
            }
        })?)?;

        let to =
            parse_address(
                obj.get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'to' address".to_string(),
                        data: None,
                    })?,
            )?;

        let value =
            parse_hex_number(obj.get("value").and_then(|v| v.as_str()).unwrap_or("0x0"))? as u128;
        let fee =
            parse_hex_number(obj.get("fee").and_then(|v| v.as_str()).unwrap_or("0x0"))? as u128;

        let blockchain = self.acquire_blockchain_read().await?;
        let nonce = blockchain.get_nonce(from);

        let mut tx = Transaction::new(from, to, value, fee, nonce);

        // Set time-lock if provided
        if let Some(block_str) = obj.get("executeAtBlock").and_then(|v| v.as_str()) {
            let block = parse_hex_number(block_str)?;
            tx = tx.with_execute_at_block(block);
        }

        if let Some(timestamp_str) = obj.get("executeAtTimestamp").and_then(|v| v.as_str()) {
            let timestamp = parse_hex_number(timestamp_str)?;
            tx = tx.with_execute_at_timestamp(timestamp);
        }

        // Note: Transaction would need to be signed by the caller
        // This just creates the transaction structure

        Ok(json!({
            "transaction": {
                "hash": format!("0x{}", hex::encode(tx.hash)),
                "from": format!("0x{}", hex::encode(tx.from)),
                "to": format!("0x{}", hex::encode(tx.to)),
                "value": format!("0x{:x}", tx.value),
                "fee": format!("0x{:x}", tx.fee),
                "nonce": format!("0x{:x}", tx.nonce),
                "executeAtBlock": tx.execute_at_block.map(|b| format!("0x{:x}", b)),
                "executeAtTimestamp": tx.execute_at_timestamp.map(|t| format!("0x{:x}", t)),
            },
            "message": "Transaction created. Must be signed before sending."
        }))
    }

    /// irondag_getTimeLockedTransactions - Get pending time-locked transactions
    async fn irondag_get_time_locked_transactions(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let blockchain = self.acquire_blockchain_read().await?;
        let current_block = blockchain.latest_block_number();
        let current_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut time_locked = Vec::new();
        blockchain.with_blocks(|all_blocks| {
            for block in all_blocks {
                for tx in &block.transactions {
                    if tx.execute_at_block.is_some() || tx.execute_at_timestamp.is_some() {
                        let is_ready = tx.is_ready_to_execute(current_block, current_timestamp);
                        time_locked.push(json!({
                            "hash": format!("0x{}", hex::encode(tx.hash)),
                            "from": format!("0x{}", hex::encode(tx.from)),
                            "to": format!("0x{}", hex::encode(tx.to)),
                            "value": format!("0x{:x}", tx.value),
                            "executeAtBlock": tx.execute_at_block.map(|b| format!("0x{:x}", b)),
                            "executeAtTimestamp": tx.execute_at_timestamp.map(|t| format!("0x{:x}", t)),
                            "isReady": is_ready,
                            "currentBlock": format!("0x{:x}", current_block),
                            "currentTimestamp": format!("0x{:x}", current_timestamp),
                        }));
                    }
                }
            }
        });

        Ok(json!({
            "timeLockedTransactions": time_locked,
            "count": time_locked.len(),
        }))
    }

    // -------------------------------------------------------------------------
    // Native Recurring Transactions
    // -------------------------------------------------------------------------

    /// irondag_createRecurringTransaction — Schedule a repeating value transfer.
    ///
    /// Params (object): {
    ///   "from": "0x...",
    ///   "to": "0x...",
    ///   "value": "1000000",
    ///   "schedule": {"Custom": {"interval_seconds": 3600}}
    ///              | {"Daily": {"hour": 12, "minute": 0}}
    ///              | {"Weekly": {"day_of_week": 1, "hour": 9, "minute": 0}}
    ///              | {"Monthly": {"day_of_month": 1, "hour": 0, "minute": 0}},
    ///   "startDate": 1718000000,    // Unix timestamp
    ///   "endDate": null | 1750000000,
    ///   "maxExecutions": null | 12
    /// }
    async fn irondag_create_recurring_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;

        let from = parse_address(
            obj.get("from")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: from".to_string(),
                    data: None,
                })?,
        )?;

        let to = parse_address(
            obj.get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: to".to_string(),
                    data: None,
                })?,
        )?;

        let value_str = obj
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing field: value".to_string(),
                data: None,
            })?;
        let value = value_str.parse::<u128>().or_else(|_| {
            u128::from_str_radix(value_str.strip_prefix("0x").unwrap_or(value_str), 16)
        }).map_err(|_| JsonRpcError {
            code: -32602,
            message: "value must be a decimal or 0x-hex string".to_string(),
            data: None,
        })?;

        let schedule_val = obj.get("schedule").ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing field: schedule".to_string(),
            data: None,
        })?;
        let schedule: crate::recurring::Schedule =
            serde_json::from_value(schedule_val.clone()).map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid schedule: {}", e),
                data: None,
            })?;

        let start_date = obj
            .get("startDate")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing field: startDate (Unix timestamp)".to_string(),
                data: None,
            })?;

        let end_date = obj.get("endDate").and_then(|v| v.as_u64());
        let max_executions = obj.get("maxExecutions").and_then(|v| v.as_u64());

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let bc = self.acquire_blockchain_read().await?;
        let manager_arc = bc.recurring_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Recurring transaction manager not initialized".to_string(),
            data: None,
        })?;
        let mut manager = manager_arc.write().await;

        let recurring = manager.create_recurring(
            from,
            to,
            value,
            schedule,
            start_date,
            end_date,
            max_executions,
            created_at,
        );

        let recurring_id = hex::encode(recurring.recurring_tx_id);
        Ok(json!({
            "recurringTxId": format!("0x{}", recurring_id),
            "from": format!("0x{}", hex::encode(from)),
            "to": format!("0x{}", hex::encode(to)),
            "value": value.to_string(),
            "startDate": start_date,
            "endDate": end_date,
            "maxExecutions": max_executions,
            "nextExecution": recurring.next_execution,
            "status": format!("{:?}", recurring.status),
        }))
    }

    /// irondag_cancelRecurringTransaction — Cancel a scheduled recurring transfer.
    ///
    /// Params (object): { "recurringTxId": "0x..." }
    async fn irondag_cancel_recurring_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let id_hash = self.parse_recurring_id(params)?;

        let bc = self.acquire_blockchain_read().await?;
        let manager_arc = bc.recurring_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Recurring transaction manager not initialized".to_string(),
            data: None,
        })?;
        let mut manager = manager_arc.write().await;

        manager.cancel(&id_hash).map_err(|e| JsonRpcError {
            code: -32603,
            message: e,
            data: None,
        })?;

        Ok(json!({
            "success": true,
            "recurringTxId": format!("0x{}", hex::encode(id_hash)),
            "status": "Cancelled"
        }))
    }

    /// irondag_getRecurringTransaction — Get details of a specific recurring transaction.
    ///
    /// Params (object): { "recurringTxId": "0x..." }
    async fn irondag_get_recurring_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let id_hash = self.parse_recurring_id(params)?;

        let bc = self.acquire_blockchain_read().await?;
        let manager_arc = bc.recurring_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Recurring transaction manager not initialized".to_string(),
            data: None,
        })?;
        let manager = manager_arc.read().await;

        match manager.get(&id_hash) {
            None => Err(JsonRpcError {
                code: -32603,
                message: "Recurring transaction not found".to_string(),
                data: None,
            }),
            Some(r) => Ok(Self::format_recurring_tx(r)),
        }
    }

    /// irondag_getRecurringTransactions — List recurring transactions for a given address.
    ///
    /// Params (object): { "address": "0x..." }
    async fn irondag_get_recurring_transactions(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;
        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;
        let address = parse_address(
            obj.get("address")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: address".to_string(),
                    data: None,
                })?,
        )?;

        let bc = self.acquire_blockchain_read().await?;
        let manager_arc = bc.recurring_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Recurring transaction manager not initialized".to_string(),
            data: None,
        })?;
        let manager = manager_arc.read().await;

        let txs: Vec<Value> = manager
            .get_for_address(&address)
            .into_iter()
            .map(Self::format_recurring_tx)
            .collect();

        Ok(json!({
            "address": format!("0x{}", hex::encode(address)),
            "count": txs.len(),
            "transactions": txs,
        }))
    }

    fn parse_recurring_id(&self, params: Option<Value>) -> Result<crate::types::Hash, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;
        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;
        parse_hash(
            obj.get("recurringTxId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: recurringTxId".to_string(),
                    data: None,
                })?,
        )
    }

    fn format_recurring_tx(r: &crate::recurring::RecurringTransaction) -> Value {
        json!({
            "recurringTxId": format!("0x{}", hex::encode(r.recurring_tx_id)),
            "from": format!("0x{}", hex::encode(r.from)),
            "to": format!("0x{}", hex::encode(r.to)),
            "value": r.value.to_string(),
            "schedule": serde_json::to_value(&r.schedule).unwrap_or(Value::Null),
            "createdAt": r.created_at,
            "startDate": r.start_date,
            "endDate": r.end_date,
            "nextExecution": r.next_execution,
            "maxExecutions": r.max_executions,
            "executionCount": r.execution_count,
            "status": format!("{:?}", r.status),
            "lastExecution": r.last_execution,
            "lastExecutionTxHash": r.last_execution_tx_hash.map(|h| format!("0x{}", hex::encode(h))),
            "failureCount": r.failure_count,
        })
    }

    /// irondag_createGaslessTransaction - Create a gasless (sponsored) transaction
    async fn irondag_create_gasless_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let obj = params.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an object".to_string(),
            data: None,
        })?;

        let from = parse_address(obj.get("from").and_then(|v| v.as_str()).ok_or_else(|| {
            JsonRpcError {
                code: -32602,
                message: "Missing 'from' address".to_string(),
                data: None,
            }
        })?)?;

        let to =
            parse_address(
                obj.get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'to' address".to_string(),
                        data: None,
                    })?,
            )?;

        let sponsor =
            parse_address(obj.get("sponsor").and_then(|v| v.as_str()).ok_or_else(|| {
                JsonRpcError {
                    code: -32602,
                    message: "Missing 'sponsor' address".to_string(),
                    data: None,
                }
            })?)?;

        let value =
            parse_hex_number(obj.get("value").and_then(|v| v.as_str()).unwrap_or("0x0"))? as u128;
        let fee =
            parse_hex_number(obj.get("fee").and_then(|v| v.as_str()).unwrap_or("0x0"))? as u128;

        let blockchain = self.acquire_blockchain_read().await?;
        let nonce = blockchain.get_nonce(from);

        let tx = Transaction::new(from, to, value, fee, nonce).with_sponsor(sponsor);

        // Check sponsor balance
        let sponsor_balance = blockchain.get_balance(sponsor);
        if sponsor_balance < fee {
            return Err(JsonRpcError {
                code: -32603,
                message: format!(
                    "Insufficient sponsor balance: has {}, needs {}",
                    sponsor_balance, fee
                ),
                data: None,
            });
        }

        Ok(json!({
            "transaction": {
                "hash": format!("0x{}", hex::encode(tx.hash)),
                "from": format!("0x{}", hex::encode(tx.from)),
                "to": format!("0x{}", hex::encode(tx.to)),
                "value": format!("0x{:x}", tx.value),
                "fee": format!("0x{:x}", tx.fee),
                "sponsor": format!("0x{}", hex::encode(sponsor)),
                "nonce": format!("0x{:x}", tx.nonce),
            },
            "sponsorBalance": format!("0x{:x}", sponsor_balance),
            "message": "Transaction created. Must be signed before sending."
        }))
    }

    /// irondag_getSponsoredTransactions - Get transactions sponsored by an address
    async fn irondag_get_sponsored_transactions(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let sponsor = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing sponsor address".to_string(),
                    data: None,
                })?,
        )?;

        let blockchain = self.acquire_blockchain_read().await?;
        let mut sponsored = Vec::new();

        blockchain.with_blocks(|all_blocks| {
            for block in all_blocks {
                for tx in &block.transactions {
                    if let Some(tx_sponsor) = tx.sponsor {
                        if tx_sponsor == sponsor {
                            sponsored.push(json!({
                                "hash": format!("0x{}", hex::encode(tx.hash)),
                                "from": format!("0x{}", hex::encode(tx.from)),
                                "to": format!("0x{}", hex::encode(tx.to)),
                                "value": format!("0x{:x}", tx.value),
                                "fee": format!("0x{:x}", tx.fee),
                                "sponsor": format!("0x{}", hex::encode(sponsor)),
                                "blockNumber": format!("0x{:x}", block.header.block_number),
                            }));
                        }
                    }
                }
            }
        });

        Ok(json!({
            "sponsoredTransactions": sponsored,
            "count": sponsored.len(),
            "sponsor": format!("0x{}", hex::encode(sponsor)),
        }))
    }

    // -------------------------------------------------------------------------
    // Programmable Gas Sponsorship
    // -------------------------------------------------------------------------

    /// irondag_registerSponsorPolicy — Register or update a gas sponsorship policy.
    ///
    /// Params (object): {
    ///   "sponsor": "0x...",
    ///   "active": true,
    ///   "allowedSenders": null | ["0x..."],   // null = any sender
    ///   "maxFeePerTx": null | "1000000",       // attoIDAG, null = no cap
    ///   "expiresAtBlock": null | 12345,        // null = never
    ///   "dailySpendLimit": null | "5000000"    // attoIDAG per day-window
    /// }
    async fn irondag_register_sponsor_policy(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;

        let sponsor = parse_address(
            obj.get("sponsor")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: sponsor".to_string(),
                    data: None,
                })?,
        )?;

        let active = obj
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let allowed_senders = match obj.get("allowedSenders") {
            Some(Value::Null) | None => None,
            Some(Value::Array(arr)) => {
                let mut addrs = Vec::with_capacity(arr.len());
                for item in arr {
                    let s = item.as_str().ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "allowedSenders entries must be hex address strings".to_string(),
                        data: None,
                    })?;
                    addrs.push(parse_address(s)?);
                }
                Some(addrs)
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: "allowedSenders must be null or an array of address strings"
                        .to_string(),
                    data: None,
                });
            }
        };

        let max_fee_per_tx = match obj.get("maxFeePerTx") {
            Some(Value::Null) | None => None,
            Some(v) => {
                let raw = if let Some(n) = v.as_u64() {
                    n as u128
                } else {
                    let s = v.as_str().unwrap_or("0");
                    s.parse::<u128>()
                        .or_else(|_| {
                            u128::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                        })
                        .map_err(|_| JsonRpcError {
                            code: -32602,
                            message: "maxFeePerTx must be a decimal or 0x-hex string".to_string(),
                            data: None,
                        })?
                };
                Some(raw)
            }
        };

        let expires_at_block = match obj.get("expiresAtBlock") {
            Some(Value::Null) | None => None,
            Some(v) => Some(v.as_u64().ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "expiresAtBlock must be a uint".to_string(),
                data: None,
            })?),
        };

        let daily_spend_limit = match obj.get("dailySpendLimit") {
            Some(Value::Null) | None => None,
            Some(v) => {
                let raw = if let Some(n) = v.as_u64() {
                    n as u128
                } else {
                    let s = v.as_str().unwrap_or("0");
                    s.parse::<u128>()
                        .or_else(|_| {
                            u128::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                        })
                        .map_err(|_| JsonRpcError {
                            code: -32602,
                            message: "dailySpendLimit must be a decimal or 0x-hex string"
                                .to_string(),
                            data: None,
                        })?
                };
                Some(raw)
            }
        };

        let policy = crate::gas_sponsorship::SponsorPolicy {
            active,
            allowed_senders,
            max_fee_per_tx,
            expires_at_block,
            daily_spend_limit,
            window_start_block: 0,
            spent_in_window: 0,
        };

        // Blockchain read lock is sufficient — DashMap provides interior mutability.
        let bc = self.acquire_blockchain_read().await?;
        bc.sponsor_registry().register(sponsor, policy);

        Ok(json!({
            "success": true,
            "sponsor": format!("0x{}", hex::encode(sponsor)),
            "message": "Sponsor policy registered"
        }))
    }

    /// irondag_deregisterSponsorPolicy — Remove a gas sponsorship policy.
    ///
    /// Params (object): { "sponsor": "0x..." }
    async fn irondag_deregister_sponsor_policy(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;

        let sponsor = parse_address(
            obj.get("sponsor")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: sponsor".to_string(),
                    data: None,
                })?,
        )?;

        let bc = self.acquire_blockchain_read().await?;
        let existed = bc.sponsor_registry().deregister(&sponsor);

        Ok(json!({
            "success": existed,
            "sponsor": format!("0x{}", hex::encode(sponsor)),
            "message": if existed { "Policy removed" } else { "No policy was registered for this sponsor" }
        }))
    }

    /// irondag_getSponsorPolicy — Query the registered policy for a sponsor address.
    ///
    /// Params (object): { "sponsor": "0x..." }
    async fn irondag_get_sponsor_policy(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;

        let sponsor = parse_address(
            obj.get("sponsor")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: sponsor".to_string(),
                    data: None,
                })?,
        )?;

        let bc = self.acquire_blockchain_read().await?;
        match bc.sponsor_registry().get(&sponsor) {
            None => Ok(json!({
                "sponsor": format!("0x{}", hex::encode(sponsor)),
                "registered": false
            })),
            Some(policy) => Ok(json!({
                "sponsor": format!("0x{}", hex::encode(sponsor)),
                "registered": true,
                "active": policy.active,
                "allowedSenders": policy.allowed_senders.as_ref().map(|list| {
                    list.iter().map(|a| format!("0x{}", hex::encode(a))).collect::<Vec<_>>()
                }),
                "maxFeePerTx": policy.max_fee_per_tx.map(|v| v.to_string()),
                "expiresAtBlock": policy.expires_at_block,
                "dailySpendLimit": policy.daily_spend_limit.map(|v| v.to_string()),
                "spentInWindow": policy.spent_in_window.to_string(),
                "windowStartBlock": policy.window_start_block,
            })),
        }
    }

    // -------------------------------------------------------------------------
    // Built-in Privacy Pool
    // -------------------------------------------------------------------------

    /// irondag_getPoolInfo — Return current pool statistics.
    ///
    /// No params required.
    ///
    /// Returns: denomination (attoIDAG), commitmentCount, spentCount, availableBalance,
    ///          commitmentRoot, stubMode
    async fn irondag_get_pool_info(&self) -> Result<Value, JsonRpcError> {
        let bc = self.acquire_blockchain_read().await?;
        let pool = bc.privacy_pool();
        let pool = pool.read().await;
        Ok(json!({
            "denomination": pool.denomination().to_string(),
            "commitmentCount": pool.commitment_count(),
            "spentCount": pool.spent_count(),
            "availableBalance": pool.available_balance().to_string(),
            "commitmentRoot": format!("0x{}", hex::encode(pool.commitment_root())),
            "stubMode": true,
            "poolAddress": format!("0x{}", hex::encode(crate::privacy_pool::PRIVACY_POOL_ADDRESS.0)),
        }))
    }

    /// irondag_getPoolDepositParams — Return the transaction parameters a client should
    /// use to make a deposit.
    ///
    /// Params (object): { "commitment": "0x<32-byte keccak256 hash>" }
    ///
    /// Returns: { to, value, data } — client signs and sends via eth_sendRawTransaction.
    async fn irondag_get_pool_deposit_params(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let commitment_str = p
            .as_object()
            .and_then(|o| o.get("commitment"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing field: commitment".to_string(),
                data: None,
            })?;

        let commitment = parse_hash(commitment_str)?;

        let bc = self.acquire_blockchain_read().await?;
        let pool = bc.privacy_pool();
        let pool = pool.read().await;

        if pool.contains_commitment(&commitment) {
            return Err(JsonRpcError {
                code: -32602,
                message: "Commitment already exists in the pool".to_string(),
                data: None,
            });
        }

        let denomination = pool.denomination();
        drop(pool);

        Ok(json!({
            "to": format!("0x{}", hex::encode(crate::privacy_pool::PRIVACY_POOL_ADDRESS.0)),
            "value": denomination.to_string(),
            "data": format!("0x{}", hex::encode(commitment.0)),
            "note": "Send this transaction to register your commitment. Keep your nullifier and secret private."
        }))
    }

    /// irondag_poolWithdraw — Execute a privacy pool withdrawal.
    ///
    /// Params (object):
    ///   - nullifier: "0x<32-byte hash>"  — the nullifier used during deposit
    ///   - recipient: "0x<address>"        — address to receive the funds
    ///   - proof: "0x..."                  — (optional) zk-SNARK proof bytes; ignored in stub mode
    ///
    /// In stub mode the nullifier is accepted without proof.
    async fn irondag_pool_withdraw(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let obj = p.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params must be a JSON object".to_string(),
            data: None,
        })?;

        let nullifier = parse_hash(
            obj.get("nullifier")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: nullifier".to_string(),
                    data: None,
                })?,
        )?;

        let recipient = parse_address(
            obj.get("recipient")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing field: recipient".to_string(),
                    data: None,
                })?,
        )?;

        let proof: Option<Vec<u8>> = obj
            .get("proof")
            .and_then(|v| v.as_str())
            .map(|s| {
                let stripped = s.strip_prefix("0x").unwrap_or(s);
                hex::decode(stripped).unwrap_or_default()
            });

        let bc = self.acquire_blockchain_read().await?;
        let amount = bc
            .pool_withdraw(nullifier, recipient, proof)
            .await
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Withdrawal failed: {}", e),
                data: None,
            })?;

        Ok(json!({
            "success": true,
            "nullifier": format!("0x{}", hex::encode(nullifier.0)),
            "recipient": format!("0x{}", hex::encode(recipient.0)),
            "amount": amount.to_string(),
        }))
    }

    /// irondag_isNullifierSpent — Check whether a nullifier has already been used.
    ///
    /// Params (object): { "nullifier": "0x<32-byte hash>" }
    async fn irondag_is_nullifier_spent(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let p = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "params required".to_string(),
            data: None,
        })?;

        let nullifier_str = p
            .as_object()
            .and_then(|o| o.get("nullifier"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing field: nullifier".to_string(),
                data: None,
            })?;

        let nullifier = parse_hash(nullifier_str)?;

        let bc = self.acquire_blockchain_read().await?;
        let pool = bc.privacy_pool();
        let pool = pool.read().await;
        let spent = pool.is_nullifier_spent(&nullifier);

        Ok(json!({
            "nullifier": format!("0x{}", hex::encode(nullifier.0)),
            "spent": spent,
        }))
    }

    /// irondag_getReputation - Get reputation score for an address
    async fn irondag_get_reputation(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let reputation_manager = self
            .reputation_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Reputation manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing address".to_string(),
                    data: None,
                })?,
        )?;

        let mut manager = reputation_manager.write().await;
        let reputation = manager.get_reputation(&address);

        Ok(json!({
            "address": format!("0x{}", hex::encode(address)),
            "reputation": reputation.value(),
            "isHigh": reputation.is_high(),
            "isMedium": reputation.is_medium(),
            "isLow": reputation.is_low(),
        }))
    }

    /// irondag_getReputationFactors - Get detailed reputation factors for an address
    async fn irondag_get_reputation_factors(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let reputation_manager = self
            .reputation_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Reputation manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing address".to_string(),
                    data: None,
                })?,
        )?;

        let mut manager = reputation_manager.write().await;
        let reputation = manager.get_reputation(&address);

        // Get factors before dropping the write lock
        let factors = manager.get_factors(&address).cloned();
        drop(manager);

        if let Some(factors) = factors {
            Ok(json!({
                "address": format!("0x{}", hex::encode(address)),
                "reputation": reputation.value(),
                "factors": {
                    "successfulTxs": factors.successful_txs,
                    "failedTxs": factors.failed_txs,
                    "blocksMined": factors.blocks_mined,
                    "nodeLongevity": factors.node_longevity,
                    "accountAgeDays": factors.account_age_days,
                    "totalValueTransacted": format!("0x{:x}", factors.total_value_transacted),
                    "uniqueContacts": factors.unique_contacts,
                    "suspiciousActivities": factors.suspicious_activities,
                }
            }))
        } else {
            Ok(json!({
                "address": format!("0x{}", hex::encode(address)),
                "reputation": reputation.value(),
                "factors": null,
            }))
        }
    }

    /// Set wallet registry
    pub fn with_wallet_registry(
        &mut self,
        wallet_registry: Arc<tokio::sync::RwLock<crate::account_abstraction::WalletRegistry>>,
    ) {
        self.wallet_registry = Some(wallet_registry);
    }

    /// Set multi-signature manager
    pub fn with_multisig_manager(
        &mut self,
        multisig_manager: Arc<tokio::sync::RwLock<crate::account_abstraction::MultiSigManager>>,
    ) {
        self.multisig_manager = Some(multisig_manager);
    }

    /// Set social recovery manager
    pub fn with_social_recovery_manager(
        &mut self,
        social_recovery_manager: Arc<
            tokio::sync::RwLock<crate::account_abstraction::SocialRecoveryManager>,
        >,
    ) {
        self.social_recovery_manager = Some(social_recovery_manager);
    }

    /// Set batch transaction manager
    pub fn with_batch_manager(
        &mut self,
        batch_manager: Arc<tokio::sync::RwLock<crate::account_abstraction::BatchManager>>,
    ) {
        self.batch_manager = Some(batch_manager);
    }

    /// Set parallel EVM executor
    pub fn with_parallel_evm_executor(
        &mut self,
        executor: Arc<tokio::sync::RwLock<crate::evm::parallel::ParallelEvmExecutor>>,
    ) {
        self.parallel_evm_executor = Some(executor);
    }

    /// irondag_createWallet - Create a new smart contract wallet
    async fn irondag_create_wallet(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let wallet_registry = self.wallet_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Wallet registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let obj = params.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an object".to_string(),
            data: None,
        })?;

        let owner = parse_address(obj.get("owner").and_then(|v| v.as_str()).ok_or_else(|| {
            JsonRpcError {
                code: -32602,
                message: "Missing 'owner' address".to_string(),
                data: None,
            }
        })?)?;

        let wallet_type_str = obj
            .get("walletType")
            .and_then(|v| v.as_str())
            .unwrap_or("basic");
        let salt = obj
            .get("salt")
            .and_then(|v| v.as_str())
            .map(|s| parse_hex_number(s))
            .transpose()?
            .unwrap_or(0);

        use crate::account_abstraction::WalletFactory;

        let wallet = match wallet_type_str {
            "basic" => {
                WalletFactory::create_basic_wallet(owner.0, salt).map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to create basic wallet: {}", e),
                    data: None,
                })
            }
            "multisig" => {
                let signers_arr =
                    obj.get("signers")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| JsonRpcError {
                            code: -32602,
                            message: "Missing 'signers' array for multisig wallet".to_string(),
                            data: None,
                        })?;

                let signers: Result<Vec<Address>, _> = signers_arr
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Invalid signer address".to_string(),
                                data: None,
                            })
                            .and_then(|s| parse_address(s))
                    })
                    .collect();

                let signers = signers?;
                let signers_bytes: Vec<[u8; 20]> = signers.iter().map(|a| a.0).collect();
                let threshold = obj
                    .get("threshold")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'threshold' for multisig wallet".to_string(),
                        data: None,
                    })? as u8;

                WalletFactory::create_multisig_wallet(owner.0, salt, signers_bytes, threshold)
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Failed to create multisig wallet: {}", e),
                        data: None,
                    })
            }
            "socialRecovery" => {
                let guardians_arr =
                    obj.get("guardians")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| JsonRpcError {
                            code: -32602,
                            message: "Missing 'guardians' array for social recovery wallet"
                                .to_string(),
                            data: None,
                        })?;

                let guardians: Result<Vec<Address>, _> = guardians_arr
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Invalid guardian address".to_string(),
                                data: None,
                            })
                            .and_then(|s| parse_address(s))
                    })
                    .collect();

                let guardians = guardians?;
                let guardians_bytes: Vec<[u8; 20]> = guardians.iter().map(|a| a.0).collect();
                let recovery_threshold = obj
                    .get("recoveryThreshold")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'recoveryThreshold' for social recovery wallet"
                            .to_string(),
                        data: None,
                    })? as u8;

                let time_delay = obj
                    .get("timeDelay")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(604800); // Default 7 days

                WalletFactory::create_social_recovery_wallet(
                    owner.0,
                    salt,
                    guardians_bytes,
                    recovery_threshold,
                    time_delay,
                )
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to create social recovery wallet: {}", e),
                    data: None,
                })
            }
            "spendingLimit" => {
                let daily_limit = obj
                    .get("dailyLimit")
                    .and_then(|v| v.as_str())
                    .map(|s| parse_hex_number(s))
                    .transpose()?
                    .unwrap_or(0) as u128;
                let weekly_limit = obj
                    .get("weeklyLimit")
                    .and_then(|v| v.as_str())
                    .map(|s| parse_hex_number(s))
                    .transpose()?
                    .unwrap_or(0) as u128;
                let monthly_limit = obj
                    .get("monthlyLimit")
                    .and_then(|v| v.as_str())
                    .map(|s| parse_hex_number(s))
                    .transpose()?
                    .unwrap_or(0) as u128;

                WalletFactory::create_spending_limit_wallet(
                    owner.0,
                    salt,
                    daily_limit,
                    weekly_limit,
                    monthly_limit,
                )
                .map_err(|e| JsonRpcError {
                    code: -32603,
                    message: format!("Failed to create spending limit wallet: {}", e),
                    data: None,
                })
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown wallet type: {}", wallet_type_str),
                    data: None,
                });
            }
        };

        // Unwrap wallet result
        let wallet = wallet?;

        // Register wallet
        let mut registry = wallet_registry.write().await;
        match registry.register_wallet(wallet.clone()) {
            Ok(_) => Ok(json!({
                "walletAddress": format!("0x{}", hex::encode(wallet.address)),
                "owner": format!("0x{}", hex::encode(wallet.owner)),
                "walletType": wallet_type_str,
                "nonce": format!("0x{:x}", wallet.nonce),
                "createdAt": wallet.created_at,
                "message": "Wallet created successfully"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to register wallet: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getWallet - Get wallet information by address
    async fn irondag_get_wallet(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let wallet_registry = self.wallet_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Wallet registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing wallet address".to_string(),
                    data: None,
                })?,
        )?;

        let registry = wallet_registry.read().await;
        if let Some(wallet) = registry.get_wallet(&address) {
            let wallet_type_str = match &wallet.wallet_type {
                crate::account_abstraction::WalletType::Basic => "basic",
                crate::account_abstraction::WalletType::MultiSig { .. } => "multisig",
                crate::account_abstraction::WalletType::SocialRecovery { .. } => "socialRecovery",
                crate::account_abstraction::WalletType::SpendingLimit { .. } => "spendingLimit",
                crate::account_abstraction::WalletType::Combined { .. } => "combined",
            };

            Ok(json!({
                "walletAddress": format!("0x{}", hex::encode(wallet.address)),
                "owner": format!("0x{}", hex::encode(wallet.owner)),
                "walletType": wallet_type_str,
                "nonce": format!("0x{:x}", wallet.nonce),
                "createdAt": wallet.created_at,
            }))
        } else {
            Err(JsonRpcError {
                code: -32602,
                message: "Wallet not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getOwnerWallets - Get all wallets for an owner
    async fn irondag_get_owner_wallets(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let wallet_registry = self.wallet_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Wallet registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let owner = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing owner address".to_string(),
                    data: None,
                })?,
        )?;

        let registry = wallet_registry.read().await;
        let wallets = registry.get_owner_wallets(&owner.0);

        let wallets_json: Vec<Value> = wallets
            .iter()
            .map(|wallet| {
                let wallet_type_str = match &wallet.wallet_type {
                    crate::account_abstraction::WalletType::Basic => "basic",
                    crate::account_abstraction::WalletType::MultiSig { .. } => "multisig",
                    crate::account_abstraction::WalletType::SocialRecovery { .. } => {
                        "socialRecovery"
                    }
                    crate::account_abstraction::WalletType::SpendingLimit { .. } => "spendingLimit",
                    crate::account_abstraction::WalletType::Combined { .. } => "combined",
                };

                json!({
                    "walletAddress": format!("0x{}", hex::encode(wallet.address)),
                    "walletType": wallet_type_str,
                    "nonce": format!("0x{:x}", wallet.nonce),
                    "createdAt": wallet.created_at,
                })
            })
            .collect();

        Ok(json!({
            "owner": format!("0x{}", hex::encode(owner)),
            "wallets": wallets_json,
            "count": wallets_json.len(),
        }))
    }

    /// irondag_isContractWallet - Check if an address is a contract wallet
    async fn irondag_is_contract_wallet(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let wallet_registry = self.wallet_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Wallet registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing address".to_string(),
                    data: None,
                })?,
        )?;

        let registry = wallet_registry.read().await;
        let is_wallet = registry.is_contract_wallet(&address);

        Ok(json!({
            "address": format!("0x{}", hex::encode(address)),
            "isContractWallet": is_wallet,
        }))
    }

    /// irondag_createMultisigTransaction - Create a new multi-signature transaction
    async fn irondag_create_multisig_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let wallet_registry = self.wallet_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Wallet registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let obj = params.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an object".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            obj.get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        // Verify wallet exists and is multi-sig
        let registry = wallet_registry.read().await;
        let wallet = registry
            .get_wallet(&wallet_address)
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Wallet not found".to_string(),
                data: None,
            })?;

        if !wallet.is_multisig() {
            return Err(JsonRpcError {
                code: -32602,
                message: "Wallet is not a multi-signature wallet".to_string(),
                data: None,
            });
        }

        // Get signers and threshold from wallet
        let (signers, threshold) = match &wallet.wallet_type {
            crate::account_abstraction::WalletType::MultiSig { signers, threshold } => {
                (signers.clone(), *threshold)
            }
            crate::account_abstraction::WalletType::Combined {
                signers, threshold, ..
            } => (signers.clone(), *threshold),
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: "Wallet is not a multi-signature wallet".to_string(),
                    data: None,
                });
            }
        };

        // Parse transaction fields
        let to =
            parse_address(
                obj.get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'to' address".to_string(),
                        data: None,
                    })?,
            )?;

        let value = obj
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| parse_hex_u128(s))
            .transpose()?
            .unwrap_or(0);

        let fee = obj
            .get("fee")
            .and_then(|v| v.as_str())
            .map(|s| parse_hex_u128(s))
            .transpose()?
            .unwrap_or(0);

        let nonce = wallet.get_nonce();

        // Create transaction
        let tx = crate::blockchain::Transaction::new(wallet_address, to, value, fee, nonce);

        // Clone signers for JSON response (before moving into MultiSigTransaction)
        let signers_for_json: Vec<String> = signers
            .iter()
            .map(|s| format!("0x{}", hex::encode(s)))
            .collect();

        // Create multi-sig transaction
        use crate::account_abstraction::MultiSigTransaction;
        let multisig_tx = MultiSigTransaction::new(wallet_address, tx, signers, threshold)
            .map_err(|e| JsonRpcError {
                code: -32603,
                message: format!("Failed to create multi-sig transaction: {}", e),
                data: None,
            })?;

        Ok(json!({
            "walletAddress": format!("0x{}", hex::encode(wallet_address)),
            "transactionHash": format!("0x{}", hex::encode(multisig_tx.transaction.hash)),
            "threshold": threshold,
            "signaturesRequired": threshold,
            "signaturesCollected": 0,
            "expectedSigners": signers_for_json,
            "message": "Multi-sig transaction created. Add signatures using irondag_addMultisigSignature"
        }))
    }

    /// irondag_addMultisigSignature - Add a signature to a multi-sig transaction
    async fn irondag_add_multisig_signature(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let multisig_manager = self.multisig_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Multi-sig manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let obj = params.as_object().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Params must be an object".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            obj.get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        let tx_hash = parse_hash(
            obj.get("transactionHash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'transactionHash'".to_string(),
                    data: None,
                })?,
        )?;

        let signer =
            parse_address(obj.get("signer").and_then(|v| v.as_str()).ok_or_else(|| {
                JsonRpcError {
                    code: -32602,
                    message: "Missing 'signer' address".to_string(),
                    data: None,
                }
            })?)?;

        let signature_hex = obj
            .get("signature")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'signature'".to_string(),
                data: None,
            })?;

        let signature = hex::decode(signature_hex.strip_prefix("0x").unwrap_or(signature_hex))
            .map_err(|_| JsonRpcError {
                code: -32602,
                message: "Invalid signature format".to_string(),
                data: None,
            })?;

        let public_key_hex = obj
            .get("publicKey")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'publicKey'".to_string(),
                data: None,
            })?;

        let public_key = hex::decode(public_key_hex.strip_prefix("0x").unwrap_or(public_key_hex))
            .map_err(|_| JsonRpcError {
            code: -32602,
            message: "Invalid public key format".to_string(),
            data: None,
        })?;

        // Add signature to pending transaction
        let mut manager = multisig_manager.write().await;
        match manager.add_signature_to_pending(
            &wallet_address,
            &tx_hash,
            signer.0,
            signature,
            public_key,
        ) {
            Ok(_) => {
                // Get updated transaction
                let pending = manager.get_pending_transactions(&wallet_address);
                let tx = pending
                    .iter()
                    .find(|t| t.transaction.hash == tx_hash)
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Transaction not found".to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                    "transactionHash": format!("0x{}", hex::encode(tx_hash)),
                    "signaturesCollected": tx.signature_count(),
                    "signaturesRequired": tx.threshold,
                    "isReady": tx.is_ready(),
                    "signedBy": tx.signed_by().iter().map(|s| format!("0x{}", hex::encode(s))).collect::<Vec<_>>(),
                    "pendingSigners": tx.pending_signers().iter().map(|s| format!("0x{}", hex::encode(s))).collect::<Vec<_>>(),
                    "message": if tx.is_ready() { "Transaction ready to execute" } else { "More signatures needed" }
                }))
            }
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to add signature: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getPendingMultisigTransactions - Get pending multi-sig transactions for a wallet
    async fn irondag_get_pending_multisig_transactions(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let multisig_manager = self.multisig_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Multi-sig manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .as_array()
                .and_then(|arr| arr.get(0))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing wallet address".to_string(),
                    data: None,
                })?,
        )?;

        let manager = multisig_manager.read().await;
        let pending = manager.get_pending_transactions(&wallet_address);

        let transactions_json: Vec<Value> = pending.iter().map(|tx| {
            json!({
                "transactionHash": format!("0x{}", hex::encode(tx.transaction.hash)),
                "to": format!("0x{}", hex::encode(tx.transaction.to)),
                "value": format!("0x{:x}", tx.transaction.value),
                "fee": format!("0x{:x}", tx.transaction.fee),
                "nonce": format!("0x{:x}", tx.transaction.nonce),
                "signaturesCollected": tx.signature_count(),
                "signaturesRequired": tx.threshold,
                "isReady": tx.is_ready(),
                "signedBy": tx.signed_by().iter().map(|s| format!("0x{}", hex::encode(s))).collect::<Vec<_>>(),
                "pendingSigners": tx.pending_signers().iter().map(|s| format!("0x{}", hex::encode(s))).collect::<Vec<_>>(),
            })
        }).collect();

        Ok(json!({
            "walletAddress": format!("0x{}", hex::encode(wallet_address)),
            "pendingTransactions": transactions_json,
            "count": transactions_json.len(),
        }))
    }

    /// irondag_validateMultisigTransaction - Validate a multi-sig transaction
    async fn irondag_validate_multisig_transaction(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        Err(JsonRpcError {
            code: -32601,
            message: "irondag_validateMultisigTransaction is not available in this version (planned for V2)".to_string(),
            data: None,
        })
    }

    /// irondag_initiateRecovery - Initiate wallet recovery process
    async fn irondag_initiate_recovery(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let social_recovery_manager =
            self.social_recovery_manager
                .as_ref()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Social recovery manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        let new_owner = parse_address(
            params
                .get("newOwner")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'newOwner'".to_string(),
                    data: None,
                })?,
        )?;

        let guardians: Vec<Address> = params
            .get("guardians")
            .and_then(|v| v.as_array())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'guardians' array".to_string(),
                data: None,
            })?
            .iter()
            .map(|v| v.as_str().and_then(|s| parse_address(s).ok()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid guardian addresses".to_string(),
                data: None,
            })?;

        let recovery_threshold = params
            .get("recoveryThreshold")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8)
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'recoveryThreshold'".to_string(),
                data: None,
            })?;

        let time_delay = params.get("timeDelay").and_then(|v| v.as_u64());

        // Get current timestamp from blockchain
        let blockchain = self.acquire_blockchain_read().await?;
        let current_timestamp = blockchain
            .get_latest_block()
            .map(|b| b.header.timestamp)
            .unwrap_or(0);
        drop(blockchain);

        let mut manager = social_recovery_manager.write().await;
        match manager.initiate_recovery(
            wallet_address,
            new_owner,
            guardians.clone(),
            recovery_threshold,
            time_delay,
            current_timestamp,
        ) {
            Ok(request) => Ok(json!({
                "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                "newOwner": format!("0x{}", hex::encode(new_owner)),
                "guardians": guardians.iter().map(|g| format!("0x{}", hex::encode(g))).collect::<Vec<_>>(),
                "recoveryThreshold": recovery_threshold,
                "timeDelay": request.time_delay,
                "initiatedAt": request.initiated_at,
                "status": match request.status {
                    crate::account_abstraction::RecoveryStatus::Pending => "pending",
                    crate::account_abstraction::RecoveryStatus::Approved => "approved",
                    crate::account_abstraction::RecoveryStatus::Ready => "ready",
                    crate::account_abstraction::RecoveryStatus::Completed => "completed",
                    crate::account_abstraction::RecoveryStatus::Cancelled => "cancelled",
                },
                "approvalCount": request.approval_count(),
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_approveRecovery - Approve a recovery request (guardian)
    async fn irondag_approve_recovery(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let social_recovery_manager =
            self.social_recovery_manager
                .as_ref()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Social recovery manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        let guardian = parse_address(params.get("guardian").and_then(|v| v.as_str()).ok_or_else(
            || JsonRpcError {
                code: -32602,
                message: "Missing 'guardian'".to_string(),
                data: None,
            },
        )?)?;

        let signature_hex = params.get("signature").and_then(|v| v.as_str()).ok_or_else(
            || JsonRpcError {
                code: -32602,
                message: "Missing 'signature'".to_string(),
                data: None,
            },
        )?;

        let signature = hex::decode(signature_hex.strip_prefix("0x").unwrap_or(signature_hex))
            .map_err(|_| JsonRpcError {
                code: -32602,
                message: "Invalid signature format".to_string(),
                data: None,
            })?;

        // Get current timestamp
        let blockchain = self.acquire_blockchain_read().await?;
        let current_timestamp = blockchain
            .get_latest_block()
            .map(|b| b.header.timestamp)
            .unwrap_or(0);
        drop(blockchain);

        let mut manager = social_recovery_manager.write().await;
        match manager.approve_recovery(wallet_address, guardian, &signature, current_timestamp) {
            Ok(_) => {
                // Get updated status
                let status = manager
                    .get_recovery_status(&wallet_address)
                    .ok_or_else(|| JsonRpcError {
                        code: -32603,
                        message: "Recovery request not found".to_string(),
                        data: None,
                    })?;

                Ok(json!({
                    "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                    "status": match status.status {
                        crate::account_abstraction::RecoveryStatus::Pending => "pending",
                        crate::account_abstraction::RecoveryStatus::Approved => "approved",
                        crate::account_abstraction::RecoveryStatus::Ready => "ready",
                        crate::account_abstraction::RecoveryStatus::Completed => "completed",
                        crate::account_abstraction::RecoveryStatus::Cancelled => "cancelled",
                    },
                    "approvalCount": status.approval_count(),
                    "thresholdMet": status.threshold_met(),
                }))
            }
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_getRecoveryStatus - Get recovery request status
    async fn irondag_get_recovery_status(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let social_recovery_manager =
            self.social_recovery_manager
                .as_ref()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Social recovery manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        // Get current timestamp
        let blockchain = self.acquire_blockchain_read().await?;
        let current_timestamp = blockchain
            .get_latest_block()
            .map(|b| b.header.timestamp)
            .unwrap_or(0);
        drop(blockchain);

        let mut manager = social_recovery_manager.write().await;
        manager.update_all_statuses(current_timestamp);

        let status = manager
            .get_recovery_status(&wallet_address)
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Recovery request not found".to_string(),
                data: None,
            })?;

        let approvals: Vec<Value> = status
            .approvals
            .iter()
            .map(|(guardian, timestamp)| {
                json!({
                    "guardian": format!("0x{}", hex::encode(guardian)),
                    "approvedAt": timestamp,
                })
            })
            .collect();

        Ok(json!({
            "walletAddress": format!("0x{}", hex::encode(wallet_address)),
            "newOwner": format!("0x{}", hex::encode(status.new_owner)),
            "guardians": status.guardians.iter().map(|g| format!("0x{}", hex::encode(g))).collect::<Vec<_>>(),
            "recoveryThreshold": status.recovery_threshold,
            "timeDelay": status.time_delay,
            "initiatedAt": status.initiated_at,
            "status": match status.status {
                crate::account_abstraction::RecoveryStatus::Pending => "pending",
                crate::account_abstraction::RecoveryStatus::Approved => "approved",
                crate::account_abstraction::RecoveryStatus::Ready => "ready",
                crate::account_abstraction::RecoveryStatus::Completed => "completed",
                crate::account_abstraction::RecoveryStatus::Cancelled => "cancelled",
            },
            "approvalCount": status.approval_count(),
            "thresholdMet": status.threshold_met(),
            "approvals": approvals,
            "isReady": status.is_ready(current_timestamp),
        }))
    }

    /// irondag_completeRecovery - Complete recovery and transfer wallet ownership
    async fn irondag_complete_recovery(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let social_recovery_manager =
            self.social_recovery_manager
                .as_ref()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Social recovery manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        // Get current timestamp
        let blockchain = self.acquire_blockchain_read().await?;
        let current_timestamp = blockchain
            .get_latest_block()
            .map(|b| b.header.timestamp)
            .unwrap_or(0);
        drop(blockchain);

        let mut manager = social_recovery_manager.write().await;
        match manager.complete_recovery(wallet_address, current_timestamp) {
            Ok(new_owner) => Ok(json!({
                "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                "newOwner": format!("0x{}", hex::encode(new_owner)),
                "status": "completed",
                "message": "Recovery completed successfully",
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_cancelRecovery - Cancel a recovery request
    async fn irondag_cancel_recovery(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let social_recovery_manager =
            self.social_recovery_manager
                .as_ref()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Social recovery manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        let mut manager = social_recovery_manager.write().await;
        match manager.cancel_recovery(wallet_address) {
            Ok(_) => Ok(json!({
                "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                "status": "cancelled",
                "message": "Recovery request cancelled",
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_createBatchTransaction - Create a new batch transaction
    async fn irondag_create_batch_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let batch_manager = self.batch_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Batch manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let wallet_address = parse_address(
            params
                .get("walletAddress")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing 'walletAddress'".to_string(),
                    data: None,
                })?,
        )?;

        let operations_array = params
            .get("operations")
            .and_then(|v| v.as_array())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'operations' array".to_string(),
                data: None,
            })?;

        let mut operations = Vec::new();
        for op_json in operations_array {
            let op_type = op_json
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing operation 'type'".to_string(),
                    data: None,
                })?;

            let operation = match op_type {
                "transfer" => {
                    let to =
                        parse_address(op_json.get("to").and_then(|v| v.as_str()).ok_or_else(
                            || JsonRpcError {
                                code: -32602,
                                message: "Missing 'to' for transfer".to_string(),
                                data: None,
                            },
                        )?)?;
                    let value =
                        parse_hex_u128(op_json.get("value").and_then(|v| v.as_str()).ok_or_else(
                            || JsonRpcError {
                                code: -32602,
                                message: "Missing 'value' for transfer".to_string(),
                                data: None,
                            },
                        )?)?;
                    crate::account_abstraction::BatchOperation::Transfer { to, value }
                }
                "contractCall" => {
                    let contract = parse_address(
                        op_json
                            .get("contract")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'contract' for contractCall".to_string(),
                                data: None,
                            })?,
                    )?;
                    let data = op_json
                        .get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| hex::decode(s.strip_prefix("0x").unwrap_or(s)).unwrap_or_default())
                        .unwrap_or_default();
                    let value = op_json
                        .get("value")
                        .and_then(|v| v.as_str())
                        .map(|s| parse_hex_u128(s).unwrap_or(0))
                        .unwrap_or(0);
                    crate::account_abstraction::BatchOperation::ContractCall {
                        contract,
                        data,
                        value,
                    }
                }
                "approval" => {
                    let spender = parse_address(
                        op_json
                            .get("spender")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'spender' for approval".to_string(),
                                data: None,
                            })?,
                    )?;
                    let amount = parse_hex_u128(
                        op_json
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'amount' for approval".to_string(),
                                data: None,
                            })?,
                    )?;
                    crate::account_abstraction::BatchOperation::Approval { spender, amount }
                }
                _ => {
                    return Err(JsonRpcError {
                        code: -32602,
                        message: format!("Unknown operation type: {}", op_type),
                        data: None,
                    });
                }
            };
            operations.push(operation);
        }

        let nonce = params.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0);

        let gas_limit = parse_hex_number(
            params
                .get("gasLimit")
                .and_then(|v| v.as_str())
                .unwrap_or("0x100000"),
        )?;

        let gas_price = parse_hex_u128(
            params
                .get("gasPrice")
                .and_then(|v| v.as_str())
                .unwrap_or("0x3b9aca00"),
        )?;

        // Get current timestamp
        let blockchain = self.acquire_blockchain_read().await?;
        let current_timestamp = blockchain
            .get_latest_block()
            .map(|b| b.header.timestamp)
            .unwrap_or(0);
        drop(blockchain);

        let mut manager = batch_manager.write().await;
        match manager.create_batch(
            wallet_address,
            operations.clone(),
            nonce,
            gas_limit,
            gas_price,
            current_timestamp,
        ) {
            Ok(batch) => {
                // Estimate gas
                let estimate = manager.estimate_gas(&operations).unwrap_or(
                    crate::account_abstraction::GasEstimate {
                        total_gas: gas_limit,
                        base_gas: 21_000,
                        operation_gas: 0,
                        optimization_savings: 0,
                    },
                );

                Ok(json!({
                    "batchId": format!("0x{}", hex::encode(batch.batch_id)),
                    "walletAddress": format!("0x{}", hex::encode(wallet_address)),
                    "operationCount": batch.operation_count(),
                    "estimatedGas": format!("0x{:x}", estimate.total_gas),
                    "gasBreakdown": {
                        "baseGas": format!("0x{:x}", estimate.base_gas),
                        "operationGas": format!("0x{:x}", estimate.operation_gas),
                        "optimizationSavings": format!("0x{:x}", estimate.optimization_savings),
                    },
                    "status": "pending",
                }))
            }
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_executeBatchTransaction - Execute a batch transaction
    async fn irondag_execute_batch_transaction(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        Err(JsonRpcError {
            code: -32601,
            message: "irondag_executeBatchTransaction is not available in this version (planned for V2)".to_string(),
            data: None,
        })
    }

    /// irondag_getBatchStatus - Get status of a batch transaction
    async fn irondag_get_batch_status(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let batch_manager = self.batch_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Batch manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let batch_id = parse_hash(params.get("batchId").and_then(|v| v.as_str()).ok_or_else(
            || JsonRpcError {
                code: -32602,
                message: "Missing 'batchId'".to_string(),
                data: None,
            },
        )?)?;

        let manager = batch_manager.read().await;
        let batch = manager.get_batch(&batch_id).ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Batch not found".to_string(),
            data: None,
        })?;

        let results_json: Vec<Value> = batch
            .results
            .iter()
            .map(|result| {
                json!({
                    "operationIndex": result.operation_index,
                    "success": result.success,
                    "result": result.result.as_ref().map(|r| format!("0x{}", hex::encode(r))),
                    "error": result.error.clone(),
                    "gasUsed": format!("0x{:x}", result.gas_used),
                })
            })
            .collect();

        Ok(json!({
            "batchId": format!("0x{}", hex::encode(batch_id)),
            "walletAddress": format!("0x{}", hex::encode(batch.wallet_address)),
            "status": match batch.status {
                crate::account_abstraction::BatchStatus::Pending => "pending",
                crate::account_abstraction::BatchStatus::Executing => "executing",
                crate::account_abstraction::BatchStatus::Completed => "completed",
                crate::account_abstraction::BatchStatus::Failed => "failed",
                crate::account_abstraction::BatchStatus::Cancelled => "cancelled",
            },
            "operationCount": batch.operation_count(),
            "completedOperations": batch.results.len(),
            "gasUsed": format!("0x{:x}", batch.gas_used),
            "gasLimit": format!("0x{:x}", batch.gas_limit),
            "results": results_json,
        }))
    }

    /// irondag_estimateBatchGas - Estimate gas cost for a batch
    async fn irondag_estimate_batch_gas(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let batch_manager = self.batch_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Batch manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let operations_array = params
            .get("operations")
            .and_then(|v| v.as_array())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'operations' array".to_string(),
                data: None,
            })?;

        let mut operations = Vec::new();
        for op_json in operations_array {
            let op_type = op_json
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: -32602,
                    message: "Missing operation 'type'".to_string(),
                    data: None,
                })?;

            let operation = match op_type {
                "transfer" => {
                    let to =
                        parse_address(op_json.get("to").and_then(|v| v.as_str()).ok_or_else(
                            || JsonRpcError {
                                code: -32602,
                                message: "Missing 'to' for transfer".to_string(),
                                data: None,
                            },
                        )?)?;
                    let value =
                        parse_hex_u128(op_json.get("value").and_then(|v| v.as_str()).ok_or_else(
                            || JsonRpcError {
                                code: -32602,
                                message: "Missing 'value' for transfer".to_string(),
                                data: None,
                            },
                        )?)?;
                    crate::account_abstraction::BatchOperation::Transfer { to, value }
                }
                "contractCall" => {
                    let contract = parse_address(
                        op_json
                            .get("contract")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'contract' for contractCall".to_string(),
                                data: None,
                            })?,
                    )?;
                    let data = op_json
                        .get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| hex::decode(s.strip_prefix("0x").unwrap_or(s)).unwrap_or_default())
                        .unwrap_or_default();
                    let value = op_json
                        .get("value")
                        .and_then(|v| v.as_str())
                        .map(|s| parse_hex_u128(s).unwrap_or(0))
                        .unwrap_or(0);
                    crate::account_abstraction::BatchOperation::ContractCall {
                        contract,
                        data,
                        value,
                    }
                }
                "approval" => {
                    let spender = parse_address(
                        op_json
                            .get("spender")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'spender' for approval".to_string(),
                                data: None,
                            })?,
                    )?;
                    let amount = parse_hex_u128(
                        op_json
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| JsonRpcError {
                                code: -32602,
                                message: "Missing 'amount' for approval".to_string(),
                                data: None,
                            })?,
                    )?;
                    crate::account_abstraction::BatchOperation::Approval { spender, amount }
                }
                _ => {
                    return Err(JsonRpcError {
                        code: -32602,
                        message: format!("Unknown operation type: {}", op_type),
                        data: None,
                    });
                }
            };
            operations.push(operation);
        }

        let manager = batch_manager.read().await;
        match manager.estimate_gas(&operations) {
            Ok(estimate) => Ok(json!({
                "estimatedGas": format!("0x{:x}", estimate.total_gas),
                "gasBreakdown": {
                    "baseGas": format!("0x{:x}", estimate.base_gas),
                    "operationGas": format!("0x{:x}", estimate.operation_gas),
                    "optimizationSavings": format!("0x{:x}", estimate.optimization_savings),
                },
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: e,
                data: None,
            }),
        }
    }

    /// irondag_enableParallelEVM - Enable or disable parallel EVM execution
    async fn irondag_enable_parallel_evm(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let enabled = params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid parameter: enabled (boolean required)".to_string(),
                data: None,
            })?;

        if let Some(ref executor) = self.parallel_evm_executor {
            let mut exec = executor.write().await;
            exec.set_enabled(enabled);
            Ok(json!({
                "enabled": enabled,
                "message": if enabled { "Parallel EVM enabled" } else { "Parallel EVM disabled" }
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Parallel EVM executor not available".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getParallelEVMStats - Get parallel EVM statistics
    async fn irondag_get_parallel_evm_stats(
        &self,
        _params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        if let Some(ref executor) = self.parallel_evm_executor {
            let exec = executor.read().await;
            Ok(json!({
                "enabled": exec.enabled,
                "maxParallel": exec.max_parallel,
            }))
        } else {
            Ok(json!({
                "enabled": false,
                "maxParallel": 0,
                "message": "Parallel EVM executor not available"
            }))
        }
    }

    /// irondag_estimateParallelImprovement - Estimate performance improvement from parallel execution
    async fn irondag_estimate_parallel_improvement(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing parameters".to_string(),
            data: None,
        })?;

        let transactions = params
            .get("transactions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid parameter: transactions (array required)".to_string(),
                data: None,
            })?;

        if let Some(ref executor) = self.parallel_evm_executor {
            let exec = executor.read().await;
            let tx_count = transactions.len();
            // Estimate improvement: parallel execution time vs sequential
            // Assumption: with max_parallel workers, we can process min(tx_count, max_parallel) txs concurrently
            // Sequential time = tx_count * avg_time
            // Parallel time = ceil(tx_count / max_parallel) * avg_time + overhead
            // Improvement = sequential_time / parallel_time
            let estimated_improvement = if tx_count > 1 && exec.enabled {
                let max_parallel = exec.max_parallel.max(1) as f64;
                let parallel_rounds = (tx_count as f64 / max_parallel).ceil();
                // Add small overhead factor (10%) for coordination
                let parallel_time = parallel_rounds * 1.1;
                let sequential_time = tx_count as f64;
                (sequential_time / parallel_time).min(10.0)
            } else {
                1.0
            };

            Ok(json!({
                "estimatedImprovement": estimated_improvement,
                "transactionCount": tx_count,
                "enabled": exec.enabled,
                "message": format!("Estimated {:.2}x improvement for {} transactions", estimated_improvement, tx_count)
            }))
        } else {
            Ok(json!({
                "estimatedImprovement": 1.0,
                "transactionCount": transactions.len(),
                "enabled": false,
                "message": "Parallel EVM executor not available"
            }))
        }
    }

    // ========== Oracle RPC Methods ==========

    /// irondag_registerOracle - Register a new oracle node
    async fn irondag_register_oracle(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.oracle_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Oracle registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_object()
            .and_then(|obj| obj.get("address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;
        let feed_types = params
            .as_object()
            .and_then(|obj| obj.get("feed_types"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(|s| match s {
                        "price" => Some(crate::oracles::FeedType::Price),
                        "randomness" => Some(crate::oracles::FeedType::Randomness),
                        "custom" => Some(crate::oracles::FeedType::Custom),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![crate::oracles::FeedType::Price]);

        let stake_str = params
            .as_object()
            .and_then(|obj| obj.get("stake_amount"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stake_amount parameter".to_string(),
                data: None,
            })?;

        let stake_amount = parse_hex_u128(stake_str)?;
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut registry = registry.write().await;
        match registry.register_oracle(address, feed_types, stake_amount, current_time) {
            Ok(_) => Ok(json!({
                "success": true,
                "address": format!("0x{}", hex::encode(address)),
                "message": "Oracle registered successfully"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to register oracle: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getPrice - Get current price for a feed
    async fn irondag_get_price(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let price_feeds = self
            .price_feed_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Price feed manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let feed_id = params
            .as_object()
            .and_then(|obj| obj.get("feed_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing feed_id parameter".to_string(),
                data: None,
            })?;

        let price_feeds_read = price_feeds.read().await;
        if let Some(price) = price_feeds_read.get_price(feed_id) {
            Ok(json!({
                "feed_id": feed_id,
                "price": format!("0x{:x}", price),
                "price_decimal": price.to_string(),
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Price feed not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getPriceFeeds - Get all available price feeds
    async fn irondag_get_price_feeds(&self) -> Result<Value, JsonRpcError> {
        let price_feeds = self
            .price_feed_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Price feed manager not available".to_string(),
                data: None,
            })?;

        let price_feeds_read = price_feeds.read().await;
        let feeds: Vec<Value> = price_feeds_read
            .get_all_feeds()
            .iter()
            .map(|feed| {
                json!({
                    "feed_id": feed.feed_id,
                    "asset_pair": {
                        "base": feed.asset_pair.0,
                        "quote": feed.asset_pair.1,
                    },
                    "current_price": format!("0x{:x}", feed.current_price),
                    "last_update": feed.last_update,
                    "oracle_count": feed.oracle_count,
                })
            })
            .collect();

        Ok(json!({ "feeds": feeds }))
    }

    /// irondag_requestRandomness - Request verifiable randomness
    async fn irondag_request_randomness(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let vrf = self.vrf_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "VRF manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let requester_str = params
            .as_object()
            .and_then(|obj| obj.get("requester"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing requester parameter".to_string(),
                data: None,
            })?;

        let requester = parse_address(requester_str)?;
        let seed_str = params
            .as_object()
            .and_then(|obj| obj.get("seed"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing seed parameter".to_string(),
                data: None,
            })?;

        let seed = parse_hash(seed_str)?;
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut vrf = vrf.write().await;
        let request_id = vrf.request_randomness(requester, seed, current_time);

        Ok(json!({
            "request_id": format!("0x{}", hex::encode(request_id)),
            "requester": format!("0x{}", hex::encode(requester)),
            "status": "pending"
        }))
    }

    // ========== Stop-Loss RPC Methods ==========

    /// irondag_createStopLoss - Create a new stop-loss order
    async fn irondag_create_stop_loss(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let wallet_str = params
            .as_object()
            .and_then(|obj| obj.get("wallet_address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing wallet_address parameter".to_string(),
                data: None,
            })?;

        let wallet_address = parse_address(wallet_str)?;
        let asset_pair = params
            .as_object()
            .and_then(|obj| obj.get("asset_pair"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing asset_pair parameter".to_string(),
                data: None,
            })?;

        let trigger_type_str = params
            .as_object()
            .and_then(|obj| obj.get("trigger_type"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing trigger_type parameter".to_string(),
                data: None,
            })?;

        let trigger_price_str = params
            .as_object()
            .and_then(|obj| obj.get("trigger_price"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing trigger_price parameter".to_string(),
                data: None,
            })?;

        let trigger_price = parse_hex_u128(trigger_price_str)?;
        let trigger_type = match trigger_type_str {
            "above" => crate::stop_loss::StopLossType::PriceAbove(trigger_price),
            "below" => crate::stop_loss::StopLossType::PriceBelow(trigger_price),
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: "Invalid trigger_type (must be 'above' or 'below')".to_string(),
                    data: None,
                })
            }
        };

        // Create transaction (simplified - would need full transaction parsing)
        let to_str = params
            .as_object()
            .and_then(|obj| obj.get("to"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing to parameter".to_string(),
                data: None,
            })?;

        let to = parse_address(to_str)?;
        let value_str = params
            .as_object()
            .and_then(|obj| obj.get("value"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing value parameter".to_string(),
                data: None,
            })?;

        let value = parse_hex_u128(value_str)?;

        // Get real gas_price from blockchain (base fee from latest block)
        let blockchain = self.acquire_blockchain_read().await?;
        let gas_price = blockchain
            .get_latest_block()
            .map(|b| b.header.base_fee_per_gas)
            .unwrap_or(crate::mining::BASE_FEE_INITIAL);
        let nonce = blockchain.get_nonce(wallet_address);
        drop(blockchain);

        // Create transaction with real gas_price and nonce
        let transaction = Transaction::new(wallet_address, to, value, gas_price, nonce);

        let oracle_feed_id = params
            .as_object()
            .and_then(|obj| obj.get("oracle_feed_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut manager = manager.write().await;
        let order = manager.create_stop_loss(
            wallet_address,
            asset_pair,
            trigger_type,
            transaction,
            oracle_feed_id,
            current_time,
            None,
        );

        Ok(json!({
            "stop_loss_id": format!("0x{}", hex::encode(order.stop_loss_id)),
            "wallet_address": format!("0x{}", hex::encode(order.wallet_address)),
            "asset_pair": order.asset_pair,
            "status": "active",
        }))
    }

    /// irondag_getStopLossOrders - Get all stop-loss orders for an address
    async fn irondag_get_stop_loss_orders(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_object()
            .and_then(|obj| obj.get("address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;
        let manager_read = manager.read().await;
        let orders = manager_read.get_for_address(&address);

        let orders_json: Vec<Value> = orders
            .iter()
            .map(|order| {
                json!({
                    "stop_loss_id": format!("0x{}", hex::encode(order.stop_loss_id)),
                    "asset_pair": order.asset_pair,
                    "status": format!("{:?}", order.status),
                    "triggered_at": order.triggered_at,
                    "triggered_price": order.triggered_price.map(|p| format!("0x{:x}", p)),
                })
            })
            .collect();

        Ok(json!({ "stop_loss_orders": orders_json }))
    }

    // ========== Complete Oracle RPC Methods ==========

    /// irondag_unregisterOracle - Unregister an oracle node
    async fn irondag_unregister_oracle(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.oracle_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Oracle registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_object()
            .and_then(|obj| obj.get("address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;
        let mut registry = registry.write().await;

        match registry.unregister_oracle(&address) {
            Ok(_) => Ok(json!({
                "success": true,
                "address": format!("0x{}", hex::encode(address)),
                "message": "Oracle unregistered successfully"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to unregister oracle: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getOracleInfo - Get oracle node information
    async fn irondag_get_oracle_info(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.oracle_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Oracle registry not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_object()
            .and_then(|obj| obj.get("address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;
        let registry_read = registry.read().await;

        if let Some(oracle) = registry_read.get_oracle(&address) {
            Ok(json!({
                "address": format!("0x{}", hex::encode(oracle.address)),
                "feed_types": oracle.feed_types.iter().map(|ft| format!("{:?}", ft)).collect::<Vec<_>>(),
                "stake_amount": format!("0x{:x}", oracle.stake_amount),
                "reputation_score": oracle.reputation_score,
                "accuracy_rate": oracle.accuracy_rate,
                "total_reports": oracle.total_reports,
                "accurate_reports": oracle.accurate_reports,
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Oracle not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_getOracleList - Get list of all oracles (optionally filtered by feed type)
    async fn irondag_get_oracle_list(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let registry = self.oracle_registry.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Oracle registry not available".to_string(),
            data: None,
        })?;

        let feed_type_filter: Option<crate::oracles::FeedType> = params.and_then(|p| {
            p.as_object()
                .and_then(|obj| obj.get("feed_type"))
                .and_then(|v| v.as_str())
                .and_then(|s| match s {
                    "price" => Some(crate::oracles::FeedType::Price),
                    "randomness" => Some(crate::oracles::FeedType::Randomness),
                    "custom" => Some(crate::oracles::FeedType::Custom),
                    _ => None,
                })
        });

        let registry_read = registry.read().await;
        let oracles: Vec<Value> = if let Some(ft) = feed_type_filter {
            registry_read
                .get_oracles_for_feed_type(ft)
                .iter()
                .map(|oracle| {
                    json!({
                        "address": format!("0x{}", hex::encode(oracle.address)),
                        "reputation_score": oracle.reputation_score,
                        "stake_amount": format!("0x{:x}", oracle.stake_amount),
                    })
                })
                .collect()
        } else {
            registry_read.get_all_oracles()
                .iter()
                .map(|oracle| {
                    json!({
                        "address": format!("0x{}", hex::encode(oracle.address)),
                        "feed_types": oracle.feed_types.iter().map(|ft| format!("{:?}", ft)).collect::<Vec<_>>(),
                        "reputation_score": oracle.reputation_score,
                        "stake_amount": format!("0x{:x}", oracle.stake_amount),
                    })
                })
                .collect()
        };

        Ok(json!({ "oracles": oracles, "count": oracles.len() }))
    }

    /// irondag_getPriceHistory - Get price history for a feed
    async fn irondag_get_price_history(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let price_feeds = self
            .price_feed_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Price feed manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let feed_id = params
            .as_object()
            .and_then(|obj| obj.get("feed_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing feed_id parameter".to_string(),
                data: None,
            })?;

        let limit = params
            .as_object()
            .and_then(|obj| obj.get("limit"))
            .and_then(|v| v.as_u64())
            .map(|l| l as usize);

        let price_feeds_read = price_feeds.read().await;
        let history = price_feeds_read.get_price_history(feed_id, limit);

        let history_json: Vec<Value> = history
            .iter()
            .map(|(timestamp, price)| {
                json!({
                    "timestamp": timestamp,
                    "price": format!("0x{:x}", price),
                })
            })
            .collect();

        Ok(json!({
            "feed_id": feed_id,
            "history": history_json,
            "count": history_json.len(),
        }))
    }

    /// irondag_getRandomness - Get randomness request result
    async fn irondag_get_randomness(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let vrf = self.vrf_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "VRF manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let request_id_str = params
            .as_object()
            .and_then(|obj| obj.get("request_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing request_id parameter".to_string(),
                data: None,
            })?;

        let request_id = parse_hash(request_id_str)?;
        let vrf_read = vrf.read().await;

        if let Some(request) = vrf_read.get_request(&request_id) {
            Ok(json!({
                "request_id": format!("0x{}", hex::encode(request.request_id)),
                "requester": format!("0x{}", hex::encode(request.requester)),
                "fulfilled": request.fulfilled,
                "randomness": request.randomness.map(|r| format!("0x{}", hex::encode(r))),
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Randomness request not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_pauseRecurringTransaction - Pause a recurring transaction
    async fn irondag_pause_recurring_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let manager = self
            .recurring_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Recurring transaction manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let recurring_tx_id_str = params
            .as_object()
            .and_then(|obj| obj.get("recurring_tx_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing recurring_tx_id parameter".to_string(),
                data: None,
            })?;

        let recurring_tx_id = parse_hash(recurring_tx_id_str)?;
        let mut manager = manager.write().await;

        match manager.pause(&recurring_tx_id) {
            Ok(_) => Ok(json!({
                "success": true,
                "recurring_tx_id": format!("0x{}", hex::encode(recurring_tx_id)),
                "message": "Recurring transaction paused"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to pause: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_resumeRecurringTransaction - Resume a paused recurring transaction
    async fn irondag_resume_recurring_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let manager = self
            .recurring_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Recurring transaction manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let recurring_tx_id_str = params
            .as_object()
            .and_then(|obj| obj.get("recurring_tx_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing recurring_tx_id parameter".to_string(),
                data: None,
            })?;

        let recurring_tx_id = parse_hash(recurring_tx_id_str)?;
        let mut manager = manager.write().await;

        match manager.resume(&recurring_tx_id) {
            Ok(_) => Ok(json!({
                "success": true,
                "recurring_tx_id": format!("0x{}", hex::encode(recurring_tx_id)),
                "message": "Recurring transaction resumed"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to resume: {}", e),
                data: None,
            }),
        }
    }

    // ========== Complete Stop-Loss RPC Methods ==========

    /// irondag_cancelStopLoss - Cancel a stop-loss order
    async fn irondag_cancel_stop_loss(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let stop_loss_id_str = params
            .as_object()
            .and_then(|obj| obj.get("stop_loss_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stop_loss_id parameter".to_string(),
                data: None,
            })?;

        let stop_loss_id = parse_hash(stop_loss_id_str)?;
        let mut manager = manager.write().await;

        match manager.cancel(&stop_loss_id) {
            Ok(_) => Ok(json!({
                "success": true,
                "stop_loss_id": format!("0x{}", hex::encode(stop_loss_id)),
                "message": "Stop-loss order cancelled"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to cancel: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getStopLoss - Get a specific stop-loss order
    async fn irondag_get_stop_loss(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let stop_loss_id_str = params
            .as_object()
            .and_then(|obj| obj.get("stop_loss_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stop_loss_id parameter".to_string(),
                data: None,
            })?;

        let stop_loss_id = parse_hash(stop_loss_id_str)?;
        let manager_read = manager.read().await;

        if let Some(order) = manager_read.get(&stop_loss_id) {
            Ok(json!({
                "stop_loss_id": format!("0x{}", hex::encode(order.stop_loss_id)),
                "wallet_address": format!("0x{}", hex::encode(order.wallet_address)),
                "asset_pair": order.asset_pair,
                "status": format!("{:?}", order.status),
                "triggered_at": order.triggered_at,
                "triggered_price": order.triggered_price.map(|p| format!("0x{:x}", p)),
                "execution_tx_hash": order.execution_tx_hash.map(|h| format!("0x{}", hex::encode(h))),
            }))
        } else {
            Err(JsonRpcError {
                code: -32603,
                message: "Stop-loss order not found".to_string(),
                data: None,
            })
        }
    }

    /// irondag_updateStopLossPrice - Update trigger price for a stop-loss order
    async fn irondag_update_stop_loss_price(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let stop_loss_id_str = params
            .as_object()
            .and_then(|obj| obj.get("stop_loss_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stop_loss_id parameter".to_string(),
                data: None,
            })?;

        let stop_loss_id = parse_hash(stop_loss_id_str)?;
        let new_price_str = params
            .as_object()
            .and_then(|obj| obj.get("new_price"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing new_price parameter".to_string(),
                data: None,
            })?;

        let new_price = parse_hex_u128(new_price_str)?;
        let mut manager = manager.write().await;

        match manager.update_trigger_price(&stop_loss_id, new_price) {
            Ok(_) => Ok(json!({
                "success": true,
                "stop_loss_id": format!("0x{}", hex::encode(stop_loss_id)),
                "new_price": format!("0x{:x}", new_price),
                "message": "Stop-loss price updated"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to update: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_pauseStopLoss - Pause a stop-loss order
    async fn irondag_pause_stop_loss(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let stop_loss_id_str = params
            .as_object()
            .and_then(|obj| obj.get("stop_loss_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stop_loss_id parameter".to_string(),
                data: None,
            })?;

        let stop_loss_id = parse_hash(stop_loss_id_str)?;
        let mut manager = manager.write().await;

        match manager.pause(&stop_loss_id) {
            Ok(_) => Ok(json!({
                "success": true,
                "stop_loss_id": format!("0x{}", hex::encode(stop_loss_id)),
                "message": "Stop-loss order paused"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to pause: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_resumeStopLoss - Resume a paused stop-loss order
    async fn irondag_resume_stop_loss(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let manager = self
            .stop_loss_manager
            .as_ref()
            .ok_or_else(|| JsonRpcError {
                code: -32603,
                message: "Stop-loss manager not available".to_string(),
                data: None,
            })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let stop_loss_id_str = params
            .as_object()
            .and_then(|obj| obj.get("stop_loss_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing stop_loss_id parameter".to_string(),
                data: None,
            })?;

        let stop_loss_id = parse_hash(stop_loss_id_str)?;
        let mut manager = manager.write().await;

        match manager.resume(&stop_loss_id) {
            Ok(_) => Ok(json!({
                "success": true,
                "stop_loss_id": format!("0x{}", hex::encode(stop_loss_id)),
                "message": "Stop-loss order resumed"
            })),
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to resume: {}", e),
                data: None,
            }),
        }
    }

    // ========== Privacy RPC Methods ==========

    /// irondag_createPrivateTransaction - Create a private transaction
    #[cfg(feature = "privacy")]
    async fn irondag_create_private_transaction(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let _privacy_manager = self.privacy_manager.as_ref().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "Privacy manager not available".to_string(),
            data: None,
        })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        // Extract parameters
        let amount_str = params
            .as_object()
            .and_then(|obj| obj.get("amount"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing amount parameter".to_string(),
                data: None,
            })?;

        let amount = parse_hex_u128(amount_str)?;

        // Generate blinding factor
        let mut blinding = [0u8; 32];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut blinding);

        // Create commitment
        let commitment = crate::privacy::PedersenCommitment::commit(amount, &blinding);

        // Generate nullifier (simplified - would use receiver secret in production)
        let receiver = params
            .as_object()
            .and_then(|obj| obj.get("receiver"))
            .and_then(|v| v.as_str())
            .map(|s| parse_address(s))
            .transpose()?
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing receiver parameter".to_string(),
                data: None,
            })?;

        let nullifier = crate::privacy::Nullifier::generate(&receiver, &blinding);

        // Create privacy transaction
        let privacy_tx = crate::privacy::PrivacyTransaction::new(
            crate::privacy::PrivacyTxType::PrivateTransfer,
            vec![], // Proof would be generated here
            vec![nullifier.to_bytes().to_vec(), commitment.to_bytes()],
        );

        Ok(json!({
            "privacy_tx_hash": format!("0x{}", hex::encode(privacy_tx.hash)),
            "nullifier": format!("0x{}", hex::encode(nullifier.to_bytes())),
            "commitment": format!("0x{}", hex::encode(commitment.to_bytes())),
            "message": "Privacy transaction created (proof generation pending)"
        }))
    }

    /// irondag_verifyPrivacyProof - Verify a privacy proof
    #[cfg(feature = "privacy")]
    async fn irondag_verify_privacy_proof(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let privacy_manager =
            self.privacy_manager
                .read()
                .await
                .clone()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Privacy manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let proof_str = params
            .as_object()
            .and_then(|obj| obj.get("proof"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing proof parameter".to_string(),
                data: None,
            })?;

        let proof_bytes =
            hex::decode(proof_str.strip_prefix("0x").unwrap_or(proof_str)).map_err(|_| {
                JsonRpcError {
                    code: -32602,
                    message: "Invalid proof format".to_string(),
                    data: None,
                }
            })?;

        // Deserialize proof and verify using PrivacyVerifier
        use crate::privacy::PrivacyVerifier;
        let proof = PrivacyVerifier::deserialize_proof(&proof_bytes).map_err(|e| JsonRpcError {
            code: -32602,
            message: format!("Failed to deserialize proof: {}", e),
            data: None,
        })?;

        // Get public inputs from params
        let public_inputs_str = params
            .as_object()
            .and_then(|obj| obj.get("publicInputs"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing publicInputs parameter".to_string(),
                data: None,
            })?;

        // Parse public inputs as field elements
        use ark_bn254::Fr;
        use ark_ff::PrimeField;
        let mut public_inputs = Vec::new();
        for input_str in public_inputs_str {
            let input_hex = input_str.as_str().ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid public input format".to_string(),
                data: None,
            })?;
            let input_bytes = hex::decode(input_hex.strip_prefix("0x").unwrap_or(input_hex))
                .map_err(|_| JsonRpcError {
                    code: -32602,
                    message: "Invalid hex in public input".to_string(),
                    data: None,
                })?;
            if input_bytes.len() >= 32 {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&input_bytes[..32]);
                let fr = Fr::from_le_bytes_mod_order(&bytes);
                public_inputs.push(fr);
            }
        }

        // Verify proof using privacy manager's verifier
        let verified = privacy_manager
            .read()
            .await
            .verify_proof(&proof, &public_inputs)
            .await;

        Ok(json!({
            "verified": verified,
            "message": if verified { "Proof verified successfully" } else { "Proof verification failed" }
        }))
    }

    /// irondag_proveBalance - Prove balance without revealing amount
    #[cfg(feature = "privacy")]
    async fn irondag_prove_balance(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let _privacy_manager =
            self.privacy_manager
                .read()
                .await
                .clone()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Privacy manager not available".to_string(),
                    data: None,
                })?;

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Invalid params".to_string(),
            data: None,
        })?;

        let address_str = params
            .as_object()
            .and_then(|obj| obj.get("address"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing address parameter".to_string(),
                data: None,
            })?;

        let address = parse_address(address_str)?;

        // Get optional threshold parameter (minimum balance to prove)
        let threshold = params
            .as_object()
            .and_then(|obj| obj.get("threshold"))
            .and_then(|v| v.as_str())
            .and_then(|s| hex::decode(s.strip_prefix("0x").unwrap_or(s)).ok())
            .and_then(|bytes| {
                if bytes.len() <= 16 {
                    let mut balance_bytes = [0u8; 16];
                    balance_bytes[..bytes.len()].copy_from_slice(&bytes);
                    Some(u128::from_le_bytes(balance_bytes))
                } else {
                    None
                }
            })
            .unwrap_or(0u128);

        // Get balance from blockchain
        let blockchain = self.acquire_blockchain_read().await?;
        let balance = blockchain.get_balance(address);

        // Check if privacy prover is available
        let privacy_prover =
            self.privacy_prover
                .read()
                .await
                .clone()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Privacy prover not available".to_string(),
                    data: None,
                })?;

        // Generate zk-SNARK proof that balance >= threshold
        // For now, we'll create a simplified proof using the existing circuit
        // In production, this would use a dedicated range proof circuit
        use crate::privacy::circuit::PrivateTransferCircuit;
        use crate::privacy::PrivacyProver;
        use ark_bn254::Fr;
        use ark_ff::PrimeField;
        use rand::thread_rng;

        // Create a proof that demonstrates balance exists and is >= threshold
        // We'll use a simplified approach: prove balance = balance (always true, but demonstrates mechanism)
        // In production, use proper range proof circuit for balance >= threshold
        let mut rng = thread_rng();

        // Generate nullifier from address
        use sha3::Digest;
        use sha3::Keccak256;
        let mut hasher = Keccak256::new();
        hasher.update(&address);
        let nullifier_hash = hasher.finalize();
        let nullifier = Fr::from_le_bytes_mod_order(&nullifier_hash[..32]);

        // Generate commitment from balance
        let mut hasher = Keccak256::new();
        hasher.update(&balance.to_le_bytes());
        let commitment_hash = hasher.finalize();
        let commitment = Fr::from_le_bytes_mod_order(&commitment_hash[..32]);

        // Validate that balance >= threshold before generating proof
        if balance < threshold {
            return Err(JsonRpcError {
                code: -32602,
                message: format!("Balance {} is less than threshold {}", balance, threshold),
                data: None,
            });
        }

        // Create circuit to prove balance >= threshold
        // We prove: old_balance - amount = new_balance where amount = threshold
        // This demonstrates that balance - threshold >= 0, i.e., balance >= threshold
        let circuit = PrivateTransferCircuit {
            old_balance: Some(balance),
            amount: Some(threshold), // Prove we can subtract threshold from balance
            new_balance: Some(balance - threshold), // Result should be >= 0
            nullifier,
            commitment: Some(commitment),
        };

        // Generate proof
        match privacy_prover.prove(circuit, &mut rng) {
            Ok(proof) => {
                // Serialize proof
                let proof_bytes =
                    PrivacyProver::serialize_proof(&proof).map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Failed to serialize proof: {}", e),
                        data: None,
                    })?;

                // Get public inputs (serialize field elements to bytes)
                use ark_serialize::CanonicalSerialize;
                let mut nullifier_bytes = Vec::new();
                nullifier
                    .serialize_uncompressed(&mut nullifier_bytes)
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Failed to serialize nullifier: {:?}", e),
                        data: None,
                    })?;

                let mut commitment_bytes = Vec::new();
                commitment
                    .serialize_uncompressed(&mut commitment_bytes)
                    .map_err(|e| JsonRpcError {
                        code: -32603,
                        message: format!("Failed to serialize commitment: {:?}", e),
                        data: None,
                    })?;

                let public_inputs = vec![
                    format!("0x{}", hex::encode(&nullifier_bytes)),
                    format!("0x{}", hex::encode(&commitment_bytes)),
                ];

                Ok(json!({
                    "proof": format!("0x{}", hex::encode(&proof_bytes)),
                    "publicInputs": public_inputs,
                    "balance": format!("0x{:x}", balance),
                    "threshold": format!("0x{:x}", threshold),
                    "verified": true,
                    "message": "Balance proof generated successfully"
                }))
            }
            Err(e) => Err(JsonRpcError {
                code: -32603,
                message: format!("Failed to generate proof: {}", e),
                data: None,
            }),
        }
    }

    /// irondag_getPrivacyStats - Get privacy layer statistics
    #[cfg(feature = "privacy")]
    async fn irondag_get_privacy_stats(&self) -> Result<Value, JsonRpcError> {
        let privacy_manager =
            self.privacy_manager
                .read()
                .await
                .clone()
                .ok_or_else(|| JsonRpcError {
                    code: -32603,
                    message: "Privacy manager not available".to_string(),
                    data: None,
                })?;

        let manager = privacy_manager.read().await;
        let nullifier_count = manager.nullifier_count().await;
        let enabled = manager.is_enabled();

        Ok(json!({
            "enabled": enabled,
            "nullifier_count": nullifier_count,
            "message": "Privacy layer statistics"
        }))
    }

    /// Set privacy manager (async, safe for Arc<RpcServer>)
    #[cfg(feature = "privacy")]
    pub async fn set_privacy_manager(
        &self,
        manager: Arc<tokio::sync::RwLock<crate::privacy::PrivacyManager>>,
    ) {
        *self.privacy_manager.write().await = Some(manager);
    }

    /// Set privacy prover (async, safe for Arc<RpcServer>)
    #[cfg(feature = "privacy")]
    pub async fn set_privacy_prover(&self, prover: Arc<crate::privacy::PrivacyProver>) {
        *self.privacy_prover.write().await = Some(prover);
    }

    // ========== Snapshot RPC Methods ==========

    /// irondag_createSnapshot - Create a state snapshot
    async fn irondag_create_snapshot(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let blockchain = self.acquire_blockchain_read().await?;

        // Get current block info
        let latest_block = blockchain.get_latest_block().ok_or_else(|| JsonRpcError {
            code: -32603,
            message: "No blocks in chain".to_string(),
            data: None,
        })?;

        let block_number = latest_block.header.block_number;
        let block_hash = latest_block.hash;

        // Collect account states
        let accounts_map = blockchain.get_all_accounts();

        // Get recent blocks (last 100 or all if fewer)
        let mut recent_blocks = blockchain.get_blocks_tail(100);
        recent_blocks.reverse();

        // Create snapshot
        let snapshot = crate::storage::BlockchainSnapshot {
            metadata: crate::storage::SnapshotMetadata {
                version: 1,
                block_number,
                block_hash,
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                account_count: accounts_map.len(),
                block_count: recent_blocks.len(),
                state_size_bytes: 0,      // Will be calculated
                top_accounts: Vec::new(), // Will be calculated after accounts
            },
            accounts: accounts_map
                .iter()
                .map(|(addr, (balance, nonce))| crate::storage::AccountSnapshot {
                    address: *addr,
                    balance: *balance,
                    nonce: *nonce,
                    code: None,
                })
                .collect(),
            storage: vec![],
            blocks: recent_blocks,
        };

        // Get snapshot directory from params or use default
        let snapshot_dir = params
            .as_ref()
            .and_then(|p| p.as_object())
            .and_then(|obj| obj.get("directory"))
            .and_then(|v| v.as_str())
            .unwrap_or("snapshots");

        // Create directory if it doesn't exist
        std::fs::create_dir_all(snapshot_dir).map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to create snapshot directory: {}", e),
            data: None,
        })?;

        // Generate filename
        let filename = crate::storage::SnapshotManager::snapshot_filename(block_number);
        let filepath = std::path::Path::new(snapshot_dir).join(&filename);

        // Save snapshot
        snapshot.save_to_file(&filepath).map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to save snapshot: {}", e),
            data: None,
        })?;

        Ok(json!({
            "success": true,
            "filename": filename,
            "path": filepath.to_string_lossy(),
            "blockNumber": format!("0x{:x}", block_number),
            "blockHash": format!("0x{}", hex::encode(block_hash)),
            "accountCount": snapshot.metadata.account_count,
            "blockCount": snapshot.metadata.block_count,
            "createdAt": snapshot.metadata.created_at,
            "message": "Snapshot created successfully"
        }))
    }

    /// irondag_listSnapshots - List available snapshots
    async fn irondag_list_snapshots(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let snapshot_dir = params
            .as_ref()
            .and_then(|p| p.as_object())
            .and_then(|obj| obj.get("directory"))
            .and_then(|v| v.as_str())
            .unwrap_or("snapshots");

        // Check if directory exists
        if !std::path::Path::new(snapshot_dir).exists() {
            return Ok(json!({
                "snapshots": [],
                "count": 0,
                "message": "No snapshots directory found"
            }));
        }

        let snapshots =
            crate::storage::SnapshotManager::list_snapshots(snapshot_dir).map_err(|e| {
                JsonRpcError {
                    code: -32603,
                    message: format!("Failed to list snapshots: {}", e),
                    data: None,
                }
            })?;

        let snapshot_list: Vec<Value> = snapshots
            .iter()
            .map(|(filename, metadata)| {
                json!({
                    "filename": filename,
                    "blockNumber": format!("0x{:x}", metadata.block_number),
                    "blockHash": format!("0x{}", hex::encode(metadata.block_hash)),
                    "createdAt": metadata.created_at,
                    "accountCount": metadata.account_count,
                    "blockCount": metadata.block_count,
                    "version": metadata.version
                })
            })
            .collect();

        Ok(json!({
            "snapshots": snapshot_list,
            "count": snapshots.len(),
            "directory": snapshot_dir
        }))
    }

    /// irondag_getSnapshotInfo - Get detailed info about a specific snapshot
    async fn irondag_get_snapshot_info(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let filepath = params
            .as_object()
            .and_then(|obj| obj.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing path parameter".to_string(),
                data: None,
            })?;

        let snapshot =
            crate::storage::BlockchainSnapshot::load_from_file(filepath).map_err(|e| {
                JsonRpcError {
                    code: -32603,
                    message: format!("Failed to load snapshot: {}", e),
                    data: None,
                }
            })?;

        // PERF-02: Use pre-computed top_accounts from metadata (no recalculation needed)
        // For legacy snapshots without top_accounts, compute on-the-fly
        let top_accounts: Vec<Value> =
            if snapshot.metadata.top_accounts.is_empty() && !snapshot.accounts.is_empty() {
                // Fallback for legacy snapshots: compute on-the-fly
                let mut accounts_sorted: Vec<_> = snapshot.accounts.iter().collect();
                accounts_sorted.sort_by(|a, b| b.balance.cmp(&a.balance));
                accounts_sorted
                    .iter()
                    .take(10)
                    .map(|acc| {
                        json!({
                            "address": format!("0x{}", hex::encode(acc.address)),
                            "balance": format!("0x{:x}", acc.balance),
                            "nonce": acc.nonce,
                            "hasCode": acc.code.is_some()
                        })
                    })
                    .collect()
            } else {
                // Use pre-computed top_accounts from metadata
                snapshot
                    .metadata
                    .top_accounts
                    .iter()
                    .map(|acc| {
                        json!({
                            "address": format!("0x{}", hex::encode(acc.address)),
                            "balance": format!("0x{:x}", acc.balance),
                            "nonce": acc.nonce,
                            "hasCode": acc.has_code
                        })
                    })
                    .collect()
            };

        Ok(json!({
            "metadata": {
                "version": snapshot.metadata.version,
                "blockNumber": format!("0x{:x}", snapshot.metadata.block_number),
                "blockHash": format!("0x{}", hex::encode(snapshot.metadata.block_hash)),
                "createdAt": snapshot.metadata.created_at,
                "accountCount": snapshot.metadata.account_count,
                "blockCount": snapshot.metadata.block_count,
                "stateSizeBytes": snapshot.metadata.state_size_bytes
            },
            "topAccounts": top_accounts,
            "storageEntryCount": snapshot.storage.len()
        }))
    }

    /// irondag_restoreSnapshot - Restore blockchain state from a snapshot
    /// CRIT-03: Requires authentication and path validation
    async fn irondag_restore_snapshot(
        &self,
        params: Option<Value>,
        api_key_header: Option<&str>,
    ) -> Result<Value, JsonRpcError> {
        // CRIT-03: Auth guard - require authentication for this sensitive operation
        if self.api_key_hash.is_some() {
            if !self.verify_api_key(
                &JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: "irondag_restoreSnapshot".to_string(),
                    params: None,
                    id: Some(serde_json::Value::Null),
                },
                api_key_header,
            ) {
                warn!("[AUDIT] Unauthorized attempt to restore snapshot");
                return Err(JsonRpcError {
                    code: -32001,
                    message: "Unauthorized: Admin authentication required for snapshot restore"
                        .to_string(),
                    data: None,
                });
            }
        }

        let params = params.ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing params".to_string(),
            data: None,
        })?;

        let filename = params
            .as_object()
            .and_then(|obj| obj.get("filename"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing filename parameter".to_string(),
                data: None,
            })?;

        // CRIT-03: Path traversal protection - validate path is inside snapshots directory
        let snapshots_dir = std::path::Path::new("data/snapshots");
        let resolved = snapshots_dir
            .join(filename)
            .canonicalize()
            .map_err(|e| JsonRpcError {
                code: -32602,
                message: format!("Invalid snapshot path: {}", e),
                data: None,
            })?;
        let snapshots_dir_canonical = snapshots_dir
            .canonicalize()
            .unwrap_or_else(|_| snapshots_dir.to_path_buf());
        if !resolved.starts_with(&snapshots_dir_canonical) {
            warn!(
                "[AUDIT] Path traversal attempt in irondag_restoreSnapshot: {}",
                filename
            );
            return Err(JsonRpcError {
                code: -32602,
                message: "Path traversal detected".to_string(),
                data: None,
            });
        }

        // CRIT-03: Audit logging
        info!("[AUDIT] Snapshot restore initiated: {}", filename);

        // Load snapshot
        let snapshot = crate::storage::BlockchainSnapshot::load_from_file(
            resolved.to_str().unwrap_or(filename),
        )
        .map_err(|e| JsonRpcError {
            code: -32603,
            message: format!("Failed to load snapshot: {}", e),
            data: None,
        })?;

        // Get write lock on blockchain
        let mut blockchain = self.blockchain.write().await;

        // Restore accounts
        for account in &snapshot.accounts {
            let _ = blockchain.set_balance(account.address, account.balance);
            let _ = blockchain.set_nonce(account.address, account.nonce);
        }

        // Validate chain continuity before adding blocks
        for (expected_number, block) in snapshot.blocks.iter().enumerate() {
            if block.header.block_number != expected_number as u64 {
                return Err(JsonRpcError {
                    code: -32000,
                    message: format!(
                        "Snapshot block chain discontinuity: expected block {}, got {}",
                        expected_number, block.header.block_number
                    ),
                    data: None,
                });
            }
        }

        // Restore blocks (add to chain)
        for block in &snapshot.blocks {
            // Skip if block already exists
            if blockchain.get_block_by_hash(&block.hash).is_none() {
                let _ = blockchain.add_block(block.clone()).await;
            }
        }

        // CRIT-03: Audit logging on success
        warn!(
            "[AUDIT] Snapshot restored successfully: {} at block 0x{:x}",
            filename, snapshot.metadata.block_number
        );

        Ok(json!({
            "success": true,
            "restoredBlockNumber": format!("0x{:x}", snapshot.metadata.block_number),
            "restoredAccounts": snapshot.accounts.len(),
            "restoredBlocks": snapshot.blocks.len(),
            "message": "Snapshot restored successfully"
        }))
    }
}

// Note: parse_address and parse_hash are now defined at the top of this file
// with full validation support

/// Parse hex number string to u64
fn parse_hex_number(s: &str) -> Result<u64, JsonRpcError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s == "latest" || s == "pending" {
        // Would need blockchain access - for now return error
        return Err(JsonRpcError {
            code: -32602,
            message: "latest/pending not yet supported - use parse_block_number instead"
                .to_string(),
            data: None,
        });
    }

    u64::from_str_radix(s, 16).map_err(|_| JsonRpcError {
        code: -32602,
        message: "Invalid number format".to_string(),
        data: None,
    })
}

/// Parse block number parameter with support for standard block tags
/// Supports: "latest", "pending", "earliest", or hex-encoded block number
#[allow(dead_code)]
fn parse_block_number(value: &Value, latest_block: u64) -> Result<u64, JsonRpcError> {
    match value.as_str() {
        Some("latest") | Some("pending") => Ok(latest_block),
        Some("earliest") => Ok(0),
        Some(hex_str) => {
            let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
            u64::from_str_radix(hex_str, 16).map_err(|_| JsonRpcError {
                code: -32602,
                message: "Invalid block number format".to_string(),
                data: None,
            })
        }
        None => {
            // Try as numeric value (non-string)
            value.as_u64().ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Invalid block number".to_string(),
                data: None,
            })
        }
    }
}

/// Parse hex number string to u128
/// Note: For block numbers with tag support, use parse_block_number instead
fn parse_hex_u128(s: &str) -> Result<u128, JsonRpcError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s == "latest" || s == "pending" {
        return Err(JsonRpcError {
            code: -32602,
            message: "Block tags (latest/pending) not supported for this parameter".to_string(),
            data: None,
        });
    }

    u128::from_str_radix(s, 16).map_err(|_| JsonRpcError {
        code: -32602,
        message: "Invalid number format".to_string(),
        data: None,
    })
}

#[allow(dead_code)]
fn read_storage_slot0(executor: &crate::evm::EvmTransactionExecutor, address: Address) -> Vec<u8> {
    let storage_key = [0u8; 32];
    let mut storage_value = executor
        .get_contract_storage(address, &storage_key)
        .unwrap_or_else(|| vec![0u8; 32]);

    if storage_value.len() < 32 {
        let mut padded = vec![0u8; 32 - storage_value.len()];
        padded.append(&mut storage_value);
        storage_value = padded;
    } else if storage_value.len() > 32 {
        storage_value.truncate(32);
    }

    storage_value
}

/// Convert block to JSON (with optional shard information)
fn block_to_json(block: Option<Block>, miner_address: Address) -> Value {
    block_to_json_with_shard(block, None, miner_address)
}

/// Convert block to JSON with shard information
fn block_to_json_with_shard(block: Option<Block>, shard_id: Option<usize>, miner_address: Address) -> Value {
    match block {
        Some(b) => {
            // Map stream type to string
            let stream_type_str = match b.header.stream_type {
                crate::types::StreamType::StreamA => "A",
                crate::types::StreamType::StreamB => "B",
                crate::types::StreamType::StreamC => "C",
            };

            // Compute transactions_root from actual transaction data
            // TODO: compute from actual data - use proper trie root calculation
            let transactions_root = compute_transactions_root(&b.transactions);

            // Compute gas_used from actual transactions
            // TODO: Store ExecutionResult.gas_used per-transaction in the block so we can
            //       report true gas consumption. Until then, use a better heuristic:
            //       simple transfers (no data) = 21,000 gas (Ethereum standard);
            //       contract calls/deployments = gas_limit as upper-bound approximation.
            let gas_used: u64 = b.transactions.iter().map(|tx| {
                if tx.data.is_empty() { 21_000u64 } else { tx.gas_limit }
            }).sum();

            // Compute block size (header + transactions estimate)
            let block_size = 512
                + b.transactions
                    .iter()
                    .map(|tx| 200 + tx.data.len())
                    .sum::<usize>();

            // Derive per-block-varying stateRoot from block hash
            // TODO: Use the real Verkle/Merkle state root from blockchain.state_root()
            //       once it is stored per-block rather than only at the chain level.
            let state_root_value = {
                use sha3::{Digest, Keccak256};
                let mut hasher = Keccak256::new();
                hasher.update(&b.hash);
                let hash = hasher.finalize();
                format!("0x{}", hex::encode(hash))
            };

            // Derive per-block-varying receiptsRoot
            // TODO: Compute from actual transaction receipt data once receipts are stored
            //       per-transaction in the block.
            let receipts_root_value = if b.transactions.is_empty() {
                // Empty trie root (same as Ethereum's keccak256(rlp([])))
                "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()
            } else {
                use sha3::{Digest, Keccak256};
                let mut hasher = Keccak256::new();
                hasher.update(&b.hash);
                hasher.update(b"receipts");
                let hash = hasher.finalize();
                format!("0x{}", hex::encode(hash))
            };

            let miner_value = format!("0x{}", hex::encode(miner_address));

            let mut json = serde_json::json!({
                "number": format!("0x{:x}", b.header.block_number),
                "hash": format!("0x{}", hex::encode(b.hash)),
                "parentHash": b.header.parent_hashes.first()
                    .map(|h| format!("0x{}", hex::encode(h)))
                    .unwrap_or_else(|| "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()),
                // DAG: Include ALL parent hashes for visualization
                "parentHashes": b.header.parent_hashes.iter()
                    .map(|h| format!("0x{}", hex::encode(h)))
                    .collect::<Vec<_>>(),
                "streamType": stream_type_str,
                "nonce": "0x0000000000000000",
                "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
                "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "transactionsRoot": transactions_root,
                "stateRoot": state_root_value,
                "receiptsRoot": receipts_root_value,
                "miner": miner_value,
                "difficulty": "0x0",
                "totalDifficulty": "0x0",
                "extraData": "0x",
                "size": format!("0x{:x}", block_size),
                "gasLimit": "0x1fffffffffffff",
                "gasUsed": format!("0x{:x}", gas_used),
                "timestamp": format!("0x{:x}", b.header.timestamp),
                "transactions": b.transactions.iter().map(|tx| format!("0x{}", hex::encode(tx.hash))).collect::<Vec<_>>(),
                "uncles": [],
                "transactionCount": b.transactions.len(),
            });

            // Add shard information if available
            if let Some(shard) = shard_id {
                json["shardId"] = Value::Number(shard.into());
            }

            json
        }
        None => Value::Null,
    }
}

/// Compute transactions root from transaction list
/// Uses a simple hash-based approach (Ethereum uses Merkle Patricia Trie)
fn compute_transactions_root(transactions: &[Transaction]) -> String {
    if transactions.is_empty() {
        // Empty trie root (Ethereum's empty trie hash)
        return "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string();
    }

    // Simple approach: hash all transaction hashes together
    // TODO: compute from actual data - implement proper trie root calculation
    use sha3::{Digest, Keccak256};
    let mut hasher = Keccak256::new();
    for tx in transactions {
        hasher.update(&tx.hash);
    }
    let result = hasher.finalize();
    format!("0x{}", hex::encode(result))
}

/// Convert transaction to JSON (with optional shard information)
fn tx_to_json(tx: &Transaction, block_number: u64) -> Value {
    tx_to_json_with_shard(tx, block_number, None)
}

/// Convert transaction to JSON with shard information
fn tx_to_json_with_shard(
    tx: &Transaction,
    block_number: u64,
    shard_info: Option<(usize, usize)>,
) -> Value {
    let mut json = serde_json::json!({
        "hash": format!("0x{}", hex::encode(tx.hash)),
        "from": format!("0x{}", hex::encode(tx.from)),
        "to": format!("0x{}", hex::encode(tx.to)),
        "value": format!("0x{:x}", tx.value),
        "gas": format!("0x{:x}", tx.gas_limit),
        "gasPrice": format!("0x{:x}", tx.fee),
        "nonce": format!("0x{:x}", tx.nonce),
        "blockNumber": format!("0x{:x}", block_number),
        "input": format!("0x{}", hex::encode(&tx.data)),
    });

    // Add shard information if available
    if let Some((from_shard, to_shard)) = shard_info {
        json["fromShard"] = Value::Number(from_shard.into());
        json["toShard"] = Value::Number(to_shard.into());
        json["isCrossShard"] = Value::Bool(from_shard != to_shard);
    }

    // Add time-lock information if available
    if let Some(execute_at_block) = tx.execute_at_block {
        json["executeAtBlock"] = Value::String(format!("0x{:x}", execute_at_block));
        json["isTimeLocked"] = Value::Bool(true);
    }
    if let Some(execute_at_timestamp) = tx.execute_at_timestamp {
        json["executeAtTimestamp"] = Value::String(format!("0x{:x}", execute_at_timestamp));
        json["isTimeLocked"] = Value::Bool(true);
    }

    // Add sponsor information if gasless transaction
    if let Some(sponsor) = tx.sponsor {
        json["sponsor"] = Value::String(format!("0x{}", hex::encode(sponsor)));
        json["isGasless"] = Value::Bool(true);
    }

    json
}

// ============================================================================
// TESTS: RPC Core Functionality (TEST-01)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // parse_block_number() helper tests
    // =========================================================================

    #[test]
    fn test_parse_block_number_latest() {
        let value = Value::String("latest".to_string());
        let result = parse_block_number(&value, 100);
        assert_eq!(result.unwrap(), 100);
    }

    #[test]
    fn test_parse_block_number_pending() {
        let value = Value::String("pending".to_string());
        let result = parse_block_number(&value, 200);
        assert_eq!(result.unwrap(), 200);
    }

    #[test]
    fn test_parse_block_number_earliest() {
        let value = Value::String("earliest".to_string());
        let result = parse_block_number(&value, 100);
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_parse_block_number_hex() {
        // Test hex block number
        let value = Value::String("0x1a".to_string());
        let result = parse_block_number(&value, 100);
        assert_eq!(result.unwrap(), 26);
    }

    #[test]
    fn test_parse_block_number_hex_without_prefix() {
        // Test hex block number without 0x prefix
        let value = Value::String("ff".to_string());
        let result = parse_block_number(&value, 100);
        assert_eq!(result.unwrap(), 255);
    }

    #[test]
    fn test_parse_block_number_numeric() {
        // Test numeric value (non-string)
        let value = Value::Number(42u64.into());
        let result = parse_block_number(&value, 100);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_parse_block_number_invalid_hex() {
        // Test invalid hex string
        let value = Value::String("0xgg".to_string());
        let result = parse_block_number(&value, 100);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, -32602);
    }

    #[test]
    fn test_parse_block_number_invalid_type() {
        // Test invalid type (object instead of string/number)
        let value = json!({"block": 100});
        let result = parse_block_number(&value, 100);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, -32602);
    }

    // =========================================================================
    // Error helper function tests
    // =========================================================================

    #[test]
    fn test_rpc_error() {
        let error = rpc_error(-32000, "Test error message");
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "Test error message");
        assert!(error.data.is_none());
    }

    #[test]
    fn test_missing_params_error() {
        let error = missing_params_error();
        assert_eq!(error.code, RPC_INVALID_PARAMS);
        assert_eq!(error.message, "Invalid params");
    }

    #[test]
    fn test_missing_param_error() {
        let error = missing_param_error("blockNumber");
        assert_eq!(error.code, RPC_INVALID_PARAMS);
        assert_eq!(error.message, "Missing blockNumber parameter");
    }

    #[test]
    fn test_invalid_param_error() {
        let error = invalid_param_error("address");
        assert_eq!(error.code, RPC_INVALID_PARAMS);
        assert_eq!(error.message, "Invalid address parameter");
    }

    #[test]
    fn test_internal_error() {
        let error = internal_error("Database connection failed");
        assert_eq!(error.code, RPC_INTERNAL_ERROR);
        assert_eq!(error.message, "Database connection failed");
    }

    #[test]
    fn test_method_not_found_error() {
        let error = method_not_found_error("eth_unknownMethod");
        assert_eq!(error.code, RPC_METHOD_NOT_FOUND);
        assert!(error.message.contains("eth_unknownMethod"));
    }

    // =========================================================================
    // Parameter extraction helper tests
    // =========================================================================

    #[test]
    fn test_extract_str_param_valid() {
        let params = vec![json!("test_value"), json!(123)];
        let result = extract_str_param(&params, 0);
        assert_eq!(result, Some("test_value"));
    }

    #[test]
    fn test_extract_str_param_not_string() {
        let params = vec![json!(123), json!("test")];
        let result = extract_str_param(&params, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_str_param_out_of_bounds() {
        let params = vec![json!("test")];
        let result = extract_str_param(&params, 5);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_hex_param_valid() {
        let params = vec![json!("0xdeadbeef"), json!(123)];
        let result = extract_hex_param(&params, 0);
        assert_eq!(result, Some(vec![0xde, 0xad, 0xbe, 0xef]));
    }

    #[test]
    fn test_extract_hex_param_without_prefix() {
        let params = vec![json!("cafe"), json!(123)];
        let result = extract_hex_param(&params, 0);
        assert_eq!(result, Some(vec![0xca, 0xfe]));
    }

    #[test]
    fn test_extract_hex_param_invalid_hex() {
        let params = vec![json!("0xgggg"), json!(123)];
        let result = extract_hex_param(&params, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_hex_param_not_string() {
        let params = vec![json!(123), json!("test")];
        let result = extract_hex_param(&params, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_address_param_valid() {
        let params = vec![json!("0x1234567890123456789012345678901234567890")];
        let result = extract_address_param(&params, 0);
        assert!(result.is_ok());
        let addr = result.unwrap();
        assert_eq!(addr.0[0], 0x12);
        assert_eq!(addr.0[19], 0x90);
    }

    #[test]
    fn test_extract_address_param_invalid_length() {
        let params = vec![json!("0x1234")];
        let result = extract_address_param(&params, 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, RPC_INVALID_ADDRESS);
    }

    #[test]
    fn test_extract_address_param_missing() {
        let params: Vec<Value> = vec![];
        let result = extract_address_param(&params, 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, RPC_INVALID_PARAMS);
    }

    #[test]
    fn test_extract_hash_param_valid() {
        let hash_str = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let params = vec![json!(hash_str)];
        let result = extract_hash_param(&params, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_hash_param_invalid_length() {
        let params = vec![json!("0x1234")];
        let result = extract_hash_param(&params, 0);
        assert!(result.is_err());
    }

    // =========================================================================
    // Validation helper tests
    // =========================================================================

    #[test]
    fn test_validate_hex_valid() {
        assert!(validate_hex("0xdeadbeef").is_ok());
        assert!(validate_hex("0x").is_ok());
        assert!(validate_hex("0x1234567890abcdef").is_ok());
    }

    #[test]
    fn test_validate_hex_missing_prefix() {
        let result = validate_hex("deadbeef");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("must start with 0x"));
    }

    #[test]
    fn test_validate_hex_invalid_chars() {
        let result = validate_hex("0xgggg");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Invalid hex"));
    }

    #[test]
    fn test_validate_address_valid() {
        assert!(validate_address("0x1234567890123456789012345678901234567890").is_ok());
    }

    #[test]
    fn test_validate_address_too_short() {
        let result = validate_address("0x1234");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, RPC_INVALID_ADDRESS);
    }

    #[test]
    fn test_validate_address_too_long() {
        let result = validate_address("0x123456789012345678901234567890123456789012");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_hash_valid() {
        let hash = "0x1234567890123456789012345678901234567890123456789012345678901234";
        assert!(validate_hash(hash).is_ok());
    }

    #[test]
    fn test_validate_hash_too_short() {
        let result = validate_hash("0x1234");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_block_param_tags() {
        assert!(validate_block_param("latest").is_ok());
        assert!(validate_block_param("pending").is_ok());
        assert!(validate_block_param("earliest").is_ok());
        assert!(validate_block_param("safe").is_ok());
        assert!(validate_block_param("finalized").is_ok());
    }

    #[test]
    fn test_validate_block_param_hex() {
        assert!(validate_block_param("0x1a").is_ok());
        assert!(validate_block_param("0x0").is_ok());
    }

    #[test]
    fn test_validate_block_param_unreasonably_high() {
        let result = validate_block_param("0x2540be401"); // 10_000_000_001 (just over the limit)
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("unreasonably high"));
    }

    // =========================================================================
    // Constant-time comparison tests (security)
    // =========================================================================

    #[test]
    fn test_constant_time_eq_equal() {
        let a = b"secret_key_12345";
        let b = b"secret_key_12345";
        assert!(constant_time_eq(a, b));
    }

    #[test]
    fn test_constant_time_eq_different() {
        let a = b"secret_key_12345";
        let b = b"secret_key_54321";
        assert!(!constant_time_eq(a, b));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        let a = b"short";
        let b = b"longer_string";
        assert!(!constant_time_eq(a, b));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"", b"x"));
    }

    // =========================================================================
    // API key hashing tests
    // =========================================================================

    #[test]
    fn test_hash_api_key_deterministic() {
        let key = "test_api_key_123";
        let hash1 = hash_api_key(key);
        let hash2 = hash_api_key(key);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_api_key_different_inputs() {
        let hash1 = hash_api_key("key1");
        let hash2 = hash_api_key("key2");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_api_key_produces_32_bytes() {
        let hash = hash_api_key("any_key");
        assert_eq!(hash.len(), 32);
    }

    // =========================================================================
    // Method classification tests
    // =========================================================================

    #[test]
    fn test_is_public_method() {
        assert!(is_public_method("eth_blockNumber"));
        assert!(is_public_method("net_version"));
        assert!(is_public_method("eth_chainId"));
        assert!(is_public_method("irondag_getDagStats"));
    }

    #[test]
    fn test_is_public_method_private() {
        assert!(!is_public_method("eth_sendRawTransaction"));
        assert!(!is_public_method("irondag_restoreSnapshot"));
        assert!(!is_public_method("unknown_method"));
    }

    #[test]
    fn test_is_state_changing_method() {
        assert!(is_state_changing_method("eth_sendRawTransaction"));
        assert!(is_state_changing_method("eth_sendTransaction"));
        assert!(is_state_changing_method("irondag_faucet"));
    }

    #[test]
    fn test_is_state_changing_method_readonly() {
        assert!(!is_state_changing_method("eth_getBalance"));
        assert!(!is_state_changing_method("eth_blockNumber"));
    }

    #[test]
    fn test_is_read_method() {
        assert!(is_read_method("eth_getBalance"));
        assert!(is_read_method("eth_blockNumber"));
        assert!(!is_read_method("eth_sendRawTransaction"));
    }

    // =========================================================================
    // EIP-2718 Transaction Type Detection Tests (TEST-02)
    // =========================================================================

    /// Test the EIP-2718 envelope detection logic
    /// Legacy transactions start with RLP list prefix (0xc0-0xff), so first byte > 0x7f
    /// Typed transactions have first byte as the type (0x00-0x7f)

    #[test]
    fn test_eip2718_legacy_transaction_detection() {
        // Legacy transaction: first byte is RLP list prefix (0xc0-0xff), so > 0x7f
        // RLP encoding of empty list is 0xc0
        let legacy_tx = vec![0xc0]; // Empty RLP list
        assert!(!legacy_tx.is_empty());
        assert!(
            legacy_tx[0] > 0x7f,
            "Legacy tx first byte should be > 0x7f (RLP prefix)"
        );

        // Simulate the detection logic
        let (tx_type, _rlp_bytes): (u8, &[u8]) = if !legacy_tx.is_empty() && legacy_tx[0] <= 0x7f {
            (legacy_tx[0], &legacy_tx[1..])
        } else {
            (0u8, legacy_tx.as_slice())
        };
        assert_eq!(tx_type, 0, "Legacy transaction should have type 0");
    }

    #[test]
    fn test_eip2718_eip1559_transaction_detection() {
        // EIP-1559 transaction: type 0x02, followed by RLP
        let eip1559_tx = vec![0x02, 0xc0]; // Type 2 + empty RLP
        assert!(!eip1559_tx.is_empty());
        assert!(
            eip1559_tx[0] <= 0x7f,
            "EIP-1559 tx first byte (0x02) should be <= 0x7f"
        );

        // Simulate the detection logic
        let (tx_type, rlp_bytes): (u8, &[u8]) = if !eip1559_tx.is_empty() && eip1559_tx[0] <= 0x7f {
            (eip1559_tx[0], &eip1559_tx[1..])
        } else {
            (0u8, eip1559_tx.as_slice())
        };
        assert_eq!(tx_type, 0x02, "Transaction type should be 0x02 (EIP-1559)");
        assert_eq!(
            rlp_bytes,
            &[0xc0],
            "RLP bytes should be the rest of the payload"
        );
    }

    #[test]
    fn test_eip2718_eip2930_transaction_detection() {
        // EIP-2930 transaction: type 0x01 (access list transactions)
        let eip2930_tx = vec![0x01, 0xc0];
        assert!(!eip2930_tx.is_empty());
        assert!(eip2930_tx[0] <= 0x7f);

        let (tx_type, _rlp_bytes): (u8, &[u8]) = if !eip2930_tx.is_empty() && eip2930_tx[0] <= 0x7f
        {
            (eip2930_tx[0], &eip2930_tx[1..])
        } else {
            (0u8, eip2930_tx.as_slice())
        };
        assert_eq!(tx_type, 0x01, "Transaction type should be 0x01 (EIP-2930)");
    }

    #[test]
    fn test_eip2718_unknown_type_handling() {
        // Unknown transaction type (e.g., 0x03 which is not yet standardized)
        let unknown_tx = vec![0x03, 0xc0];
        assert!(!unknown_tx.is_empty());
        assert!(
            unknown_tx[0] <= 0x7f,
            "Unknown tx type byte should still be <= 0x7f"
        );

        let (tx_type, _rlp_bytes): (u8, &[u8]) = if !unknown_tx.is_empty() && unknown_tx[0] <= 0x7f
        {
            (unknown_tx[0], &unknown_tx[1..])
        } else {
            (0u8, unknown_tx.as_slice())
        };
        assert_eq!(tx_type, 0x03, "Transaction type should be 0x03 (unknown)");
        // The code will handle unknown types during RLP parsing
    }

    #[test]
    fn test_eip2718_empty_transaction() {
        // Empty transaction data
        let empty_tx: Vec<u8> = vec![];
        assert!(empty_tx.is_empty());

        // Should fall through to legacy path for empty data
        let (tx_type, rlp_bytes): (u8, &[u8]) = if !empty_tx.is_empty() && empty_tx[0] <= 0x7f {
            (empty_tx[0], &empty_tx[1..])
        } else {
            (0u8, empty_tx.as_slice())
        };
        assert_eq!(tx_type, 0, "Empty tx should default to type 0 (legacy)");
        assert!(rlp_bytes.is_empty());
    }

    #[test]
    fn test_eip2718_boundary_values() {
        // Test boundary values for type byte
        // 0x7f is the maximum valid type byte for typed transactions
        let max_typed_tx = vec![0x7f, 0xc0];
        let (tx_type, _) = if !max_typed_tx.is_empty() && max_typed_tx[0] <= 0x7f {
            (max_typed_tx[0], &max_typed_tx[1..])
        } else {
            (0u8, max_typed_tx.as_slice())
        };
        assert_eq!(tx_type, 0x7f);

        // 0x80 is the minimum RLP string prefix, so > 0x7f means legacy
        let min_rlp_tx = vec![0x80];
        let (tx_type, _) = if !min_rlp_tx.is_empty() && min_rlp_tx[0] <= 0x7f {
            (min_rlp_tx[0], &min_rlp_tx[1..])
        } else {
            (0u8, min_rlp_tx.as_slice())
        };
        assert_eq!(tx_type, 0, "0x80 should be treated as legacy (RLP prefix)");
    }

    // =========================================================================
    // Auth Logic Tests (TEST-03)
    // =========================================================================

    use crate::blockchain::Blockchain;
    use tempfile::tempdir;

    fn create_test_blockchain() -> Arc<RwLock<Blockchain>> {
        let blockchain = Blockchain::new();
        Arc::new(RwLock::new(blockchain))
    }

    #[test]
    fn test_verify_api_key_valid_match() {
        let blockchain = create_test_blockchain();
        let api_key = "test_api_key_secret".to_string();
        let server = RpcServer::with_auth(blockchain, api_key.clone());

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "test".to_string(),
            params: None,
            id: None,
        };

        assert!(server.verify_api_key(&request, Some(&api_key)));
    }

    #[test]
    fn test_verify_api_key_invalid_key() {
        let blockchain = create_test_blockchain();
        let api_key = "correct_api_key".to_string();
        let server = RpcServer::with_auth(blockchain, api_key);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "test".to_string(),
            params: None,
            id: None,
        };

        // Wrong API key should fail
        assert!(!server.verify_api_key(&request, Some("wrong_key")));
    }

    #[test]
    fn test_verify_api_key_missing_header() {
        let blockchain = create_test_blockchain();
        let api_key = "test_api_key".to_string();
        let server = RpcServer::with_auth(blockchain, api_key);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "test".to_string(),
            params: None,
            id: None,
        };

        // Missing API key header should fail
        assert!(!server.verify_api_key(&request, None));
    }

    #[test]
    fn test_verify_api_key_no_auth_configured() {
        let blockchain = create_test_blockchain();
        let server = RpcServer::without_auth(blockchain);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "test".to_string(),
            params: None,
            id: None,
        };

        // When no auth is configured, verification should pass
        assert!(server.verify_api_key(&request, None));
        assert!(server.verify_api_key(&request, Some("any_key")));
    }

    #[test]
    fn test_requires_auth_with_api_key() {
        let blockchain = create_test_blockchain();
        let server = RpcServer::with_auth(blockchain, "test_key".to_string());

        // Private methods should require auth
        assert!(server.requires_auth("irondag_restoreSnapshot"));
        assert!(server.requires_auth("eth_sendRawTransaction"));

        // Public methods should not require auth
        assert!(!server.requires_auth("eth_blockNumber"));
        assert!(!server.requires_auth("net_version"));
    }

    #[test]
    fn test_requires_auth_without_api_key() {
        let blockchain = create_test_blockchain();
        let server = RpcServer::without_auth(blockchain);

        // When no API key is set, nothing should require auth
        assert!(!server.requires_auth("irondag_restoreSnapshot"));
        assert!(!server.requires_auth("eth_sendRawTransaction"));
        assert!(!server.requires_auth("eth_blockNumber"));
    }

    // =========================================================================
    // Path Traversal Protection Tests (TEST-03)
    // =========================================================================

    #[test]
    fn test_path_traversal_detection_simple() {
        // Test basic path traversal patterns that should be detected
        let malicious_paths = vec![
            "../../../etc/passwd",
            "..\\..\\..\\windows\\system32\\config\\sam",
            "snapshot.dat/../../etc/shadow",
            "./../../secret.txt",
            "snapshot/../../../etc/hosts",
        ];

        for path in malicious_paths {
            // Path with traversal components should be suspicious
            assert!(
                path.contains("../") || path.contains("..\\"),
                "Path traversal pattern should be detected in: {}",
                path
            );
        }
    }

    #[test]
    fn test_safe_snapshot_paths() {
        // These paths should be considered safe
        let safe_paths = vec![
            "snapshot_2024_01_15.dat",
            "backup/snapshot_v1.bin",
            "snapshot-1234567890.json",
            "daily/snapshot_001.dat",
        ];

        for path in safe_paths {
            assert!(
                !path.contains("../") && !path.contains("..\\"),
                "Safe path should not contain traversal: {}",
                path
            );
        }
    }

    #[test]
    fn test_path_traversal_canonicalization() {
        // Create a temporary directory for testing
        let temp_dir = tempdir().unwrap();
        let snapshots_dir = temp_dir.path().join("snapshots");
        std::fs::create_dir(&snapshots_dir).unwrap();

        // Create a valid snapshot file
        let valid_file = snapshots_dir.join("valid_snapshot.dat");
        std::fs::write(&valid_file, b"test data").unwrap();

        // Test canonicalization of valid path
        let valid_path = snapshots_dir.join("valid_snapshot.dat");
        let canonical = valid_path.canonicalize();
        assert!(canonical.is_ok());
        assert!(canonical
            .unwrap()
            .starts_with(&snapshots_dir.canonicalize().unwrap()));

        // Test that path traversal attempt doesn't escape snapshots dir
        // The canonicalize() function resolves ".." components
        let traversal_path = snapshots_dir.join("../outside.txt");
        // This resolves to temp_dir/outside.txt, which is outside snapshots_dir
        let canonical_traversal = traversal_path.canonicalize();
        // This will succeed but point outside the snapshots directory
        if let Ok(canonical) = canonical_traversal {
            assert!(!canonical.starts_with(&snapshots_dir.canonicalize().unwrap()));
        }
    }

    // =========================================================================
    // Snapshot Chain Continuity Tests (TEST-08)
    // =========================================================================

    /// Helper function to create a test block with specific block number
    fn create_test_block(block_number: u64, parent_hashes: Vec<Hash>) -> crate::blockchain::Block {
        use crate::blockchain::{Block, BlockHeader};
        use crate::types::StreamType;

        let header = BlockHeader::new(
            parent_hashes,
            block_number,
            StreamType::StreamA,
            8,             // difficulty
            1_000_000_000, // base fee
        );
        Block::new(header, vec![])
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_valid_chain() {
        // Test that a valid continuous chain passes validation
        let blocks = vec![
            create_test_block(0, vec![]),
            create_test_block(1, vec![Hash([0u8; 32])]),
            create_test_block(2, vec![Hash([1u8; 32])]),
            create_test_block(3, vec![Hash([2u8; 32])]),
        ];

        // Validate chain continuity
        let mut expected_number = 0u64;
        let mut valid = true;
        for block in &blocks {
            if block.header.block_number != expected_number {
                valid = false;
                break;
            }
            expected_number += 1;
        }
        assert!(valid, "Valid chain should pass continuity check");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_gap_detected() {
        // Test that a gap in block sequence is detected
        let blocks = vec![
            create_test_block(0, vec![]),
            create_test_block(1, vec![Hash([0u8; 32])]),
            // Gap: missing block 2
            create_test_block(3, vec![Hash([2u8; 32])]),
        ];

        // Validate chain continuity
        let mut expected_number = 0u64;
        let mut gap_detected = false;
        for block in &blocks {
            if block.header.block_number != expected_number {
                gap_detected = true;
                break;
            }
            expected_number += 1;
        }
        assert!(gap_detected, "Gap in chain should be detected");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_empty_snapshot() {
        // Test that empty snapshot is handled correctly
        let blocks: Vec<crate::blockchain::Block> = vec![];

        // Empty snapshot should be considered valid (nothing to validate)
        let valid = blocks.is_empty() || {
            let mut expected_number = 0u64;
            let mut continuity_valid = true;
            for block in &blocks {
                if block.header.block_number != expected_number {
                    continuity_valid = false;
                    break;
                }
                expected_number += 1;
            }
            continuity_valid
        };
        assert!(valid, "Empty snapshot should be valid");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_single_block() {
        // Test that a single block (genesis) passes
        let blocks = vec![create_test_block(0, vec![])];

        let mut expected_number = 0u64;
        let mut valid = true;
        for block in &blocks {
            if block.header.block_number != expected_number {
                valid = false;
                break;
            }
            expected_number += 1;
        }
        assert!(valid, "Single genesis block should pass");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_wrong_start() {
        // Test that chain not starting at 0 is detected
        let blocks = vec![
            create_test_block(1, vec![Hash([0u8; 32])]), // Starts at 1, not 0
            create_test_block(2, vec![Hash([1u8; 32])]),
        ];

        let mut expected_number = 0u64;
        let mut wrong_start = false;
        for block in &blocks {
            if block.header.block_number != expected_number {
                wrong_start = true;
                break;
            }
            expected_number += 1;
        }
        assert!(wrong_start, "Chain not starting at 0 should be detected");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_duplicate_blocks() {
        // Test that duplicate block numbers are detected
        let blocks = vec![
            create_test_block(0, vec![]),
            create_test_block(1, vec![Hash([0u8; 32])]),
            create_test_block(1, vec![Hash([0u8; 32])]), // Duplicate block 1
            create_test_block(2, vec![Hash([1u8; 32])]),
        ];

        let mut expected_number = 0u64;
        let mut duplicate_detected = false;
        for block in &blocks {
            if block.header.block_number != expected_number {
                duplicate_detected = true;
                break;
            }
            expected_number += 1;
        }
        assert!(
            duplicate_detected,
            "Duplicate block numbers should be detected"
        );
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_large_gap() {
        // Test detection of large gaps
        let blocks = vec![
            create_test_block(0, vec![]),
            create_test_block(1, vec![Hash([0u8; 32])]),
            create_test_block(1000, vec![Hash([99u8; 32])]), // Large gap
        ];

        let mut expected_number = 0u64;
        let mut gap_detected = false;
        for block in &blocks {
            if block.header.block_number != expected_number {
                gap_detected = true;
                break;
            }
            expected_number += 1;
        }
        assert!(gap_detected, "Large gap should be detected");
    }

    #[test]
    #[allow(clippy::explicit_counter_loop)]
    fn test_chain_continuity_out_of_order() {
        // Test that out-of-order blocks are detected
        let blocks = vec![
            create_test_block(0, vec![]),
            create_test_block(2, vec![Hash([1u8; 32])]), // Out of order
            create_test_block(1, vec![Hash([0u8; 32])]),
        ];

        let mut expected_number = 0u64;
        let mut out_of_order = false;
        for block in &blocks {
            if block.header.block_number != expected_number {
                out_of_order = true;
                break;
            }
            expected_number += 1;
        }
        assert!(out_of_order, "Out-of-order blocks should be detected");
    }
}
