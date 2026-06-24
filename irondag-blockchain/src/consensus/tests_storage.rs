//! Tests for hybrid storage

#[cfg(test)]
mod tests {
    use super::storage::*;
    use crate::blockchain::{Block, BlockHeader};
    use crate::types::{Hash, StreamType};
    use crate::storage::Database;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_block(block_number: u64) -> Block {
        let header = BlockHeader {
            parent_hashes: vec![],
            block_number,
            stream_type: StreamType::StreamA,
            difficulty: 8,
            timestamp: 1000 + block_number,
            nonce: 0,
        };
        let transactions = vec![];
        Block::new(header, transactions, vec![])
    }

    #[test]
    fn test_hybrid_storage_basic() {
        let storage = HybridDagStorage::new(Default::default());

        // Add a block
        let block = create_test_block(1);
        let hash = block.hash;
        storage.add_block(block.clone()).unwrap();

        // Should be retrievable from hot cache
        let retrieved = storage.get_block(&hash).unwrap();
        assert!(retrieved.is_some(), "Block should be in hot cache");
        assert_eq!(retrieved.unwrap().header.block_number, 1);
    }

    #[test]
    fn test_hybrid_storage_with_database() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");
        let database = Arc::new(Database::open(&db_path).unwrap());

        let config = DagStorageConfig {
            hot_cache_size: 5,
            finalized_depth: 3,
            confirmations_for_checkpoint: 0,
        };
        let mut storage = HybridDagStorage::with_database(database, config);

        // Add blocks
        for i in 1..=10 {
            let block = create_test_block(i);
            storage.add_block(block).unwrap();
        }

        // All blocks should be retrievable (from hot cache or disk)
        for i in 1..=10 {
            let block = create_test_block(i);
            let hash = block.hash;
            let retrieved = storage.get_block(&hash).unwrap();
            assert!(retrieved.is_some(), "Block {} should be retrievable", i);
        }
    }

    #[test]
    fn test_hot_cache_pruning() {
        let config = DagStorageConfig {
            hot_cache_size: 3,
            finalized_depth: 2,
            confirmations_for_checkpoint: 0,
        };
        let mut storage = HybridDagStorage::new(config);

        // Add 5 blocks
        let mut hashes = Vec::new();
        for i in 1..=5 {
            let block = create_test_block(i);
            let hash = block.hash;
            hashes.push(hash);
            storage.add_block(block).unwrap();
        }

        // Hot cache should only have 3 blocks (pruned)
        let hot_blocks = storage.get_hot_blocks();
        assert!(hot_blocks.len() <= 3, "Hot cache should be pruned to 3 blocks");

        // But all blocks should still be retrievable (if database is used)
        // For in-memory only, older blocks might be gone
        // This test verifies pruning happens
    }

    #[test]
    fn test_blue_set_storage() {
        let mut storage = HybridDagStorage::new(Default::default());

        let block = create_test_block(1);
        let hash = block.hash;
        storage.add_block(block).unwrap();

        // Add to blue set
        storage.add_to_blue_set(hash);
        assert!(storage.is_blue(&hash), "Block should be in blue set");

        // Set blue score
        storage.set_blue_score(hash, 100);
        assert_eq!(storage.get_blue_score(&hash), Some(100));
    }

    #[test]
    fn test_children_storage() {
        let mut storage = HybridDagStorage::new(Default::default());

        let parent = create_test_block(1);
        let parent_hash = parent.hash;
        storage.add_block(parent).unwrap();

        let child = create_test_block(2);
        let child_hash = child.hash;
        storage.add_block(child).unwrap();

        // Set children relationship
        storage.set_children(parent_hash, vec![child_hash]);

        // Get children
        let children = storage.get_children(&parent_hash).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], child_hash);
    }

    #[test]
    fn test_persistence_across_restarts() {
        // This test simulates a restart by creating new storage with same database
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");

        // First "session": add blocks
        {
            let database = Arc::new(Database::open(&db_path).unwrap());
            let mut storage = HybridDagStorage::with_database(database, Default::default());

            for i in 1..=5 {
                let block = create_test_block(i);
                storage.add_block(block).unwrap();
            }
        }

        // Second "session": blocks should still be there
        {
            let database = Arc::new(Database::open(&db_path).unwrap());
            let storage = HybridDagStorage::with_database(database, Default::default());

            // Blocks should be retrievable from disk
            for i in 1..=5 {
                let block = create_test_block(i);
                let hash = block.hash;
                let retrieved = storage.get_block(&hash).unwrap();
                assert!(retrieved.is_some(), "Block {} should persist across restart", i);
            }
        }
    }
}
