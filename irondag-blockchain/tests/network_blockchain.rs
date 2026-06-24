//! Integration tests for Network + Blockchain

use irondag_blockchain::blockchain::{Block, BlockHeader, Blockchain, Transaction};
use irondag_blockchain::network::NetworkManager;
use irondag_blockchain::types::{Address, StreamType};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Test network manager creation
#[tokio::test]
async fn test_network_blockchain_integration() {
    // Create network manager
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let listen_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let network = NetworkManager::new(blockchain, listen_addr);
    assert_eq!(network.peer_count(), 0);
}

/// Test block broadcasting (simplified - actual broadcasting needs running nodes)
#[tokio::test]
async fn test_block_broadcasting() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let listen_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let network = NetworkManager::new(blockchain, listen_addr);

    // Create a block
    let block_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let block = Block::new(block_header, vec![]);

    // Broadcast block (will work even without peers)
    let result = network.broadcast_block(&block, true).await;
    assert!(result.is_ok());

    // Verify network is set up
    assert_eq!(network.peer_count(), 0); // No peers yet
}

/// Test transaction broadcasting
#[tokio::test]
async fn test_transaction_broadcasting() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let sender = Address([1u8; 20]);
    {
        let mut bc = blockchain.write().await;
        bc.set_balance(sender, 10000).unwrap();
    }

    let listen_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let network = NetworkManager::new(blockchain, listen_addr);

    // Create transaction
    let tx = Transaction::new(sender, Address([2u8; 20]), 1000, 10, 0);

    // Broadcast transaction
    let result = network.broadcast_transaction(&tx).await;
    assert!(result.is_ok());

    // Verify network is set up
    assert_eq!(network.peer_count(), 0); // No peers yet
}
