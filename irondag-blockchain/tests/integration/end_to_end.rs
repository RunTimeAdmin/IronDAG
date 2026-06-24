//! End-to-end integration tests

use irondag::blockchain::{Blockchain, Block, Transaction};
use irondag::consensus::GhostDAG;
use irondag::storage::{Database, BlockStore, StateStore};
use irondag::node::pool::TransactionPool;
use tempfile::TempDir;

/// Test complete transaction flow
#[tokio::test]
async fn test_complete_transaction_flow() {
    // Setup
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Database::open(&db_path).unwrap();
    let block_store = BlockStore::new(&database);
    let state_store = StateStore::new(&database);
    
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();
    let mut tx_pool = TransactionPool::new(100);
    
    // 1. Create transaction
    let tx = Transaction::new(
        [1u8; 20],
        [2u8; 20],
        1000,
        10,
        0,
    );
    
    // 2. Add to pool
    tx_pool.add(tx.clone());
    
    // 3. Create block with transaction
    use irondag::blockchain::BlockHeader;
    use irondag::types::StreamType;
    
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).unwrap();
    consensus.add_block(genesis);
    
    let block_header = BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamA, 4);
    let block = Block::new(block_header, vec![tx.clone()]);
    let block_hash = block.hash;
    
    // 4. Add block to blockchain
    blockchain.add_block(block.clone()).unwrap();
    
    // 5. Add to consensus
    consensus.add_block(block.clone());
    
    // 6. Store in database
    block_store.put(&block).unwrap();
    
    // 7. Remove transaction from pool
    tx_pool.remove(&tx.hash);
    
    // Verify everything
    assert_eq!(blockchain.get_blocks().len(), 2); // genesis + block
    assert!(consensus.get_blue_set().contains(&block_hash));
    assert_eq!(tx_pool.len(), 0);
    
    // Verify block in database
    let retrieved = block_store.get(&block_hash).unwrap();
    assert!(retrieved.is_some());
}

/// Test blockchain state consistency
#[tokio::test]
async fn test_blockchain_state_consistency() {
    use irondag::blockchain::BlockHeader;
    use irondag::types::StreamType;
    
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();
    
    // Create chain
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4);
    let genesis = Block::new(genesis_header, vec![]);
    let mut prev_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).unwrap();
    consensus.add_block(genesis);
    
    for i in 1..=5 {
        let block_header = BlockHeader::new(vec![prev_hash], i, StreamType::StreamA, 4);
        let block = Block::new(block_header, vec![]);
        prev_hash = block.hash;
        blockchain.add_block(block.clone()).unwrap();
        consensus.add_block(block);
    }
    
    // Verify chain length
    assert_eq!(blockchain.get_blocks().len(), 6); // genesis + 5 blocks
    
    // Verify ordering
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert_eq!(ordered.len(), 6);
}


