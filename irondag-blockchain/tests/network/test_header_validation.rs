//! Tests for header validation in headers-first sync

#[cfg(test)]
mod tests {
    use irondag::blockchain::{Block, BlockHeader};
    use irondag::network::sync::{BlockHeaderSync, HeadersFirstSync};
    use irondag::types::{Hash, StreamType};

    #[tokio::test]
    async fn test_validate_valid_headers() {
        let sync = HeadersFirstSync::new();
        
        // Create a valid header
        let header = BlockHeader::new(
            vec![Hash::zero()], // parent hash
            1, // block number
            StreamType::StreamA,
            100, // difficulty
        );
        let hash = header.calculate_header_hash();
        let header_sync = BlockHeaderSync { header, hash };
        
        // Add header
        sync.add_headers(vec![header_sync]).await;
        
        // Validate
        let validated = sync.validate_headers().await;
        assert_eq!(validated, 1, "Should validate 1 header");
    }

    #[tokio::test]
    async fn test_reject_header_with_wrong_hash() {
        let sync = HeadersFirstSync::new();
        
        // Create a header with wrong hash
        let header = BlockHeader::new(
            vec![Hash::zero()],
            1,
            StreamType::StreamA,
            100,
        );
        let wrong_hash = Hash([1u8; 32]); // Wrong hash
        let header_sync = BlockHeaderSync { header, hash: wrong_hash };
        
        // Add header
        sync.add_headers(vec![header_sync]).await;
        
        // Validate - should reject
        let validated = sync.validate_headers().await;
        assert_eq!(validated, 0, "Should reject header with wrong hash");
    }

    #[tokio::test]
    async fn test_reject_header_with_zero_difficulty() {
        let sync = HeadersFirstSync::new();
        
        // Create a header with zero difficulty
        let header = BlockHeader::new(
            vec![Hash::zero()],
            1,
            StreamType::StreamA,
            0, // Zero difficulty - invalid
        );
        let hash = header.calculate_header_hash();
        let header_sync = BlockHeaderSync { header, hash };
        
        // Add header
        sync.add_headers(vec![header_sync]).await;
        
        // Validate - should reject
        let validated = sync.validate_headers().await;
        assert_eq!(validated, 0, "Should reject header with zero difficulty");
    }

    #[tokio::test]
    async fn test_reject_header_with_future_timestamp() {
        let sync = HeadersFirstSync::new();
        
        // Create a header with timestamp far in future
        let mut header = BlockHeader::new(
            vec![Hash::zero()],
            1,
            StreamType::StreamA,
            100,
        );
        // Set timestamp to 1 hour in future (more than 10 minute tolerance)
        let future_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() + 3600;
        header.timestamp = future_time;
        
        let hash = header.calculate_header_hash();
        let header_sync = BlockHeaderSync { header, hash };
        
        // Add header
        sync.add_headers(vec![header_sync]).await;
        
        // Validate - should reject
        let validated = sync.validate_headers().await;
        assert_eq!(validated, 0, "Should reject header with future timestamp");
    }
}

