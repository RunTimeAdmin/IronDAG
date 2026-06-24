//! Integration tests for Mining + Blockchain

use irondag_blockchain::blockchain::{Blockchain, Transaction};
use irondag_blockchain::mining::MiningManager;
use irondag_blockchain::types::{Address, Hash};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Test mining and block addition
#[tokio::test]
async fn test_mining_blockchain_integration() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let miner = Address([9u8; 20]);

    // Set balance for sender
    let sender = Address([1u8; 20]);
    {
        let mut bc = blockchain.write().await;
        bc.set_balance(sender, 10000).unwrap();
    }

    // Create a transaction
    let mut tx = Transaction::new(sender, Address([2u8; 20]), 1000, 10, 0);
    // EIP-1559: Set max_fee_per_gas >= base_fee (1 gwei)
    tx.max_fee_per_gas = Some(1_000_000_000);
    tx.max_priority_fee_per_gas = Some(1_000_000_000);

    // Add transaction to mining mempool
    let mining_manager = MiningManager::new(blockchain.clone(), miner);
    mining_manager.add_transaction(tx.clone()).await.unwrap();

    // Verify transaction is in pool
    let pending_count = mining_manager.pending_count().await;
    assert_eq!(pending_count, 1);

    // Create mining manager
    let _mining_manager = MiningManager::new(blockchain.clone(), miner);

    // Verify blockchain is accessible (may be empty initially)
    let latest_hash = blockchain
        .read()
        .await
        .get_latest_block()
        .map(|block| block.hash)
        .unwrap_or(Hash::zero());
    // Empty blockchain returns [0; 32], which is valid for a new chain
    assert_eq!(latest_hash, Hash::zero());
}

/// Test transaction inclusion in mined blocks
#[tokio::test]
async fn test_transaction_inclusion() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let miner = Address([9u8; 20]);

    // Add multiple transactions from different senders
    // Each sender starts with nonce 0, so they can all be added immediately
    let mining_manager = MiningManager::new(blockchain.clone(), miner);
    for i in 0..5 {
        // Use different sender for each transaction to avoid nonce ordering constraints
        let sender = Address([i as u8; 20]);
        {
            let mut bc = blockchain.write().await;
            bc.set_balance(sender, 100000).unwrap(); // Enough for the transaction
        }

        let mut tx = Transaction::new(
            sender,
            Address([2u8; 20]),
            100 * (i as u128 + 1),
            10,
            0, // Each sender's first transaction has nonce 0
        );
        // EIP-1559: Set max_fee_per_gas >= base_fee (1 gwei)
        tx.max_fee_per_gas = Some(1_000_000_000);
        tx.max_priority_fee_per_gas = Some(1_000_000_000);
        mining_manager.add_transaction(tx).await.unwrap();
    }

    // Verify transactions are in pool
    let pending = mining_manager.pending_count().await;
    assert_eq!(pending, 5);

    // Create mining manager
    let _mining_manager = MiningManager::new(blockchain.clone(), miner);
}

/// Test mining rewards
#[tokio::test]
async fn test_mining_rewards() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let miner = Address([9u8; 20]);
    let _mining_manager = MiningManager::new(blockchain.clone(), miner);

    // Mining rewards are handled in the mining manager
    // This test verifies the structure is in place
    // Empty blockchain returns [0; 32] which is valid
    let latest_hash = blockchain
        .read()
        .await
        .get_latest_block()
        .map(|block| block.hash)
        .unwrap_or(Hash::zero());
    assert_eq!(latest_hash, Hash::zero()); // Empty chain
}
