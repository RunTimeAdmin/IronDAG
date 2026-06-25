//! Block propagation tests

use irondag::blockchain::{Block, BlockHeader, Blockchain};
use irondag::types::{Hash, StreamType};

/// Test block structure for propagation
#[tokio::test]
async fn test_block_structure_for_propagation() {
    let mut blockchain = Blockchain::new();

    // Create a block
    let block_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let block = Block::new(block_header, vec![]);

    // Add to blockchain
    let result = blockchain.add_block(block.clone()).await;
    assert!(result.is_ok());

    // Verify block is in chain
    assert_eq!(blockchain.get_blocks().len(), 1);
}

/// Test block validation
#[tokio::test]
async fn test_block_validation() {
    let block_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let block = Block::new(block_header, vec![]);

    // Block hash should be non-zero
    assert_ne!(block.hash, Hash::zero());
}
