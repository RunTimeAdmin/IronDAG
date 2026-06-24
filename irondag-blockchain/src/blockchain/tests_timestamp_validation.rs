//! Tests for timestamp validation

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::blockchain::{Block, BlockHeader};
    use crate::types::{Hash, StreamType};

    fn create_test_block(block_number: u64, timestamp: u64, parent_hashes: Vec<Hash>) -> Block {
        let header = BlockHeader {
            parent_hashes: parent_hashes.clone(),
            block_number,
            stream_type: StreamType::StreamA,
            difficulty: 8,
            timestamp,
            nonce: 0,
        };
        let transactions = vec![];
        Block::new(header, transactions, parent_hashes)
    }

    #[test]
    fn test_timestamp_median_validation() {
        let mut blockchain = Blockchain::new();

        // Create genesis block
        let genesis = create_test_block(0, 1000, vec![]);
        futures::executor::block_on(blockchain.add_block(genesis)).unwrap();

        // Add blocks with increasing timestamps
        let mut parent_hash = blockchain.blocks[0].hash;
        for i in 1..=11 {
            let block = create_test_block(i, 1000 + i, vec![parent_hash]);
            futures::executor::block_on(blockchain.add_block(block.clone())).unwrap();
            parent_hash = block.hash;
        }

        // Now try to add a block with timestamp BEFORE median
        // Median of last 11 blocks: [1001, 1002, ..., 1011] -> median = 1006
        let median_timestamp = 1006;
        let block_with_old_timestamp = create_test_block(12, median_timestamp - 1, vec![parent_hash]);

        // Should fail validation
        let result = futures::executor::block_on(blockchain.add_block(block_with_old_timestamp));
        assert!(result.is_err(), "Block with timestamp before median should be rejected");

        // Should fail with appropriate error message
        if let Err(e) = result {
            assert!(e.to_string().contains("median timestamp"),
                "Error should mention median timestamp");
        }
    }

    #[test]
    fn test_timestamp_future_limit() {
        let mut blockchain = Blockchain::new();

        // Create genesis block
        let genesis = create_test_block(0, 1000, vec![]);
        futures::executor::block_on(blockchain.add_block(genesis)).unwrap();

        // Try to add block with timestamp too far in future (10 minutes = 600 seconds)
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let future_timestamp = current_time + 601; // 601 seconds = too far

        let block = create_test_block(1, future_timestamp, vec![blockchain.blocks[0].hash]);

        // Should fail validation
        let result = futures::executor::block_on(blockchain.add_block(block));
        assert!(result.is_err(), "Block with timestamp too far in future should be rejected");

        if let Err(e) = result {
            assert!(e.to_string().contains("too far in future"),
                "Error should mention future timestamp");
        }
    }

    #[test]
    fn test_timestamp_median_with_odd_number_of_blocks() {
        let mut blockchain = Blockchain::new();

        // Create genesis block
        let genesis = create_test_block(0, 1000, vec![]);
        futures::executor::block_on(blockchain.add_block(genesis)).unwrap();

        // Add 5 blocks (total 6 blocks, median of last 5 = middle value)
        let mut parent_hash = blockchain.blocks[0].hash;
        let timestamps = vec![1001, 1002, 1003, 1004, 1005];
        for (i, &ts) in timestamps.iter().enumerate() {
            let block = create_test_block(i as u64 + 1, ts, vec![parent_hash]);
            futures::executor::block_on(blockchain.add_block(block.clone())).unwrap();
            parent_hash = block.hash;
        }

        // Median of [1001, 1002, 1003, 1004, 1005] = 1003
        // Block with timestamp >= 1003 should be accepted
        let valid_block = create_test_block(6, 1003, vec![parent_hash]);
        assert!(futures::executor::block_on(blockchain.add_block(valid_block)).is_ok(),
            "Block with timestamp equal to median should be accepted");

        // Block with timestamp < 1003 should be rejected
        let invalid_block = create_test_block(7, 1002, vec![parent_hash]);
        assert!(futures::executor::block_on(blockchain.add_block(invalid_block)).is_err(),
            "Block with timestamp before median should be rejected");
    }

    #[test]
    fn test_timestamp_validation_allows_valid_timestamps() {
        let mut blockchain = Blockchain::new();

        // Create genesis block
        let genesis = create_test_block(0, 1000, vec![]);
        futures::executor::block_on(blockchain.add_block(genesis)).unwrap();

        // Add blocks with valid timestamps (>= median)
        let mut parent_hash = blockchain.blocks[0].hash;
        for i in 1..=5 {
            let block = create_test_block(i, 1000 + i, vec![parent_hash]);
            let result = futures::executor::block_on(blockchain.add_block(block.clone()));
            assert!(result.is_ok(), "Valid timestamp should be accepted");
            parent_hash = block.hash;
        }
    }
}
