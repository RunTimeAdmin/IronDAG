//! Error types for the blockchain
//!
//! Provides structured error handling with custom error types
//! for better error reporting and debugging.

use thiserror::Error;

/// Main blockchain error type
// ERR-003: Removed Clone derive since std::io::Error doesn't implement Clone
#[derive(Error, Debug)]
pub enum BlockchainError {
    #[error("Invalid block: {0}")]
    InvalidBlock(String),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("EVM error: {0}")]
    Evm(String),

    #[error("Consensus error: {0}")]
    Consensus(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Configuration error: {0}")]
    Config(String),

    /// ERR-003: Preserves std::io::Error to keep ErrorKind information
    #[error("IO error: {0}")]
    Io(std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unknown error: {0}")]
    Unknown(String),

    #[error("Database version corrupted: {0}")]
    CorruptedVersion(String),

    #[error("Database version {found} is newer than supported version {supported}")]
    FutureVersion { found: u32, supported: u32 },

    #[error("Database migration from v{from} to v{to} failed: {reason}")]
    MigrationFailed { from: u32, to: u32, reason: String },
}

/// Block validation error types for detailed failure reporting
#[derive(Error, Debug, PartialEq, Eq)]
pub enum BlockValidationError {
    #[error("Invalid transaction root: expected {expected}, computed {computed}")]
    InvalidTxRoot { expected: String, computed: String },

    #[error("Unknown parent hash: {0}")]
    UnknownParent(String),

    #[error("No parents specified for non-genesis block")]
    NoParents,

    #[error("Duplicate transaction: {0}")]
    DuplicateTransaction(String),

    #[error("Block timestamp too far in future: {timestamp} (current: {current}, max_future: {max_future})")]
    TimestampTooFarInFuture {
        timestamp: u64,
        current: u64,
        max_future: u64,
    },

    #[error("Block timestamp too old: {timestamp} (minimum: {minimum})")]
    TimestampTooOld { timestamp: u64, minimum: u64 },

    #[error("Invalid block number: {block_number} (expected: {expected})")]
    InvalidBlockNumber { block_number: u64, expected: u64 },

    #[error("Block exceeds maximum transaction count: {count} (max: {max})")]
    MaxTransactionsExceeded { count: usize, max: usize },

    #[error("Block size exceeds maximum: {size} bytes (max: {max} bytes)")]
    BlockSizeExceeded { size: usize, max: usize },

    #[error("Parent hash count exceeds maximum: {count} (max: {max})")]
    TooManyParentHashes { count: usize, max: usize },

    #[error("Block hash mismatch: expected {expected}, computed {computed}")]
    InvalidBlockHash { expected: String, computed: String },

    #[error("Genesis block must be first")]
    GenesisNotFirst,

    #[error("Block already exists")]
    DuplicateBlock,
}

impl From<std::io::Error> for BlockchainError {
    fn from(err: std::io::Error) -> Self {
        // ERR-003: Preserve the full io::Error to keep ErrorKind
        BlockchainError::Io(err)
    }
}

impl From<bincode::Error> for BlockchainError {
    fn from(err: bincode::Error) -> Self {
        BlockchainError::Serialization(err.to_string())
    }
}

impl From<String> for BlockchainError {
    fn from(err: String) -> Self {
        BlockchainError::Unknown(err)
    }
}

impl From<sled::Error> for BlockchainError {
    fn from(err: sled::Error) -> Self {
        BlockchainError::Storage(err.to_string())
    }
}

/// Result type alias for blockchain operations
pub type BlockchainResult<T> = Result<T, BlockchainError>;
