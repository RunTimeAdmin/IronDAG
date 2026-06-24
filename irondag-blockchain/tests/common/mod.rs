//! Common test utilities for IronDAG blockchain integration tests
//!
//! This module provides shared helper functions, types, and fixtures
//! for use across multiple integration test files.

// Placeholder for future shared test utilities
// Currently, tests are self-contained, but this module is available
// for extracting common patterns as the test suite grows.

/// Re-export commonly used types for convenience
pub use irondag::blockchain::{Block, BlockHeader, Blockchain, Transaction};
pub use irondag::types::{Address, StreamType};

/// Helper function to create a test address with a given byte value
pub fn test_address(byte: u8) -> Address {
    Address([byte; 20])
}

/// Helper function to create a genesis block for testing
pub fn create_genesis_block() -> Block {
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    Block::new(genesis_header, vec![])
}

/// Helper function to create a test transaction
pub fn create_test_transaction(
    sender: Address,
    recipient: Address,
    value: u128,
    fee: u128,
    nonce: u64,
) -> Transaction {
    Transaction::new(sender, recipient, value, fee, nonce)
}
