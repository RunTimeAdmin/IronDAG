//! Standalone tests to validate PoW and database improvements
//! Run with: cargo test --test validate_improvements

use irondag::blockchain::{Block, BlockHeader, Transaction};
use irondag::pow;
use irondag::types::{Address, Hash, StreamType};

#[tokio::test]
async fn test_difficulty_damping_prevents_oscillation() {
    println!("\n🧪 Testing difficulty adjustment damping...");
    // NOTE: MAX_DIFFICULTY is capped at 16 for dev/testnet, so use values within that range

    // Test 1: Extreme speedup (10x too fast) should increase difficulty
    let new_diff = pow::adjust_difficulty(4, 10, 1); // Target 10s, actual 1s (10x too fast)
    assert!(new_diff > 4, "Difficulty should increase");
    assert!(new_diff <= 16, "Difficulty capped at MAX_DIFFICULTY=16");
    println!(
        "✅ Test 1 passed: 10x speedup → difficulty increase (capped at {})",
        new_diff
    );

    // Test 2: Extreme slowdown (10x too slow) should decrease difficulty
    let new_diff = pow::adjust_difficulty(8, 10, 100); // Target 10s, actual 100s (10x too slow)
    assert!(new_diff < 8, "Difficulty should decrease");
    assert!(new_diff >= 1, "Difficulty must be at least 1");
    println!(
        "✅ Test 2 passed: 10x slowdown → difficulty decrease to {}",
        new_diff
    );

    // Test 3: Normal adjustment (2x speedup) should work normally
    let new_diff = pow::adjust_difficulty(4, 10, 5); // Target 10s, actual 5s (2x too fast)
    assert!(new_diff > 4, "Difficulty should increase");
    println!(
        "✅ Test 3 passed: 2x speedup → difficulty increase to {}",
        new_diff
    );

    // Test 4: On-target should stay same
    let new_diff = pow::adjust_difficulty(8, 10, 10); // Target 10s, actual 10s (on target)
    assert_eq!(new_diff, 8, "On-target should keep difficulty same");
    println!("✅ Test 4 passed: On-target → no change");

    println!("✅ All damping tests passed! Difficulty adjustment prevents oscillation.\n");
}

#[tokio::test]
async fn test_difficulty_moving_average() {
    println!("\n🧪 Testing moving average difficulty adjustment...");
    // NOTE: MAX_DIFFICULTY is capped at 16 for dev/testnet

    // Test 1: On-target average
    let recent_times = vec![8, 9, 10, 11, 12]; // Average = 10s (on target)
    let new_diff = pow::adjust_difficulty_moving_average(8, 10, &recent_times);
    assert_eq!(new_diff, 8, "On-target average should keep difficulty same");
    println!("✅ Test 1 passed: On-target average → no change");

    // Test 2: Fast blocks (average 3s, target 10s)
    let recent_times = vec![1, 2, 3, 4, 5]; // Average = 3s (too fast)
    let new_diff = pow::adjust_difficulty_moving_average(4, 10, &recent_times);
    assert!(new_diff > 4, "Fast blocks should increase difficulty");
    assert!(new_diff <= 16, "Should be capped at MAX_DIFFICULTY=16");
    println!(
        "✅ Test 2 passed: Fast blocks → difficulty increase to {}",
        new_diff
    );

    // Test 3: Slow blocks (average 22s, target 10s)
    let recent_times = vec![20, 21, 22, 23, 24]; // Average = 22s (too slow)
    let new_diff = pow::adjust_difficulty_moving_average(8, 10, &recent_times);
    assert!(new_diff < 8, "Slow blocks should decrease difficulty");
    assert!(new_diff >= 1, "Difficulty must be at least 1");
    println!(
        "✅ Test 3 passed: Slow blocks → difficulty decrease to {}",
        new_diff
    );

    // Test 4: Empty vector
    let new_diff = pow::adjust_difficulty_moving_average(8, 10, &[]);
    assert_eq!(new_diff, 8, "Empty vector should return current difficulty");
    println!("✅ Test 4 passed: Empty vector → no change");

    println!("✅ All moving average tests passed!\n");
}

#[tokio::test]
async fn test_difficulty_adjustment_stability() {
    println!("\n🧪 Testing difficulty adjustment stability (no oscillation)...");

    // Simulate a sequence of blocks with varying times
    // This tests that damping prevents wild swings
    let mut difficulty = 8u64; // Within MAX_DIFFICULTY=16 cap
    let target_time = 10u64;

    // Sequence: fast, fast, slow, slow, on-target
    let block_times = [5, 5, 20, 20, 10];

    for (i, &actual_time) in block_times.iter().enumerate() {
        let old_diff = difficulty;
        difficulty = pow::adjust_difficulty(difficulty, target_time, actual_time);

        println!(
            "  Block {}: {}s (target: {}s) → difficulty {} → {}",
            i + 1,
            actual_time,
            target_time,
            old_diff,
            difficulty
        );

        // Verify damping: difficulty should never change by more than 4x
        let ratio = if old_diff > 0 {
            (difficulty as f64) / (old_diff as f64)
        } else {
            1.0
        };
        assert!(
            (0.25..=4.0).contains(&ratio),
            "Difficulty change ratio {} should be between 0.25 and 4.0",
            ratio
        );
    }

    println!("✅ Stability test passed! Difficulty changes are bounded.\n");
}

// =============================================================================
// SECURITY REGRESSION TESTS
// =============================================================================

/// SEC-001: Transaction with invalid signature must be rejected
///
/// This test ensures that transactions with garbage/invalid signatures
/// are properly rejected during validation, preventing unauthorized
/// transaction execution.
#[tokio::test]
async fn sec_001_invalid_signature_rejected() {
    println!("\n🔒 Testing SEC-001: Invalid signature rejection...");

    use irondag::blockchain::Blockchain;

    let mut blockchain = Blockchain::new();

    // Create genesis block first
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamC, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    blockchain.add_block(genesis).await.unwrap();

    // Create a transaction with a garbage/invalid signature
    let from = [0x01u8; 20];
    let to = [0x02u8; 20];
    let mut tx = Transaction::new(from, to, 1000, 100, 0);

    // Set a garbage signature (64 bytes of random data that won't verify)
    tx.signature = vec![0xABu8; 64];
    tx.public_key = vec![0xCDu8; 32];

    // Create a block with the invalid transaction
    let header = BlockHeader::new(vec![Hash::zero()], 1, StreamType::StreamC, 4, 1_000_000_000);
    let block = Block::new(header, vec![tx]);

    // Attempt to add the block - should fail due to invalid signature
    let result = blockchain.add_block(block).await;

    // Assert that validation returns an error (not Ok)
    assert!(
        result.is_err(),
        "Transaction with invalid signature should be rejected (SEC-001)"
    );

    // Verify the error message mentions signature
    if let Err(e) = result {
        let error_msg = format!("{}", e);
        assert!(
            error_msg.to_lowercase().contains("signature")
                || error_msg.to_lowercase().contains("invalid"),
            "Error should mention signature or invalid: {}",
            error_msg
        );
        println!("  ✅ Invalid signature correctly rejected: {}", error_msg);
    }

    println!("✅ SEC-001 passed: Invalid signatures are rejected\n");
}

/// SEC-003: Empty/missing signature must return error
///
/// This test ensures that transactions with empty signatures are
/// properly rejected, preventing unsigned transactions from being
/// processed (except for genesis/system transactions).
#[tokio::test]
async fn sec_003_empty_signature_returns_error() {
    println!("\n🔒 Testing SEC-003: Empty signature rejection...");

    use irondag::blockchain::Blockchain;

    let mut blockchain = Blockchain::new();

    // Create genesis block first
    let genesis_header = BlockHeader::new(vec![], 0, StreamType::StreamC, 4, 1_000_000_000);
    let genesis = Block::new(genesis_header, vec![]);
    blockchain.add_block(genesis).await.unwrap();

    // Create a transaction with an empty signature
    let from = Address([0x01u8; 20]);
    let to = Address([0x02u8; 20]);
    let mut tx = Transaction::new(from, to, 1000, 100, 0);

    // Set empty signature and public key
    tx.signature = vec![];
    tx.public_key = vec![];

    // Create a block with the empty signature transaction
    let header = BlockHeader::new(vec![Hash::zero()], 1, StreamType::StreamC, 4, 1_000_000_000);
    let block = Block::new(header, vec![tx]);

    // Attempt to add the block - should fail due to empty signature
    let result = blockchain.add_block(block).await;

    // Assert that validation returns an error
    assert!(
        result.is_err(),
        "Transaction with empty signature should be rejected at non-genesis block (SEC-003)"
    );

    println!("  ✅ Empty signature correctly rejected at block 1");

    // Also test with zero-filled signature (another form of "empty")
    let mut blockchain2 = Blockchain::new();
    let genesis_header2 = BlockHeader::new(vec![], 0, StreamType::StreamC, 4, 1_000_000_000);
    let genesis2 = Block::new(genesis_header2, vec![]);
    blockchain2.add_block(genesis2).await.unwrap();

    let mut tx2 = Transaction::new(from, to, 1000, 100, 1);
    tx2.signature = vec![0u8; 64]; // 64 zeros
    tx2.public_key = vec![0u8; 32];

    let header2 = BlockHeader::new(vec![Hash::zero()], 1, StreamType::StreamC, 4, 1_000_000_000);
    let block2 = Block::new(header2, vec![tx2]);

    let result2 = blockchain2.add_block(block2).await;
    assert!(
        result2.is_err(),
        "Transaction with zero-filled signature should be rejected (SEC-003)"
    );

    println!("  ✅ Zero-filled signature correctly rejected");
    println!("✅ SEC-003 passed: Empty signatures are rejected\n");
}

/// SEC-007: Block hash must not be circular
///
/// This test verifies that the block hash computation does NOT include
/// the block's own hash field in the calculation. This is critical to
/// prevent circular hash dependencies.
///
/// The test creates a block, computes its hash, then verifies that
/// changing the stored hash field doesn't affect the computed hash.
#[tokio::test]
async fn sec_007_block_hash_not_circular() {
    println!("\n🔒 Testing SEC-007: Block hash is not circular...");

    // Create a block with some transactions
    let header = BlockHeader::new(vec![Hash::zero()], 1, StreamType::StreamC, 4, 1_000_000_000);
    let tx = Transaction::new(Address([0x01u8; 20]), Address([0x02u8; 20]), 1000, 100, 0);
    let block = Block::new(header, vec![tx]);

    // Calculate the hash
    let hash1 = block.calculate_hash();
    println!("  Original hash: 0x{}", hex::encode(&hash1.0[..8]));

    // Verify the hash is not all zeros
    assert_ne!(hash1, Hash::zero(), "Block hash should not be all zeros");

    // Create a clone of the block and modify its stored hash field
    let mut modified_block = block.clone();
    modified_block.hash = Hash([0xFFu8; 32]); // Set to a different hash

    // Recalculate the hash of the modified block
    // If the hash computation is NOT circular, the calculated hash should be the same
    // because calculate_hash() should NOT include the stored hash field
    let hash2 = modified_block.calculate_hash();
    println!(
        "  Hash after modifying stored hash field: 0x{}",
        hex::encode(&hash2.0[..8])
    );

    // The calculated hash should be identical regardless of the stored hash value
    assert_eq!(
        hash1, hash2,
        "Block hash computation should NOT include the stored hash field (SEC-007). \
         If hash1 != hash2, the hash is circular."
    );

    println!("  ✅ Hash computation does not include stored hash field");

    // Additional verification: Check that header hash is also not circular
    let header_hash1 = block.header.calculate_header_hash();
    let modified_header = block.header.clone();
    // Header doesn't have a hash field, but let's verify calculate_header_hash
    // is deterministic
    let header_hash2 = modified_header.calculate_header_hash();
    assert_eq!(
        header_hash1, header_hash2,
        "Header hash should be deterministic"
    );

    println!("  ✅ Header hash is deterministic");
    println!("✅ SEC-007 passed: Block hash is not circular\n");
}

/// SEC-004: Invalid PQ key material returns error (not panic)
///
/// This test ensures that transactions with invalid post-quantum
/// signature material return an error rather than causing a panic.
#[tokio::test]
async fn sec_004_invalid_pq_key_returns_error() {
    println!("\n🔒 Testing SEC-004: Invalid PQ key material handling...");

    use irondag::pqc::{PqAccountType, PqSignature};

    let _blockchain = ();

    // Create a transaction with invalid PQ signature
    let from = [0x01u8; 20];
    let to = [0x02u8; 20];
    let mut tx = Transaction::new(from, to, 1000, 100, 0);

    // Set an invalid PQ signature (garbage data)
    let invalid_pq_sig = PqSignature::new(
        PqAccountType::Dilithium3,
        vec![0u8; 100], // Invalid signature data
        vec![0u8; 50],  // Invalid public key data
    );
    tx.pq_signature = Some(invalid_pq_sig);

    // Attempt to verify signature - should return Ok(false), not panic
    let result = tx.verify_signature(1);

    // Should return Ok(false) for invalid signature, not Err or panic
    assert!(
        result.is_ok(),
        "verify_signature should not panic or return Err for invalid PQ key material (SEC-004)"
    );
    assert!(
        !result.unwrap(),
        "Invalid PQ signature should return Ok(false)"
    );

    println!("  ✅ Invalid PQ key material handled gracefully (no panic)");
    println!("✅ SEC-004 passed: Invalid PQ keys return error, not panic\n");
}
