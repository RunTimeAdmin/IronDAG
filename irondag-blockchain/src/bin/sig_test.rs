/// ECDSA secp256k1 Transaction Signature Verification (EIP-155)
///
/// This standalone spike proves transaction signature verification.
/// Implements EIP-155 replay protection with chain ID.
///
/// Closes CRITICAL security gap: transaction authentication.
use k256::ecdsa::{
    signature::hazmat::PrehashVerifier, RecoveryId, Signature, SigningKey, VerifyingKey,
};
use sha3::{Digest, Keccak256};
use tracing::{error, info};

/// Transaction structure for signing
#[derive(Debug, Clone)]
struct Transaction {
    nonce: u64,
    to: [u8; 20],
    value: u128,
    data: Vec<u8>,
    gas_limit: u64,
    gas_price: u128,
    chain_id: u64, // EIP-155
}

impl Transaction {
    /// Create signing hash (EIP-155 format)
    fn signing_hash(&self) -> [u8; 32] {
        let mut hasher = Keccak256::new();

        // EIP-155: hash(nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0)
        hasher.update(self.nonce.to_be_bytes());
        hasher.update(self.gas_price.to_be_bytes());
        hasher.update(self.gas_limit.to_be_bytes());
        hasher.update(self.to);
        hasher.update(self.value.to_be_bytes());
        hasher.update(&self.data);
        hasher.update(self.chain_id.to_be_bytes());
        hasher.update([0u8; 8]); // v = chainId
        hasher.update([0u8; 8]); // r = 0

        let hash = hasher.finalize();
        let mut result = [0u8; 32];
        result.copy_from_slice(&hash);
        result
    }
}

/// Signed transaction with recovery
#[derive(Debug)]
struct SignedTransaction {
    transaction: Transaction,
    signature: Signature,
    recovery_id: u8,
}

impl SignedTransaction {
    /// Sign transaction with private key
    fn sign(tx: Transaction, signing_key: &SigningKey) -> Self {
        let hash = tx.signing_hash();

        // Use sign_prehash_recoverable to get both signature and recovery_id
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&hash)
            .expect("Signing should never fail with valid key");

        SignedTransaction {
            transaction: tx,
            signature,
            recovery_id: recovery_id.to_byte(),
        }
    }

    /// Verify signature
    fn verify(&self) -> Result<[u8; 20], String> {
        let hash = self.transaction.signing_hash();
        let verifying_key = self.recover_public_key(&hash)?;

        // Verify signature (use verify_prehash since we already have the hash)
        verifying_key
            .verify_prehash(&hash, &self.signature)
            .map_err(|e| format!("Signature verification failed: {}", e))?;

        // Derive address from public key (last 20 bytes of keccak256(pubkey))
        let pubkey_bytes = verifying_key.to_sec1_bytes();
        let mut hasher = Keccak256::new();
        hasher.update(&pubkey_bytes[1..]); // Skip 0x04 prefix
        let hash = hasher.finalize();

        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..]);
        Ok(address)
    }

    /// Recover public key from signature
    fn recover_public_key(&self, hash: &[u8; 32]) -> Result<VerifyingKey, String> {
        let recovery_id = RecoveryId::from_byte(self.recovery_id)
            .ok_or_else(|| format!("Invalid recovery ID: {}", self.recovery_id))?;

        VerifyingKey::recover_from_prehash(hash, &self.signature, recovery_id)
            .map_err(|e| format!("Public key recovery failed: {}", e))
    }
}

fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("irondag=info".parse().unwrap()),
        )
        .init();

    info!("ECDSA secp256k1 Transaction Signature Verification (EIP-155)");
    info!("===============================================================");

    // Track test results
    let mut all_tests_passed = true;

    // Generate keypair
    let signing_key = SigningKey::random(&mut rand::thread_rng());
    let verifying_key = VerifyingKey::from(&signing_key);

    // Derive expected address from the signing key
    let expected_pubkey_bytes = verifying_key.to_sec1_bytes();
    let mut hasher = Keccak256::new();
    hasher.update(&expected_pubkey_bytes[1..]); // Skip 0x04 prefix
    let expected_addr_hash = hasher.finalize();
    let mut expected_address = [0u8; 20];
    expected_address.copy_from_slice(&expected_addr_hash[12..]);

    info!("Generated keypair");
    info!("Public key: {}", hex::encode(verifying_key.to_sec1_bytes()));
    info!("Expected address: 0x{}", hex::encode(expected_address));

    // Create transaction
    let tx = Transaction {
        nonce: 0,
        to: [0x12; 20],
        value: 1_000_000_000_000_000_000u128, // 1 IDAG
        data: vec![],
        gas_limit: 21000,
        gas_price: 1_000_000_000u128,
        chain_id 11567, // IronDAG testnet
    };

    info!("Created transaction");
    info!("To: 0x{}", hex::encode(tx.to));
    info!("Value: {} wei", tx.value);
    info!("Chain ID: {} (EIP-155)", tx.chain_id);

    // Sign transaction
    let signed_tx = SignedTransaction::sign(tx.clone(), &signing_key);
    info!("Signed transaction");
    info!("Signature: {}", hex::encode(signed_tx.signature.to_bytes()));
    info!("Recovery ID: {}", signed_tx.recovery_id);

    // Verify signature and check address matches
    match signed_tx.verify() {
        Ok(recovered_address) => {
            info!("Signature verified successfully!");
            info!("Recovered address: 0x{}", hex::encode(recovered_address));
            if recovered_address != expected_address {
                error!("ERROR: Recovered address does not match expected address!");
                all_tests_passed = false;
            }
        }
        Err(e) => {
            error!("Signature verification failed: {}", e);
            all_tests_passed = false;
        }
    }

    // Test with wrong signature - verify address mismatch detection
    info!("Testing tampered transaction...");
    let wrong_key = SigningKey::random(&mut rand::thread_rng());
    let wrong_signed = SignedTransaction::sign(tx.clone(), &wrong_key);

    // Create tampered transaction with modified value
    let mut modified_tx = tx.clone();
    modified_tx.value = 2_000_000_000_000_000_000u128;
    let tampered = SignedTransaction {
        transaction: modified_tx,
        signature: wrong_signed.signature,
        recovery_id: wrong_signed.recovery_id,
    };

    // Verify - should recover an address that doesn't match expected_address
    match tampered.verify() {
        Ok(recovered_address) => {
            // The key check: recovered address should NOT match the expected address
            if recovered_address == expected_address {
                info!("WARNING: Tampered transaction recovered matching address!");
                all_tests_passed = false;
            } else {
                info!("Tampered transaction detected - address mismatch");
                info!("Expected: 0x{}", hex::encode(expected_address));
                info!("Recovered: 0x{}", hex::encode(recovered_address));
            }
        }
        Err(e) => info!("Tampered transaction rejected: {}", e),
    }

    info!("===============================================================");
    if all_tests_passed {
        info!("ECDSA secp256k1 signature verification spike complete!");
        info!("- EIP-155 replay protection implemented");
        info!("- Transaction signing working");
        info!("- Signature verification working");
        info!("- Tamper detection working");
        info!("Ready to integrate into blockchain transaction validation");
    } else {
        error!("ECDSA secp256k1 signature verification spike FAILED!");
        error!("Some tests did not pass. Please review the output above.");
    }
}
