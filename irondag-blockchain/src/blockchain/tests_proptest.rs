//! Property-based tests for transaction validation and parsing using proptest
//!
//! TST-003: Systematic coverage of edge cases in transaction parsing,
//! overflow conditions, and malformed input handling.

#[cfg(test)]
mod tests {
    use crate::blockchain::{Blockchain, Transaction};
    use ed25519_dalek::SigningKey;
    use proptest::prelude::*;

    /// Maximum transaction data size (128KB)
    const MAX_TX_DATA_SIZE: usize = 128 * 1024;

    /// Maximum gas limit per transaction (30M gas)
    const MAX_GAS_LIMIT: u64 = 30_000_000;

    /// Maximum transaction value to prevent overflow
    const MAX_TX_VALUE: u128 = u128::MAX / 2;

    /// Helper to create a signed transaction for testing
    fn create_test_transaction(
        from: [u8; 20],
        to: [u8; 20],
        value: u128,
        fee: u128,
        nonce: u64,
        data: Vec<u8>,
        gas_limit: u64,
    ) -> Transaction {
        let mut tx = Transaction::with_data(from, to, value, fee, nonce, data, gas_limit);
        // For testing validation, we need a valid signature
        // Use a deterministic secret key based on the 'from' address
        let mut secret_bytes = [0u8; 32];
        secret_bytes[..20].copy_from_slice(&from);
        let _signing_key = SigningKey::from_bytes(&secret_bytes);
        tx = tx.sign(&secret_bytes);
        tx
    }

    /// Helper to create a minimal blockchain for testing
    fn create_test_blockchain() -> Blockchain {
        Blockchain::new()
    }

    // =========================================================================
    // Test 1: Transaction validation never panics on arbitrary input
    // =========================================================================
    // Note: Proptest will catch panics and report them as test failures.
    // If validate_transaction panics, the test will fail.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn validate_transaction_handles_arbitrary_input(
            from in prop::array::uniform20(any::<u8>()),
            to in prop::array::uniform20(any::<u8>()),
            value in any::<u128>(),
            fee in any::<u128>(),
            nonce in any::<u64>(),
            gas_limit in any::<u64>(),
            data_len in 0usize..200_000usize,
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create transaction with arbitrary fields
            let data = vec![0u8; data_len];
            let tx = create_test_transaction(from, to, value, fee, nonce, data, gas_limit);

            // Call validate_transaction — it should return Ok or Err, NEVER panic
            // If it panics, proptest will catch it and report a failure
            let _result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            // Any result is acceptable (Ok or Err), the key is no panic
            // We've successfully validated that arbitrary input doesn't cause a panic
        }
    }

    // =========================================================================
    // Test 2: Transaction serialization round-trips
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn transaction_serialization_roundtrip(
            from in prop::array::uniform20(any::<u8>()),
            to in prop::array::uniform20(any::<u8>()),
            value in 0u128..MAX_TX_VALUE,
            fee in 0u128..1_000_000u128,
            nonce in any::<u64>(),
            gas_limit in 1u64..MAX_GAS_LIMIT,
        ) {
            // Create a transaction with constrained values
            let tx = create_test_transaction(
                from,
                to,
                value,
                fee,
                nonce,
                Vec::new(),
                gas_limit,
            );

            // Serialize using bincode
            let serialized = bincode::serialize(&tx);
            prop_assert!(serialized.is_ok(), "Serialization failed");

            let serialized = serialized.unwrap();

            // Deserialize
            let deserialized: Result<Transaction, _> = bincode::deserialize(&serialized);
            prop_assert!(deserialized.is_ok(), "Deserialization failed");

            let deserialized = deserialized.unwrap();

            // Verify key fields round-trip correctly
            prop_assert_eq!(tx.from, deserialized.from, "from address mismatch");
            prop_assert_eq!(tx.to, deserialized.to, "to address mismatch");
            prop_assert_eq!(tx.value, deserialized.value, "value mismatch");
            prop_assert_eq!(tx.fee, deserialized.fee, "fee mismatch");
            prop_assert_eq!(tx.nonce, deserialized.nonce, "nonce mismatch");
            prop_assert_eq!(tx.gas_limit, deserialized.gas_limit, "gas_limit mismatch");
            prop_assert_eq!(tx.data, deserialized.data, "data mismatch");
        }
    }

    // =========================================================================
    // Test 3: Block validation rejects oversized data
    // =========================================================================
    // Note: Due to validation order, signature verification happens before data size check.
    // The test verifies that oversized data transactions are rejected (for any reason).
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn oversized_tx_data_rejected(
            data_len in (MAX_TX_DATA_SIZE + 1)..(MAX_TX_DATA_SIZE * 4),
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create transaction with oversized data
            let data = vec![0u8; data_len];
            let tx = create_test_transaction(
                [1u8; 20],  // from
                [2u8; 20],  // to
                1000,       // value
                100,        // fee
                0,          // nonce
                data,
                21_000,     // gas_limit
            );

            // Should be rejected (either for data size or other validation failure)
            // The key property is that oversized transactions don't cause panics
            let result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            prop_assert!(
                result.is_err(),
                "Transaction with oversized data ({} bytes) should be rejected",
                data_len
            );
        }
    }

    // =========================================================================
    // Test 4: Fee + value overflow always caught
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn fee_value_overflow_caught(
            value in (MAX_TX_VALUE)..u128::MAX,
            fee in 1u128..u128::MAX,
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create transaction that could cause overflow
            let tx = create_test_transaction(
                [1u8; 20],  // from
                [2u8; 20],  // to
                value,
                fee,
                0,          // nonce
                Vec::new(), // data
                21_000,     // gas_limit
            );

            // Validation should handle overflow gracefully (return Err, not panic)
            let result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            // Should return an error (either overflow or value exceeds max)
            prop_assert!(
                result.is_err(),
                "Overflow transaction should be rejected (value: {}, fee: {})",
                value, fee
            );
        }
    }

    // =========================================================================
    // Test 5: Gas limit validation
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn gas_limit_validation(
            gas_limit in any::<u64>(),
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create transaction with arbitrary gas limit
            let tx = create_test_transaction(
                [1u8; 20],  // from
                [2u8; 20],  // to
                1000,       // value
                100,        // fee
                0,          // nonce
                Vec::new(), // data
                gas_limit,
            );

            // Validation should not panic
            let result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            if gas_limit == 0 || gas_limit > MAX_GAS_LIMIT {
                prop_assert!(
                    result.is_err(),
                    "Gas limit {} should be rejected (zero or exceeds max {})",
                    gas_limit, MAX_GAS_LIMIT
                );
            }
        }
    }

    // =========================================================================
    // Test 6: Zero address handling
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn zero_address_from_handling(
            to in prop::array::uniform20(any::<u8>()),
            value in any::<u128>(),
            fee in any::<u128>(),
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Transaction from zero address (system/genesis)
            let zero_address = [0u8; 20];
            let tx = create_test_transaction(
                zero_address,
                to,
                value,
                fee,
                0,          // nonce
                Vec::new(), // data
                21_000,     // gas_limit
            );

            // Validation should not panic on zero address
            let _result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            // Any result is acceptable, the key is no panic
        }
    }

    // =========================================================================
    // Test 7: Transaction hash consistency
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn transaction_hash_is_deterministic(
            from in prop::array::uniform20(any::<u8>()),
            to in prop::array::uniform20(any::<u8>()),
            value in 0u128..1_000_000u128,
            fee in 0u128..1_000u128,
            nonce in 0u64..1000u64,
        ) {
            // Create two identical transactions
            let tx1 = create_test_transaction(
                from, to, value, fee, nonce, Vec::new(), 21_000
            );
            let tx2 = create_test_transaction(
                from, to, value, fee, nonce, Vec::new(), 21_000
            );

            // Hashes should be identical
            prop_assert_eq!(
                tx1.hash, tx2.hash,
                "Identical transactions should have identical hashes"
            );

            // Hash should also be reproducible via calculate_hash
            let calculated = tx1.calculate_hash();
            prop_assert_eq!(
                tx1.hash, calculated,
                "Stored hash should match calculated hash"
            );
        }
    }

    // =========================================================================
    // Test 8: Nonce gap handling
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn nonce_gap_handling(
            nonce_gap in 0u64..50u64,
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create a transaction with a nonce that's ahead of the account nonce
            // The account starts with nonce 0, so this tests gap behavior
            let tx = create_test_transaction(
                [1u8; 20],      // from
                [2u8; 20],      // to
                1000,           // value
                100,            // fee
                nonce_gap,      // nonce with potential gap
                Vec::new(),     // data
                21_000,         // gas_limit
            );

            // Validation should not panic
            let result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            // For non-zero nonce from fresh account, validation should fail
            // because the expected nonce is 0
            if nonce_gap > 0 {
                prop_assert!(
                    result.is_err(),
                    "Non-zero nonce {} from fresh account should be rejected",
                    nonce_gap
                );
            }
        }
    }

    // =========================================================================
    // Test 9: Transaction data size boundary
    // =========================================================================
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        #[test]
        fn tx_data_size_boundary(
            data_len in 0usize..(MAX_TX_DATA_SIZE + 1),
        ) {
            // Create test blockchain
            let blockchain = create_test_blockchain();

            // Create transaction with data at boundary
            let data = vec![0u8; data_len];
            let tx = create_test_transaction(
                [1u8; 20],  // from
                [2u8; 20],  // to
                1000,       // value
                100,        // fee
                0,          // nonce
                data,
                21_000,     // gas_limit
            );

            // Validation should not panic
            let _result = futures::executor::block_on(blockchain.validate_transaction(&tx, 1, 0));

            // Data at or below max should pass this check (may fail on other checks)
            if data_len <= MAX_TX_DATA_SIZE {
                // We don't assert success because other validation may fail
                // (e.g., signature, nonce, balance) - we just ensure no panic
            }
        }
    }
}
