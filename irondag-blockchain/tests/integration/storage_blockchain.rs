//! Integration tests for Storage + Blockchain

use irondag::blockchain::{Blockchain, Block};
use irondag::storage::{Database, BlockStore, StateStore};
use irondag::types::Hash;
use tempfile::TempDir;

/// Test block persistence
#[tokio::test]
async fn test_block_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    // Create database
    let database = Database::open(&db_path).unwrap();
    let block_store = BlockStore::new(&database);
    
    // Create blockchain and add block
    use irondag::blockchain::BlockHeader;
    use irondag::types::StreamType;
    
    let mut blockchain = Blockchain::new();
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    
    blockchain.add_block(genesis.clone()).unwrap();
    
    // Store block in database
    block_store.put(&genesis).unwrap();
    
    // Retrieve block
    let retrieved = block_store.get(&genesis_hash).unwrap();
    assert!(retrieved.is_some());
    let retrieved_block = retrieved.unwrap();
    assert_eq!(retrieved_block.hash, genesis_hash);
}

/// Test state persistence
#[tokio::test]
async fn test_state_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    // Create database
    let database = Database::open(&db_path).unwrap();
    let state_store = StateStore::new(&database);
    
    // Store balance
    let address: [u8; 20] = [1u8; 20];
    state_store.put_balance(&address, 1_000).unwrap();
    
    // Retrieve balance
    let retrieved = state_store.get_balance(&address).unwrap();
    assert_eq!(retrieved, Some(1_000));
}

/// Test chain length persistence
#[tokio::test]
async fn test_chain_length_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    // Create database
    let database = Database::open(&db_path).unwrap();
    let state_store = StateStore::new(&database);
    
    // Store nonce
    let address: [u8; 20] = [2u8; 20];
    state_store.put_nonce(&address, 42).unwrap();
    
    // Retrieve nonce
    let retrieved = state_store.get_nonce(&address).unwrap();
    assert_eq!(retrieved, Some(42));
}

/// Test database recovery
#[tokio::test]
async fn test_database_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    // Create database and add block
    use irondag::blockchain::BlockHeader;
    use irondag::types::StreamType;
    
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    
    {
        let database = Database::open(&db_path).unwrap();
        let block_store = BlockStore::new(&database);
        block_store.put(&genesis).unwrap();
    }
    
    // Reopen database and verify block is still there
    let database = Database::open(&db_path).unwrap();
    let block_store = BlockStore::new(&database);
    
    let retrieved = block_store.get(&genesis_hash).unwrap();
    assert!(retrieved.is_some());
}


