//! Integration tests for IronDAG blockchain components

// TST-006: Common test utilities module
pub mod common;

mod blockchain_consensus;
mod end_to_end;
mod mining_blockchain;
mod network_blockchain;
mod rpc_auth_rate_limit;
mod sharding_e2e;
mod sharding_integration;
mod storage_blockchain;
mod transaction_pool;
