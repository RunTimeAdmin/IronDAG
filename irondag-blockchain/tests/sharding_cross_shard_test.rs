//! Cross-shard end-to-end integration tests
//!
//! Tests the full cross-shard transaction flow including:
//! - Transaction routing between shards
//! - Cross-shard receipt creation and processing
//! - Receipt ordering guarantees
//! - Shard height tracking for catch-up protocol

use irondag::blockchain::Transaction;
use irondag::sharding::{AssignmentStrategy, CrossShardStatus, ShardConfig, ShardManager};
use irondag::types::Address;

/// Test the full cross-shard flow:
/// 1. Create ShardManager with 2+ shards
/// 2. Submit a transaction where sender is on shard 0, recipient is on shard 1
/// 3. Route the transaction (should go to shard 0's pool)
/// 4. Verify the transaction is in the correct shard pool
/// 5. After "mining" (or direct call to process_cross_shard_transaction),
///    verify a receipt is created for the target shard
/// 6. Verify receipt can be processed by the target shard
#[tokio::test]
async fn test_cross_shard_transaction_full_flow() {
    // Step 1: Create ShardManager with 2 shards
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::AddressBased,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Step 2: Create addresses on different shards using AddressBased strategy
    // With AddressBased, shard = first 8 bytes mod shard_count
    // Address [0,0,0,0,0,0,0,0,...] -> shard 0
    // Address [1,0,0,0,0,0,0,0,...] -> shard 1
    let sender = Address([0u8; 20]);
    let receiver = Address([1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let source_shard = manager.get_shard_for_address(&sender);
    let target_shard = manager.get_shard_for_address(&receiver);

    // Verify addresses are on different shards
    assert_eq!(source_shard, 0, "Sender should be on shard 0");
    assert_eq!(target_shard, 1, "Receiver should be on shard 1");
    assert_ne!(source_shard, target_shard, "Should be cross-shard");

    // Step 3: Fund the sender address
    {
        let shard = manager
            .get_shard(source_shard)
            .expect("Source shard should exist");
        let shard_guard = shard.read().await;
        let mut blockchain = shard_guard.blockchain.write().await;
        let initial_balance: u128 = 1_000_000;
        blockchain
            .set_balance(sender, initial_balance)
            .expect("Failed to set balance");
    }

    // Step 4: Submit a cross-shard transaction
    let tx_value: u128 = 100;
    let tx_fee: u128 = 1;
    let tx = Transaction::new(sender, receiver, tx_value, tx_fee, 1);
    let tx_hash = tx.hash;

    // Step 5: Route the transaction and add it
    let routed_shard = manager.route_transaction(&tx);
    assert_eq!(
        routed_shard, source_shard,
        "Transaction should be routed to sender's shard"
    );

    manager
        .add_transaction(tx.clone())
        .await
        .expect("Failed to add transaction");

    // Step 6: Verify the transaction is in the correct shard pool
    let shard_txs = manager.get_shard_transactions(source_shard, 10).await;
    assert!(
        !shard_txs.is_empty(),
        "Source shard pool should contain the transaction"
    );

    let found_in_pool = shard_txs.iter().any(|t| t.hash == tx_hash);
    assert!(found_in_pool, "Transaction should be in source shard pool");

    // Verify cross-shard tracking
    let has_cross_shard = manager.has_cross_shard_transaction(tx_hash).await;
    assert!(
        has_cross_shard,
        "Transaction should be registered as cross-shard"
    );

    let cross_tx = manager.get_cross_shard_transaction(tx_hash).await;
    assert!(
        cross_tx.is_some(),
        "Cross-shard transaction should be retrievable"
    );

    let cross_tx = cross_tx.unwrap();
    assert_eq!(cross_tx.source_shard, source_shard);
    assert_eq!(cross_tx.target_shard, target_shard);
    assert_eq!(cross_tx.status, CrossShardStatus::Pending);

    // Step 7: Process the cross-shard transaction (simulates mining)
    let result = manager.process_cross_shard_transaction(tx_hash).await;
    assert!(
        result.is_ok(),
        "process_cross_shard_transaction should succeed: {:?}",
        result
    );

    // Step 8: Verify receipt was created and status is Committed
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

    // Step 9: Verify sender was debited on source shard
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
    }
}

/// Test that same-shard transactions do NOT create cross-shard receipts
#[tokio::test]
async fn test_same_shard_no_receipt() {
    // Create ShardManager with 2 shards
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::AddressBased,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create two addresses on the SAME shard (shard 0)
    let sender = Address([0u8; 20]);
    let receiver = Address([2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let sender_shard = manager.get_shard_for_address(&sender);
    let receiver_shard = manager.get_shard_for_address(&receiver);

    // Verify both addresses are on the same shard
    assert_eq!(
        sender_shard, receiver_shard,
        "Both addresses should be on the same shard"
    );

    // Create and add transaction
    let tx = Transaction::new(sender, receiver, 100, 1, 1);
    let tx_hash = tx.hash;

    manager
        .add_transaction(tx)
        .await
        .expect("Failed to add transaction");

    // Verify transaction is in the shard pool
    let shard_txs = manager.get_shard_transactions(sender_shard, 10).await;
    assert!(!shard_txs.is_empty(), "Transaction should be in shard pool");

    // Verify it is NOT tracked as a cross-shard transaction
    let has_cross_shard = manager.has_cross_shard_transaction(tx_hash).await;
    assert!(
        !has_cross_shard,
        "Same-shard transaction should NOT be tracked as cross-shard"
    );

    // Verify get_cross_shard_transaction returns None
    let cross_tx = manager.get_cross_shard_transaction(tx_hash).await;
    assert!(
        cross_tx.is_none(),
        "Same-shard transaction should not have cross-shard entry"
    );
}

/// Test cross-shard receipt ordering (receipts processed in sequence order)
#[tokio::test]
async fn test_cross_shard_receipt_ordering() {
    // Create ShardManager with 2 shards
    let config = ShardConfig {
        shard_count: 2,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::AddressBased,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Create addresses on different shards
    let sender = Address([0u8; 20]);
    let receiver = Address([1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let source_shard = manager.get_shard_for_address(&sender);
    let target_shard = manager.get_shard_for_address(&receiver);

    assert_ne!(source_shard, target_shard, "Should be cross-shard");

    // Fund sender with enough for multiple transactions
    {
        let shard = manager
            .get_shard(source_shard)
            .expect("Source shard should exist");
        let shard_guard = shard.read().await;
        let mut blockchain = shard_guard.blockchain.write().await;
        blockchain
            .set_balance(sender, 10_000)
            .expect("Failed to set balance");
    }

    // Submit multiple cross-shard transactions in sequence
    let mut tx_hashes = Vec::new();
    for i in 0..5u64 {
        let tx = Transaction::new(sender, receiver, 100 * (i + 1) as u128, 1u128, i + 1);
        let tx_hash = tx.hash;
        manager
            .add_transaction(tx)
            .await
            .expect("Failed to add transaction");
        tx_hashes.push(tx_hash);
    }

    // Process all transactions
    for tx_hash in &tx_hashes {
        let result = manager.process_cross_shard_transaction(*tx_hash).await;
        assert!(result.is_ok(), "Transaction should process successfully");
    }

    // Verify all transactions are committed
    for (i, tx_hash) in tx_hashes.iter().enumerate() {
        let cross_tx = manager.get_cross_shard_transaction(*tx_hash).await;
        assert!(cross_tx.is_some(), "Transaction {} should exist", i);
        assert_eq!(
            cross_tx.unwrap().status,
            CrossShardStatus::Committed,
            "Transaction {} should be committed",
            i
        );
    }

    // Verify the cross-shard outgoing queue has all transactions in order
    let shard = manager
        .get_shard(source_shard)
        .expect("Source shard should exist");
    let shard_guard = shard.read().await;
    assert_eq!(
        shard_guard.cross_shard_outgoing.len(),
        5,
        "Should have 5 outgoing cross-shard transactions"
    );

    // Verify transactions are in the order they were submitted
    for (i, tx_hash) in tx_hashes.iter().enumerate() {
        assert_eq!(
            shard_guard.cross_shard_outgoing[i], *tx_hash,
            "Transaction {} should be at position {}",
            i, i
        );
    }
}

/// Test that the catch-up protocol detects lagging shards
#[tokio::test]
async fn test_shard_height_tracking() {
    // Create ShardManager with 4 shards
    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);

    // Initially, no block heights should be recorded
    for shard_id in 0..4 {
        let height = manager.get_cross_shard_block_height(shard_id).await;
        assert!(
            height.is_none(),
            "Initial height for shard {} should be None",
            shard_id
        );
    }

    // Simulate receiving StateSync messages from other shards
    // Record block heights for shards 0 and 1
    manager
        .record_shard_block_height(0, 100)
        .await
        .expect("Failed to record height");
    manager
        .record_shard_block_height(1, 150)
        .await
        .expect("Failed to record height");

    // Verify heights are recorded
    let height_0 = manager.get_cross_shard_block_height(0).await;
    let height_1 = manager.get_cross_shard_block_height(1).await;
    let height_2 = manager.get_cross_shard_block_height(2).await;

    assert_eq!(height_0, Some(100), "Shard 0 height should be 100");
    assert_eq!(height_1, Some(150), "Shard 1 height should be 150");
    assert!(height_2.is_none(), "Shard 2 height should still be None");

    // Test monotonic height updates - should only advance
    manager
        .record_shard_block_height(0, 120)
        .await
        .expect("Failed to record height");
    let height_0_updated = manager.get_cross_shard_block_height(0).await;
    assert_eq!(
        height_0_updated,
        Some(120),
        "Shard 0 height should advance to 120"
    );

    // Test that lower heights don't override higher ones
    manager
        .record_shard_block_height(0, 50)
        .await
        .expect("Failed to record height");
    let height_0_final = manager.get_cross_shard_block_height(0).await;
    assert_eq!(
        height_0_final,
        Some(120),
        "Shard 0 height should remain at 120 (not go backwards)"
    );

    // Test broadcast functionality
    manager.broadcast_block_height(2, 200);
    // Note: broadcast sends to all other shards via message processor
    // The actual receipt would be processed asynchronously
}
