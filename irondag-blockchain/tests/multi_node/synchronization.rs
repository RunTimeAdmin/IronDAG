//! Network synchronization tests

use irondag::blockchain::{Blockchain, Block, BlockHeader};
use irondag::consensus::GhostDAG;
use irondag::types::StreamType;

/// Test chain synchronization structure
#[tokio::test]
async fn test_chain_synchronization() {
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();
    
    // Create chain of blocks
    let mut prev_hash = [0u8; 32];
    for i in 0..5 {
        let parent_hashes = if i == 0 { vec![] } else { vec![prev_hash] };
        let block_header = BlockHeader::new(parent_hashes, i, StreamType::StreamA, 4);
        let block = Block::new(block_header, vec![]);
        prev_hash = block.hash;
        
        blockchain.add_block(block.clone()).await.unwrap();
        consensus.add_block(block);
    }
    
    // Verify chain length
    assert_eq!(blockchain.get_blocks().len(), 5);
    
    // Verify consensus has all blocks
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert_eq!(ordered.len(), 5);
}

/// Test state consistency across nodes (simulated)
#[tokio::test]
async fn test_state_consistency() {
    let mut blockchain1 = Blockchain::new();
    let mut blockchain2 = Blockchain::new();
    
    // Add same blocks to both
    let block_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4);
    let block = Block::new(block_header, vec![]);
    
    blockchain1.add_block(block.clone()).await.unwrap();
    blockchain2.add_block(block.clone()).await.unwrap();
    
    // Both should have same chain length
    assert_eq!(blockchain1.get_blocks().len(), blockchain2.get_blocks().len());
    assert_eq!(blockchain1.get_blocks().len(), 1);
}




