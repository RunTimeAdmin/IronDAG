//! End-to-end integration tests

use ed25519_dalek::SigningKey;
use irondag_blockchain::blockchain::{Block, Blockchain, Transaction};
use irondag_blockchain::consensus::GhostDAG;
use irondag_blockchain::node::pool::TransactionPool;
use irondag_blockchain::storage::{BlockStore, Database, StateStore};
use irondag_blockchain::types::Address;
use tempfile::TempDir;

/// Test complete transaction flow
#[test]
fn test_complete_transaction_flow() {
    // Setup
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Database::open(&db_path).unwrap();
    let block_store = BlockStore::new(&database);
    let _state_store = StateStore::new(&database);

    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();
    let mut tx_pool = TransactionPool::new(100);

    // 1. Set balance for sender
    let sender_secret = [1u8; 32];
    let sender = Transaction::derive_address_from_public_key(
        &SigningKey::from_bytes(&sender_secret)
            .verifying_key()
            .to_bytes(),
    );
    blockchain.set_balance(sender, 10000).unwrap();

    // 2. Create transaction
    let tx = Transaction::new(sender, Address([2u8; 20]), 1000, 10, 0).sign(&sender_secret);

    // 3. Add to pool
    tx_pool.add(tx.clone()).unwrap();

    // 4. Create block with transaction
    use irondag_blockchain::blockchain::BlockHeader;
    use irondag_blockchain::types::StreamType;

    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    let genesis_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).unwrap();
    consensus.add_block(&genesis).unwrap();

    let block_header =
        BlockHeader::new(vec![genesis_hash], 1, StreamType::StreamA, 4, 1_000_000_000);
    let block = Block::new(block_header, vec![tx.clone()]);
    let block_hash = block.hash;

    // 5. Add block to blockchain
    blockchain.add_block(block.clone()).unwrap();

    // 6. Add to consensus
    consensus.add_block(&block.clone()).unwrap();

    // 7. Store in database
    block_store.put(&block).unwrap();

    // 8. Remove transaction from pool
    tx_pool.remove(&tx.hash);

    // Verify everything
    assert_eq!(blockchain.get_blocks().len(), 2); // genesis + block
                                                  // Verify block is in consensus (check ordered blocks)
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert!(ordered.iter().any(|b| b.hash == block_hash));
    assert_eq!(tx_pool.len(), 0);

    // Verify block in database
    let retrieved = block_store.get(&block_hash).unwrap();
    assert!(retrieved.is_some());
}

/// Test blockchain state consistency
#[test]
fn test_blockchain_state_consistency() {
    use irondag_blockchain::blockchain::BlockHeader;
    use irondag_blockchain::types::StreamType;

    let mut blockchain = Blockchain::new();
    let mut consensus = GhostDAG::new();

    // Create chain
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamA, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    let mut prev_hash = genesis.hash;
    blockchain.add_block(genesis.clone()).unwrap();
    consensus.add_block(&genesis).unwrap();

    for i in 1..=5 {
        let block_header =
            BlockHeader::new(vec![prev_hash], i, StreamType::StreamA, 4, 1_000_000_000);
        let block = Block::new(block_header, vec![]);
        prev_hash = block.hash;
        blockchain.add_block(block.clone()).unwrap();
        consensus.add_block(&block).unwrap();
    }

    // Verify chain length
    assert_eq!(blockchain.get_blocks().len(), 6); // genesis + 5 blocks

    // Verify ordering
    let ordered = consensus.get_ordered_blocks().unwrap();
    assert_eq!(ordered.len(), 6);
}
