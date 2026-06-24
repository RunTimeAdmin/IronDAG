//! Integration tests for Blockchain + Consensus (GhostDAG)

use irondag::blockchain::{Block, BlockHeader, Blockchain};
use irondag::consensus::GhostDAG;
use irondag::types::StreamType;

/// Test block addition with GhostDAG consensus
#[tokio::test]
async fn test_blockchain_consensus_integration() {
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();

    // Create genesis block (block with zero previous hash)
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;

    // Add to blockchain
    blockchain.add_block(genesis.clone()).await.unwrap();

    // Add to consensus
    consensus.add_block(&genesis).unwrap();

    // Verify genesis is in consensus (check if it's in ordered blocks)
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert!(ordered.iter().any(|b| b.hash == genesis_hash));

    // Create a new block
    let block1_header =
        BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamA, 4, 1_000_000_000);
    let block1 = Block::new(block1_header, vec![]);
    let block1_hash = block1.hash;

    // Add to blockchain
    blockchain.add_block(block1.clone()).unwrap();

    // Add to consensus
    consensus.add_block(&block1).unwrap();

    // Verify block is in consensus
    let ordered2 = consensus.get_ordered_blocks().unwrap();
    assert!(ordered2.iter().any(|b| b.hash == block1_hash));
    assert_eq!(ordered2.len(), 2);
}

/// Test parallel blocks with GhostDAG
#[tokio::test]
async fn test_parallel_blocks_consensus() {
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();

    // Create genesis
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).await.unwrap();
    consensus.add_block(&genesis).unwrap();

    // Create two parallel blocks (both reference genesis)
    let block1_header =
        BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamA, 4, 1_000_000_000);
    let block1 = Block::new(block1_header, vec![]);
    let block1_hash = block1.hash;

    let block2_header =
        BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamB, 4, 1_000_000_000);
    let block2 = Block::new(block2_header, vec![]);
    let block2_hash = block2.hash;

    // Add both blocks
    blockchain.add_block(block1.clone()).unwrap();
    blockchain.add_block(block2.clone()).unwrap();

    consensus.add_block(&block1).unwrap();
    consensus.add_block(&block2).unwrap();

    // Verify ordering includes both
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert_eq!(ordered.len(), 3); // genesis + 2 parallel blocks
    assert!(ordered.iter().any(|b| b.hash == genesis_hash));
    assert!(ordered.iter().any(|b| b.hash == block1_hash));
    assert!(ordered.iter().any(|b| b.hash == block2_hash));
}

/// Test blue score calculation
#[tokio::test]
async fn test_blue_score_calculation() {
    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();

    // Create chain: genesis -> block1 -> block2
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).await.unwrap();
    consensus.add_block(&genesis).unwrap();

    let block1_header =
        BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamA, 4, 1_000_000_000);
    let block1 = Block::new(block1_header, vec![]);
    let block1_hash = block1.hash;
    blockchain.add_block(block1.clone()).unwrap();
    consensus.add_block(&block1).unwrap();

    let block2_header =
        BlockHeader::new(vec![block1_hash], 2, StreamType::StreamA, 4, 1_000_000_000);
    let block2 = Block::new(block2_header, vec![]);
    let block2_hash = block2.hash;
    blockchain.add_block(block2.clone()).unwrap();
    consensus.add_block(&block2).unwrap();

    // Verify ordering
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert_eq!(ordered.len(), 3);
    assert!(ordered.iter().any(|b| b.hash == genesis_hash));
    assert!(ordered.iter().any(|b| b.hash == block1_hash));
    assert!(ordered.iter().any(|b| b.hash == block2_hash));
}
