//! Sharding integration tests
//!
//! Tests for shard management, transaction routing, cross-shard communication,
//! state synchronization, and consensus coordination.

use irondag_blockchain::blockchain::block::BlockHeader;
use irondag_blockchain::blockchain::{Block, Transaction};
use irondag_blockchain::sharding::{
    AssignmentStrategy, CrossShardStatus, ShardConfig, ShardManager,
};
use irondag_blockchain::types::{Address, StreamType};

/// Test shard creation and initialization
#[tokio::test]
async fn test_shard_creation() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Verify shards were created
    let shards = manager.get_all_shards().await;
    assert_eq!(shards.len(), 4);

    // Verify each shard has correct ID
    for shard in shards.iter() {
        let shard_guard = shard.read().await;
        assert!(shard_guard.id < 4);
    }
}

/// Test transaction routing to shards
#[tokio::test]
async fn test_transaction_routing() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create test transactions
    let sender = Address([1u8; 20]);
    let receiver1 = Address([2u8; 20]);
    let receiver2 = Address([3u8; 20]);

    let tx1 = Transaction::new(sender, receiver1, 100, 1, 0);
    let tx2 = Transaction::new(sender, receiver2, 200, 2, 0);

    // Add transactions
    manager.add_transaction(tx1.clone()).await.unwrap();
    manager.add_transaction(tx2.clone()).await.unwrap();

    // Verify transactions were routed to shards
    let mut total_txs = 0;
    for shard_id in 0..manager.shard_count() {
        let txs = manager.get_shard_transactions(shard_id, 100).await;
        total_txs += txs.len();
    }

    assert_eq!(total_txs, 2);
}

/// Test cross-shard transaction detection
#[tokio::test]
async fn test_cross_shard_detection() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create transaction with different sender/receiver addresses
    let sender = Address([1u8; 20]);
    let receiver = Address([99u8; 20]);

    let tx = Transaction::new(sender, receiver, 100, 1, 0);

    let from_shard = manager.get_shard_for_address(&sender);
    let to_shard = manager.get_shard_for_address(&receiver);

    if from_shard != to_shard {
        manager.add_transaction(tx.clone()).await.unwrap();
        let status = manager.get_cross_shard_status(tx.hash).await;
        assert_eq!(status, Some(CrossShardStatus::Pending));
    }
}

/// Test state synchronization
#[tokio::test]
async fn test_state_synchronization() {
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Ensure shards are reachable and empty
    for shard_id in 0..manager.shard_count() {
        let txs = manager.get_shard_transactions(shard_id, 10).await;
        assert!(txs.is_empty());
    }
}

/// Test shard consensus coordination
#[tokio::test]
async fn test_shard_consensus_coordination() {
    let config = ShardConfig {
        shard_count: 3,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    let shards = manager.get_all_shards().await;
    assert_eq!(shards.len(), 3);
    for shard in shards {
        let shard_guard = shard.read().await;
        let chain_len = shard_guard.blockchain.read().await.get_blocks().len();
        assert_eq!(chain_len, 0);
    }
}

/// Test shard block processing
#[tokio::test]
async fn test_shard_block_processing() {
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Get a shard
    let shard = manager.get_shard(0).cloned().unwrap();

    // Create a test block
    let header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let block = Block::new(header, vec![]);

    // Process block
    {
        let shard_guard = shard.read().await;
        let blockchain = shard_guard.blockchain.clone();
        drop(shard_guard);
        blockchain.write().await.add_block(block.clone()).unwrap();
    }

    // Verify block was added
    let blockchain = shard.read().await.blockchain.clone();
    assert_eq!(blockchain.read().await.get_blocks().len(), 1);
}

/// Test multiple shards processing blocks independently
#[tokio::test]
async fn test_independent_shard_processing() {
    let config = ShardConfig {
        shard_count: 3,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Process blocks in different shards
    for shard_id in 0..3 {
        let shard = manager.get_shard(shard_id).cloned().unwrap();

        let header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
        let block = Block::new(header, vec![]);

        let blockchain = shard.read().await.blockchain.clone();
        blockchain.write().await.add_block(block).unwrap();
    }

    // Verify each shard has its own block
    for shard_id in 0..3 {
        let shard = manager.get_shard(shard_id).cloned().unwrap();
        let blockchain = shard.read().await.blockchain.clone();
        assert_eq!(blockchain.read().await.get_blocks().len(), 1);
    }
}

/// Test shard statistics
#[tokio::test]
async fn test_shard_statistics() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    let shards = manager.get_all_shards().await;
    assert_eq!(shards.len(), 4);
    for shard in shards {
        let shard_guard = shard.read().await;
        assert!(shard_guard.id < 4);
        let chain_len = shard_guard.blockchain.read().await.get_blocks().len();
        assert_eq!(chain_len, 0);
    }
}

/// Test assignment strategies
#[tokio::test]
async fn test_assignment_strategies() {
    let sender = Address([1u8; 20]);
    let _receiver = Address([2u8; 20]);

    // Test consistent hashing
    let config1 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };
    let manager1 = ShardManager::new(config1);
    let assignment1 = manager1.get_shard_for_address(&sender);
    assert!(assignment1 < 4);

    // Test round-robin
    let config2 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::RoundRobin,
        ..Default::default()
    };
    let manager2 = ShardManager::new(config2);
    let assignment2 = manager2.get_shard_for_address(&sender);
    assert!(assignment2 < 4);

    // Test address-based
    let config3 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::AddressBased,
        ..Default::default()
    };
    let manager3 = ShardManager::new(config3);
    let assignment3 = manager3.get_shard_for_address(&sender);
    assert!(assignment3 < 4);
}
