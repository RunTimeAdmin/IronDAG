//! Common types used throughout the blockchain

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha3::{Digest, Keccak256};
use std::fmt;
use std::ops::{Deref, Index};

/// Default IronDAG chain ID (11567 — registered on chainlist.org).
/// Override via --chain-id for a separate testnet deployment.
pub const DEFAULT_CHAIN_ID: u64 = 11567;

/// Legacy alias for DEFAULT_CHAIN_ID (deprecated, use DEFAULT_CHAIN_ID instead)
pub const CHAIN_ID: u64 = DEFAULT_CHAIN_ID;

// ============================================================================
// Address: 20-byte Ethereum-style address (newtype wrapper for type safety)
// ============================================================================

/// 20-byte Ethereum-style address wrapped in a newtype for compile-time type safety.
/// Prevents accidental mixing with Hash types.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Address(pub [u8; 20]);

impl Address {
    /// Create a new Address from bytes
    pub const fn new(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Create a zero address
    pub const fn zero() -> Self {
        Self([0u8; 20])
    }

    /// Check if this is the zero address
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 20]
    }

    /// Convert to hex string with 0x prefix
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Parse from hex string (with or without 0x prefix)
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 40 {
            return Err(format!("Address must be 40 hex chars, got {}", s.len()));
        }
        let bytes = hex::decode(s).map_err(|e| format!("Invalid hex: {}", e))?;
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address(0x{})", hex::encode(self.0))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl From<[u8; 20]> for Address {
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl From<Address> for [u8; 20] {
    fn from(addr: Address) -> Self {
        addr.0
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Address {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Index<usize> for Address {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize as hex string with 0x prefix
        let hex = format!("0x{}", hex::encode(self.0));
        serializer.serialize_str(&hex)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        if s.len() != 40 {
            return Err(serde::de::Error::custom(format!(
                "Address must be 40 hex chars, got {}",
                s.len()
            )));
        }
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

// ============================================================================
// Hash: 32-byte hash (newtype wrapper for type safety)
// ============================================================================

/// 32-byte hash wrapped in a newtype for compile-time type safety.
/// Prevents accidental mixing with Address types.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// Create a new Hash from bytes
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Create a zero hash
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    /// Check if this is the zero hash
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Convert to hex string with 0x prefix
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Parse from hex string (with or without 0x prefix)
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 64 {
            return Err(format!("Hash must be 64 hex chars, got {}", s.len()));
        }
        let bytes = hex::decode(s).map_err(|e| format!("Invalid hex: {}", e))?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash(0x{})", hex::encode(self.0))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl From<[u8; 32]> for Hash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<Hash> for [u8; 32] {
    fn from(hash: Hash) -> Self {
        hash.0
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Deref for Hash {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Index<usize> for Hash {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize as hex string with 0x prefix
        let hex = format!("0x{}", hex::encode(self.0));
        serializer.serialize_str(&hex)
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        if s.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "Hash must be 64 hex chars, got {}",
                s.len()
            )));
        }
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

// ============================================================================
// Genesis Configuration
// ============================================================================

/// Genesis block configuration. Can be loaded from a TOML file or use defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisConfig {
    /// Chain identifier
    pub chain_id: u64,
    /// Genesis block timestamp (Unix seconds). 0 = use current time.
    pub timestamp: u64,
    /// Initial account allocations: address -> balance
    #[serde(default)]
    pub allocations: Vec<GenesisAllocation>,
}

/// Genesis allocation entry for pre-funding accounts at genesis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenesisAllocation {
    /// Hex-encoded address (with or without 0x prefix)
    pub address: String,
    /// Initial balance in base units
    pub balance: u128,
}

impl GenesisAllocation {
    /// Validate the allocation entry
    /// - Address must be valid hex (with or without 0x prefix), 40 hex chars
    /// - Balance must be > 0
    pub fn validate(&self) -> Result<(), String> {
        // Strip 0x prefix if present
        let addr_hex = self.address.strip_prefix("0x").unwrap_or(&self.address);

        // Validate address format (40 hex chars = 20 bytes)
        if addr_hex.len() != 40 {
            return Err(format!(
                "Invalid address length for '{}': expected 40 hex chars, got {}",
                self.address,
                addr_hex.len()
            ));
        }

        if !addr_hex.chars().all(|c: char| c.is_ascii_hexdigit()) {
            return Err(format!(
                "Invalid address format for '{}': must be hexadecimal",
                self.address
            ));
        }

        // Validate balance > 0
        if self.balance == 0 {
            return Err(format!(
                "Invalid balance for '{}': must be greater than 0",
                self.address
            ));
        }

        Ok(())
    }

    /// Get the normalized address (lowercase, without 0x prefix) for comparison
    pub fn normalized_address(&self) -> String {
        self.address
            .strip_prefix("0x")
            .unwrap_or(&self.address)
            .to_lowercase()
    }

    /// Parse address bytes from the hex string
    pub fn address_bytes(&self) -> Result<Address, String> {
        let addr_hex = self.address.strip_prefix("0x").unwrap_or(&self.address);

        if addr_hex.len() != 40 || !addr_hex.chars().all(|c: char| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid address format: {}", self.address));
        }

        let bytes =
            hex::decode(addr_hex).map_err(|e| format!("Failed to decode address hex: {}", e))?;

        let mut address = [0u8; 20];
        address.copy_from_slice(&bytes);
        Ok(Address(address))
    }
}

impl PartialOrd for GenesisAllocation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GenesisAllocation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by normalized address for deterministic ordering
        self.normalized_address().cmp(&other.normalized_address())
    }
}

impl Default for GenesisConfig {
    fn default() -> Self {
        Self {
            chain_id: DEFAULT_CHAIN_ID,
            timestamp: 1735689600, // January 1, 2026, 00:00:00 UTC
            allocations: vec![GenesisAllocation {
                address: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
                balance: 1000 * 1_000_000_000_000_000_000u128, // 1000 IDAG
            }],
        }
    }
}

impl GenesisConfig {
    /// Load from a TOML file path. Falls back to defaults if file doesn't exist.
    pub fn load_or_default(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to parse genesis config {}: {}, using defaults",
                    path.display(),
                    e
                );
                Self::default()
            }),
            Err(_) => {
                tracing::info!(
                    "No genesis config file at {}, using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }
}

// ============================================================================
// Mining Stream Types
// ============================================================================

/// Mining stream types for BraidCore architecture
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamType {
    /// Stream A: ASIC mining (Blake3, 10s blocks)
    StreamA,
    /// Stream B: CPU mining (B3MemHash, 5s blocks) — GPU via OpenCL planned
    StreamB,
    /// Stream C: ZK proofs (100ms blocks)
    StreamC,
}

impl StreamType {
    /// Convert stream type to bytes for hashing
    pub fn to_bytes(&self) -> [u8; 1] {
        match self {
            StreamType::StreamA => [0],
            StreamType::StreamB => [1],
            StreamType::StreamC => [2],
        }
    }
}

/// Mining difficulty
pub type Difficulty = u64;

// ============================================================================
// Helper Functions
// ============================================================================

/// Canonical keccak256 hash function used throughout the codebase.
/// This provides a single, consistent implementation for Ethereum-compatible hashing.
pub fn keccak256(data: &[u8]) -> Hash {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    Hash(output)
}

/// Derive an Ethereum-style address from a public key.
/// Returns the last 20 bytes of keccak256(public_key).
pub fn derive_eth_address(public_key: &[u8]) -> Address {
    let hash = keccak256(public_key);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash.0[12..32]);
    Address(addr)
}

/// Parse a hex string to Address (convenience function for RPC)
pub fn hex_to_address(s: &str) -> Result<Address, String> {
    Address::from_hex(s)
}

/// Parse a hex string to Hash (convenience function for RPC)
pub fn hex_to_hash(s: &str) -> Result<Hash, String> {
    Hash::from_hex(s)
}
