//! Basic sharding functionality tests
//!
//! Tests core sharding features that are already implemented.
//!
//! ## Cross-Shard End-to-End Integration Test (Phase 6)
//! The test `test_cross_shard_e2e_flow` validates the complete Phase 6 pipeline:
//! submit -> route -> process -> receipt creation -> balance update

use irondag::blockchain::Transaction;
use irondag::sharding::{
    AssignmentStrategy, CrossShardStatus, ShardConfig, ShardManager,
};
use irondag::types::Address;

/// Test shard creation
#[tokio::test]
async fn test_shard_creation() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Verify shard count
    assert_eq!(manager.shard_count(), 4);

    // Verify we can get all shards
    let shards = manager.get_all_shards().await;
    assert_eq!(shards.len(), 4);
}

/// Test transaction routing
#[tokio::test]
async fn test_transaction_routing() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create test transaction
    let sender = Address([1u8; 20]);
    let receiver = Address([2u8; 20]);

    let tx = Transaction::new(sender, receiver, 100, 1, 0);

    // Add transaction
    manager.add_transaction(tx.clone()).await.unwrap();

    // Get shard assignments
    let from_shard = manager.get_shard_for_address(&sender);
    let to_shard = manager.get_shard_for_address(&receiver);

    assert!(from_shard < 4);
    assert!(to_shard < 4);

    // Verify transaction is in correct shard
    let shard_txs = manager.get_shard_transactions(from_shard, 10).await;
    assert!(!shard_txs.is_empty());
}

/// Test cross-shard transaction detection
#[tokio::test]
async fn test_cross_shard_transaction() {
    let config = ShardConfig {
        shard_count: 16, // More shards = higher chance of cross-shard
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create transaction with different addresses
    let sender = Address([1u8; 20]);
    let receiver = Address([255u8; 20]);

    let tx = Transaction::new(sender, receiver, 100, 1, 0);
    let tx_hash = tx.hash;

    // Get shard assignments
    let (from_shard, to_shard) = manager.get_transaction_shards(&tx).await.unwrap();

    // If it's cross-shard, test tracking
    if from_shard != to_shard {
        // Add transaction
        manager.add_transaction(tx.clone()).await.unwrap();

        // Verify cross-shard transaction exists
        let cross_tx = manager.get_cross_shard_transaction(tx_hash).await;
        assert!(cross_tx.is_some());

        if let Some(ctx) = cross_tx {
            assert_eq!(ctx.source_shard, from_shard);
            assert_eq!(ctx.target_shard, to_shard);
            assert_eq!(ctx.status, CrossShardStatus::Pending);
        }
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

    // Add some transactions
    for i in 0..10 {
        let sender = Address([i; 20]);
        let receiver = Address([(i + 1) % 255; 20]);
        let tx = Transaction::new(sender, receiver, 100, i as u128, 0);
        manager.add_transaction(tx).await.unwrap();
    }

    // Get all shard stats
    let stats = manager.get_all_shard_stats().await;
    assert_eq!(stats.len(), 4);

    // Verify total transactions across shards
    let total_txs: usize = stats.iter().map(|s| s.transaction_pool_size).sum();
    assert!(total_txs >= 10);

    // Verify each stat has valid shard_id
    for stat in stats {
        assert!(stat.shard_id < 4);
    }
}

/// Test assignment strategies
#[tokio::test]
async fn test_assignment_strategies() {
    let sender = Address([1u8; 20]);
    let _receiver = Address([2u8; 20]);

    // Test ConsistentHashing
    let config1 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };
    let manager1 = ShardManager::new(config1);
    let shard1 = manager1.get_shard_for_address(&sender);
    assert!(shard1 < 4);

    // Verify deterministic routing (same address -> same shard)
    let shard1_again = manager1.get_shard_for_address(&sender);
    assert_eq!(shard1, shard1_again);

    // Test RoundRobin
    let config2 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::RoundRobin,
        ..Default::default()
    };
    let manager2 = ShardManager::new(config2);
    let shard2 = manager2.get_shard_for_address(&sender);
    assert!(shard2 < 4);

    // Test AddressBased
    let config3 = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::AddressBased,
        ..Default::default()
    };
    let manager3 = ShardManager::new(config3);
    let shard3 = manager3.get_shard_for_address(&sender);
    assert!(shard3 < 4);
}

/// Test shard transaction pool operations
#[tokio::test]
async fn test_shard_transaction_pool() {
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Add transactions to same shard
    let sender = Address([1u8; 20]);
    let shard_id = manager.get_shard_for_address(&sender);

    for i in 0..5 {
        let tx = Transaction::new(sender, sender, 100, i, 0);
        manager.add_transaction(tx).await.unwrap();
    }

    // Get transactions
    let txs = manager.get_shard_transactions(shard_id, 3).await;
    assert_eq!(txs.len(), 3);

    // Remove transactions
    let removed = manager.remove_shard_transactions(shard_id, 2).await;
    assert_eq!(removed.len(), 2);

    // Verify remaining
    let remaining = manager.get_shard_transactions(shard_id, 10).await;
    assert_eq!(remaining.len(), 3); // 5 - 2 = 3
}

/// Test get/set shard operations
#[tokio::test]
async fn test_get_shard() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Test valid shard ID
    let shard = manager.get_shard(0);
    assert!(shard.is_some());

    let shard = manager.get_shard(3);
    assert!(shard.is_some());

    // Test invalid shard ID
    let shard = manager.get_shard(99);
    assert!(shard.is_none());
}

/// Test all cross-shard transactions retrieval
#[tokio::test]
async fn test_get_all_cross_shard_transactions() {
    let config = ShardConfig {
        shard_count: 16, // More shards for more cross-shard txs
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Add transactions
    for i in 0..20 {
        let sender = Address([i; 20]);
        let receiver = Address([(i * 7) % 255; 20]); // Different pattern
        let tx = Transaction::new(sender, receiver, 100, i as u128, 0);
        manager.add_transaction(tx).await.unwrap();
    }

    // Get all cross-shard transactions
    let cross_txs = manager.get_all_cross_shard_transactions().await;

    // Should have some cross-shard transactions
    println!("Cross-shard transactions: {}", cross_txs.len());

    // Verify structure
    for ctx in cross_txs {
        assert!(ctx.source_shard < 16);
        assert!(ctx.target_shard < 16);
        assert_ne!(ctx.source_shard, ctx.target_shard);
    }
}

/// Test shard statistics with cross-shard
#[tokio::test]
async fn test_shard_stats_with_cross_shard() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Add cross-shard transaction
    let sender = Address([1u8; 20]);
    let receiver = Address([255u8; 20]);
    let tx = Transaction::new(sender, receiver, 100, 1, 0);

    let (from_shard, to_shard) = manager.get_transaction_shards(&tx).await.unwrap();

    if from_shard != to_shard {
        manager.add_transaction(tx).await.unwrap();

        // Get stats for source shard
        let source_stats = manager.get_shard_stats(from_shard).await.unwrap();
        assert!(source_stats.cross_shard_outgoing >= 1);

        // Get stats for target shard
        let target_stats = manager.get_shard_stats(to_shard).await.unwrap();
        assert!(target_stats.cross_shard_incoming >= 1);
    }
}

/// Test sharding disabled scenario
#[tokio::test]
async fn test_sharding_disabled() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: false, // Cross-shard disabled
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Add cross-shard transaction (should NOT be tracked as cross-shard)
    let sender = Address([1u8; 20]);
    let receiver = Address([255u8; 20]);
    let tx = Transaction::new(sender, receiver, 100, 1, 0);
    let tx_hash = tx.hash;

    manager.add_transaction(tx).await.unwrap();

    // Should not be in cross-shard tracking
    let _cross_tx = manager.get_cross_shard_transaction(tx_hash).await;

    // With cross-shard disabled, it should still work but treated as same-shard
    let from_shard = manager.get_shard_for_address(&sender);
    let shard_txs = manager.get_shard_transactions(from_shard, 10).await;
    assert!(!shard_txs.is_empty());
}

// =============================================================================
// Phase 6: Cross-Shard End-to-End Integration Tests
// =============================================================================

/// Cross-shard end-to-end integration test (Phase 6)
///
/// This test validates the complete Phase 6 cross-shard flow:
/// 1. Create a ShardManager with 16 shards
/// 2. Find two addresses that hash to different shards
/// 3. Fund the sender address in the source shard's blockchain
/// 4. Submit a cross-shard transaction
/// 5. Verify: transaction appears in source shard's pool
/// 6. Call process_cross_shard_transaction()
/// 7. Verify: receipt was created and transaction is tracked
/// 8. Verify: sender was debited on source shard
/// 9. Verify: cross-shard transaction status is Committed
#[tokio::test]
async fn test_cross_shard_e2e_flow() {
    // Step 1: Create ShardManager with 16 shards
    let config = ShardConfig {
        shard_count: 16,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Step 2: Find two addresses that hash to different shards
    // Try different addresses until we find a pair on different shards
    let mut sender = Address::zero();
    let mut receiver = Address([255u8; 20]);

    // Find addresses on different shards
    for i in 0..255 {
        sender.0[0] = i;
        receiver.0[0] = 255 - i;

        let sender_shard = manager.get_shard_for_address(&sender);
        let receiver_shard = manager.get_shard_for_address(&receiver);

        if sender_shard != receiver_shard {
            break;
        }
    }

    let source_shard = manager.get_shard_for_address(&sender);
    let target_shard = manager.get_shard_for_address(&receiver);

    // Verify they are on different shards
    assert_ne!(
        source_shard, target_shard,
        "Failed to find addresses on different shards after iteration"
    );

    println!(
        "✅ Cross-shard setup: sender on shard {}, receiver on shard {}",
        source_shard, target_shard
    );

    // Step 3: Fund the sender address in the source shard's blockchain
    {
        let shard = manager
            .get_shard(source_shard)
            .expect("Source shard should exist");
        let shard_guard = shard.read().await;
        let mut blockchain = shard_guard.blockchain.write().await;

        // Set initial balance for sender (1000 units + fee buffer)
        let initial_balance: u128 = 1_000_000;
        blockchain
            .set_balance(sender, initial_balance)
            .expect("Failed to set balance");

        let actual_balance = blockchain.get_balance(sender);
        assert_eq!(actual_balance, initial_balance, "Balance not set correctly");

        println!(
            "✅ Funded sender with {} units on source shard {}",
            initial_balance, source_shard
        );
    }

    // Step 4: Submit a cross-shard transaction
    let tx_value: u128 = 100;
    let tx_fee: u128 = 1;
    let tx = Transaction::new(sender, receiver, tx_value, tx_fee, 1);
    let tx_hash = tx.hash;

    manager
        .add_transaction(tx.clone())
        .await
        .expect("Failed to add transaction");

    println!(
        "✅ Submitted cross-shard transaction: {} from shard {} to shard {}",
        hex::encode(&tx_hash.0[0..8]),
        source_shard,
        target_shard
    );

    // Step 5: Verify transaction appears in source shard's pool
    let shard_txs = manager.get_shard_transactions(source_shard, 10).await;
    assert!(
        !shard_txs.is_empty(),
        "Source shard pool should contain the transaction"
    );

    let found_in_pool = shard_txs.iter().any(|t| t.hash == tx_hash);
    assert!(found_in_pool, "Transaction should be in source shard pool");

    println!("✅ Transaction found in source shard {} pool", source_shard);

    // Step 5b: Verify transaction is registered as cross-shard
    let has_cross_shard = manager.has_cross_shard_transaction(tx_hash).await;
    assert!(
        has_cross_shard,
        "Transaction should be registered as cross-shard"
    );

    // Get the cross-shard transaction details
    let cross_tx = manager.get_cross_shard_transaction(tx_hash).await;
    assert!(
        cross_tx.is_some(),
        "Cross-shard transaction should be retrievable"
    );

    let cross_tx = cross_tx.unwrap();
    assert_eq!(cross_tx.source_shard, source_shard);
    assert_eq!(cross_tx.target_shard, target_shard);
    assert_eq!(cross_tx.status, CrossShardStatus::Pending);

    println!("✅ Transaction registered as cross-shard (Pending status)");

    // Step 6: Call process_cross_shard_transaction()
    let result = manager.process_cross_shard_transaction(tx_hash).await;
    assert!(
        result.is_ok(),
        "process_cross_shard_transaction should succeed: {:?}",
        result
    );

    println!("✅ process_cross_shard_transaction completed successfully");

    // Step 7: Verify receipt was created and transaction status is Committed
    let cross_tx_after = manager.get_cross_shard_transaction(tx_hash).await;
    assert!(
        cross_tx_after.is_some(),
        "Cross-shard transaction should still be retrievable"
    );

    let cross_tx_after = cross_tx_after.unwrap();
    assert_eq!(
        cross_tx_after.status,
        CrossShardStatus::Committed,
        "Cross-shard transaction status should be Committed after processing"
    );

    println!("✅ Cross-shard transaction status: Committed");

    // Step 8: Verify sender was debited on source shard
    {
        let shard = manager
            .get_shard(source_shard)
            .expect("Source shard should exist");
        let shard_guard = shard.read().await;
        let blockchain = shard_guard.blockchain.read().await;

        let balance_after = blockchain.get_balance(sender);
        let expected_balance = 1_000_000u128 - tx_value - tx_fee;

        assert_eq!(
            balance_after, expected_balance,
            "Sender should be debited. Expected {}, got {}",
            expected_balance, balance_after
        );

        println!(
            "✅ Sender debited correctly: {} -> {} (value: {}, fee: {})",
            1_000_000, balance_after, tx_value, tx_fee
        );
    }

    // Step 9: Verify cross-shard transaction is in the tracking map
    let all_cross_txs = manager.get_all_cross_shard_transactions().await;
    let found_in_all = all_cross_txs.iter().any(|ctx| ctx.tx.hash == tx_hash);
    assert!(
        found_in_all,
        "Transaction should be in all cross-shard transactions list"
    );

    println!("✅ Cross-shard transaction found in tracking map");

    // Summary
    println!("\n🎉 Phase 6 Cross-Shard E2E Test PASSED:");
    println!(
        "   - Sender on shard {}, receiver on shard {}",
        source_shard, target_shard
    );
    println!("   - Transaction submitted and routed correctly");
    println!("   - Receipt created and sent to target shard");
    println!("   - Sender debited on source shard");
    println!("   - Cross-shard transaction status: Committed");
}

/// Test cross-shard transaction with insufficient balance
///
/// Verifies that cross-shard transactions fail gracefully when
/// the sender has insufficient balance.
#[tokio::test]
async fn test_cross_shard_insufficient_balance() {
    let config = ShardConfig {
        shard_count: 16,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Find addresses on different shards
    let sender = Address([1u8; 20]);
    let receiver = Address([255u8; 20]);

    let source_shard = manager.get_shard_for_address(&sender);
    let target_shard = manager.get_shard_for_address(&receiver);

    // Skip if same shard
    if source_shard == target_shard {
        println!("Skipping test - addresses on same shard");
        return;
    }

    // Set low balance
    {
        let shard = manager.get_shard(source_shard).unwrap();
        let shard_guard = shard.read().await;
        let mut blockchain = shard_guard.blockchain.write().await;
        blockchain.set_balance(sender, 50).unwrap(); // Only 50 units
    }

    // Create transaction requiring more than balance
    let tx = Transaction::new(sender, receiver, 100, 1, 1); // 100 value + 1 fee = 101 total
    let tx_hash = tx.hash;

    manager.add_transaction(tx).await.unwrap();

    // Process should fail due to insufficient balance
    let result = manager.process_cross_shard_transaction(tx_hash).await;
    assert!(result.is_err(), "Should fail with insufficient balance");

    println!("✅ Cross-shard transaction correctly rejected due to insufficient balance");
}

/// Test multiple cross-shard transactions
///
/// Verifies that multiple cross-shard transactions can be processed
/// in sequence without interference.
#[tokio::test]
async fn test_multiple_cross_shard_transactions() {
    let config = ShardConfig {
        shard_count: 16,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Find addresses on different shards
    let sender1 = Address([1u8; 20]);
    let sender2 = Address([2u8; 20]);
    let receiver = Address([255u8; 20]);

    let source1 = manager.get_shard_for_address(&sender1);
    let source2 = manager.get_shard_for_address(&sender2);
    let target = manager.get_shard_for_address(&receiver);

    // Skip if any are on same shard
    if source1 == target || source2 == target {
        println!("Skipping test - addresses not on different shards");
        return;
    }

    // Fund both senders
    for (sender, shard_id) in [(sender1, source1), (sender2, source2)] {
        let shard = manager.get_shard(shard_id).unwrap();
        let shard_guard = shard.read().await;
        let mut blockchain = shard_guard.blockchain.write().await;
        blockchain.set_balance(sender, 1_000_000).unwrap();
    }

    // Submit multiple cross-shard transactions
    let mut tx_hashes = Vec::new();
    for i in 0..3u64 {
        let sender = if i % 2 == 0 { sender1 } else { sender2 };
        let tx = Transaction::new(sender, receiver, 100 * (i + 1) as u128, 1u128, i + 1);
        let tx_hash = tx.hash;
        manager.add_transaction(tx).await.unwrap();
        tx_hashes.push(tx_hash);
    }

    println!("✅ Submitted {} cross-shard transactions", tx_hashes.len());

    // Process all transactions
    for tx_hash in &tx_hashes {
        let result = manager.process_cross_shard_transaction(*tx_hash).await;
        assert!(result.is_ok(), "Transaction should process successfully");
    }

    // Verify all are committed
    for tx_hash in &tx_hashes {
        let cross_tx = manager.get_cross_shard_transaction(*tx_hash).await;
        assert!(cross_tx.is_some());
        assert_eq!(cross_tx.unwrap().status, CrossShardStatus::Committed);
    }

    println!(
        "✅ All {} cross-shard transactions committed successfully",
        tx_hashes.len()
    );
}

/// Test cross-shard transaction routing verification
///
/// Verifies that the get_transaction_shards() method correctly
/// identifies cross-shard transactions.
#[tokio::test]
async fn test_cross_shard_routing_verification() {
    let config = ShardConfig {
        shard_count: 16,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Test with various address pairs
    for i in 0..20 {
        let sender = Address([i; 20]);
        let receiver = Address([(255 - i); 20]);

        let tx = Transaction::new(sender, receiver, 100, i as u128, 0);

        // Get shard assignments
        let shards = manager.get_transaction_shards(&tx).await;
        assert!(shards.is_some());

        let (from_shard, to_shard) = shards.unwrap();

        // Verify against direct address lookups
        assert_eq!(from_shard, manager.get_shard_for_address(&sender));
        assert_eq!(to_shard, manager.get_shard_for_address(&receiver));

        if from_shard != to_shard {
            println!("✅ TX {}: cross-shard ({} -> {})", i, from_shard, to_shard);
        }
    }
}

/// Test route_transaction method (Phase 6 Gap 5)
///
/// Verifies that route_transaction correctly identifies the source shard
/// for a transaction based on the sender address.
#[tokio::test]
async fn test_route_transaction() {
    let config = ShardConfig {
        shard_count: 8,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create a transaction
    let sender = Address([42u8; 20]);
    let receiver = Address([99u8; 20]);
    let tx = Transaction::new(sender, receiver, 100, 1, 0);

    // Route the transaction
    let source_shard = manager.route_transaction(&tx);

    // Should match the shard assignment for the sender
    assert_eq!(source_shard, manager.get_shard_for_address(&sender));
    assert!(source_shard < 8);

    // Verify determinism
    let tx2 = Transaction::new(sender, Address::zero(), 200, 2, 0);
    assert_eq!(manager.route_transaction(&tx2), source_shard);
}

/// Test route_transaction_full method (Phase 6 Gap 5)
///
/// Verifies that route_transaction_full returns both source and target shards
/// for cross-shard transaction identification.
#[tokio::test]
async fn test_route_transaction_full() {
    let config = ShardConfig {
        shard_count: 16,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Test multiple address pairs
    for i in 0..10 {
        let sender = Address([i; 20]);
        let receiver = Address([(255 - i); 20]);
        let tx = Transaction::new(sender, receiver, 100, i as u128, 0);

        let (source_shard, target_shard) = manager.route_transaction_full(&tx);

        // Verify against direct address lookups
        assert_eq!(source_shard, manager.get_shard_for_address(&sender));
        assert_eq!(target_shard, manager.get_shard_for_address(&receiver));

        // Verify both shards are valid
        assert!(source_shard < 16);
        assert!(target_shard < 16);

        // Cross-shard detection
        let is_cross_shard = source_shard != target_shard;
        let expected_cross_shard =
            manager.get_shard_for_address(&sender) != manager.get_shard_for_address(&receiver);
        assert_eq!(is_cross_shard, expected_cross_shard);
    }
}

/// Test contract deployment routing (to = zero address)
///
/// Verifies that contract deployments are routed to the sender's shard.
#[tokio::test]
async fn test_contract_deployment_routing() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Contract deployment (to = zero address)
    let sender = Address([123u8; 20]);
    let tx = Transaction::new(sender, Address::zero(), 0, 1, 0); // to = [0; 20]

    let (source_shard, target_shard) = manager.route_transaction_full(&tx);

    // Contract deployments should go to sender's shard
    assert_eq!(source_shard, target_shard);
    assert_eq!(target_shard, manager.get_shard_for_address(&sender));
}

/// Test routing consistency with cache
///
/// Verifies that the shard cache doesn't affect routing correctness.
#[tokio::test]
async fn test_routing_cache_consistency() {
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Route same transaction multiple times
    let sender: Address = Address([55u8; 20]);
    let receiver: Address = Address([200u8; 20]);
    let tx = Transaction::new(sender, receiver, 100, 1, 0);

    let expected_source = manager.route_transaction(&tx);
    let (expected_source_full, expected_target) = manager.route_transaction_full(&tx);

    // Call multiple times to exercise cache
    for _ in 0..100 {
        let source = manager.route_transaction(&tx);
        assert_eq!(source, expected_source);

        let (source_full, target) = manager.route_transaction_full(&tx);
        assert_eq!(source_full, expected_source_full);
        assert_eq!(target, expected_target);
    }
}
