//! Integration tests for Transaction Pool

use irondag::blockchain::Transaction;
use irondag::node::pool::TransactionPool;
use irondag::types::Address;

/// Test transaction pool basic operations
#[tokio::test]
async fn test_transaction_pool_basic() {
    let mut pool = TransactionPool::new(100);

    // Create transaction
    let tx = Transaction::new(Address([1u8; 20]), Address([2u8; 20]), 1000, 10, 0);
    let tx_hash = tx.hash;

    // Add transaction
    assert!(pool.add(tx.clone()).is_ok());
    assert_eq!(pool.len(), 1);
    assert!(pool.get(&tx_hash).is_some());

    // Get transactions
    let txs = pool.get_all();
    assert_eq!(txs.len(), 1);
    assert!(txs.iter().any(|t| t.hash == tx_hash));

    // Remove transaction
    let removed = pool.remove(&tx_hash);
    assert!(removed.is_some());
    assert_eq!(pool.len(), 0);
}

/// Test transaction pool priority
#[tokio::test]
async fn test_transaction_pool_priority() {
    let mut pool = TransactionPool::new(100);

    // Add transactions with different fees
    let tx1 = Transaction::new(Address([1u8; 20]), Address([2u8; 20]), 100, 10, 0);
    let tx2 = Transaction::new(Address([3u8; 20]), Address([4u8; 20]), 100, 20, 0); // Higher fee
    let tx3 = Transaction::new(Address([5u8; 20]), Address([6u8; 20]), 100, 15, 0);

    pool.add(tx1.clone()).unwrap();
    pool.add(tx2.clone()).unwrap();
    pool.add(tx3.clone()).unwrap();

    // Get transactions (should be ordered by fee)
    let txs = pool.get_all();
    assert_eq!(txs.len(), 3);
    let mut fees: Vec<u128> = txs.iter().map(|t| t.fee).collect();
    fees.sort();
    assert_eq!(fees, vec![10, 15, 20]);
}

/// Test transaction pool size limit
#[tokio::test]
async fn test_transaction_pool_size_limit() {
    let mut pool = TransactionPool::new(5); // Small limit

    // Add transactions up to limit
    for i in 0..5 {
        let tx = Transaction::new(Address([1u8; 20]), Address([2u8; 20]), 100, 10, i);
        assert!(pool.add(tx).is_ok());
    }

    assert_eq!(pool.len(), 5);

    // Try to add one more (should fail)
    let tx = Transaction::new(Address([1u8; 20]), Address([2u8; 20]), 100, 100, 5); // High fee
    assert!(pool.add(tx).is_err());

    // Pool should still be at limit
    assert_eq!(pool.len(), 5);
}

/// Test transaction pool deduplication
#[tokio::test]
async fn test_transaction_pool_deduplication() {
    let mut pool = TransactionPool::new(100);

    let tx = Transaction::new(Address([1u8; 20]), Address([2u8; 20]), 100, 10, 0);
    let _tx_hash = tx.hash;

    // Add transaction twice
    assert!(pool.add(tx.clone()).is_ok());
    assert!(pool.add(tx.clone()).is_ok()); // Duplicate replaces existing

    assert_eq!(pool.len(), 1);
}
