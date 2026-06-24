//! Integration tests for Multi-Signature functionality

#[cfg(test)]
mod integration_tests {
    use crate::account_abstraction::{
        MultiSigManager, MultiSigTransaction, WalletFactory, WalletRegistry,
    };
    use crate::blockchain::Transaction;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn sign_for(tx_hash: [u8; 32], signer: [u8; 20]) -> (Vec<u8>, Vec<u8>) {
        let mut seed = [0u8; 32];
        seed[..20].copy_from_slice(&signer);
        seed[20..].copy_from_slice(&signer[..12]);

        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&tx_hash).to_bytes().to_vec();
        let public_key = signing_key.verifying_key().to_bytes().to_vec();
        (signature, public_key)
    }

    #[tokio::test]
    async fn test_multisig_transaction_flow() {
        // Setup: Create wallet registry and multi-sig wallet
        let mut registry = WalletRegistry::new();
        let owner = [1u8; 20];
        let signers = vec![[2u8; 20], [3u8; 20], [4u8; 20]];
        let threshold = 2;

        let wallet =
            WalletFactory::create_multisig_wallet(owner, 0, signers.clone(), threshold).unwrap();
        registry.register_wallet(wallet.clone()).unwrap();

        // Create transaction
        let to = Address([5u8; 20]);
        let tx = Transaction::new(wallet.address, to, 1000, 100, 0);

        // Create multi-sig transaction
        let mut multisig_tx =
            MultiSigTransaction::new(wallet.address, tx, signers.clone(), threshold).unwrap();
        let tx_hash: [u8; 32] = multisig_tx.transaction.hash.into();

        // Initially not ready
        assert!(!multisig_tx.is_ready());
        assert_eq!(multisig_tx.signature_count(), 0);

        // Add first signature
        let (sig1, pk1) = sign_for(tx_hash, [2u8; 20]);
        assert!(multisig_tx.add_signature([2u8; 20], sig1, pk1).is_ok());
        assert_eq!(multisig_tx.signature_count(), 1);
        assert!(!multisig_tx.is_ready()); // Still need one more

        // Add second signature
        let (sig2, pk2) = sign_for(tx_hash, [3u8; 20]);
        assert!(multisig_tx.add_signature([3u8; 20], sig2, pk2).is_ok());
        assert_eq!(multisig_tx.signature_count(), 2);
        assert!(multisig_tx.is_ready()); // Now ready!
    }

    #[tokio::test]
    async fn test_multisig_manager_tracking() {
        let mut manager = MultiSigManager::new();
        let wallet = Address([1u8; 20]);
        let signers = vec![[2u8; 20], [3u8; 20]];
        let threshold = 2;

        // Create transaction
        let tx = Transaction::new(wallet, Address([5u8; 20]), 1000, 100, 0);
        let multisig_tx = MultiSigTransaction::new(wallet, tx, signers, threshold).unwrap();

        // Add to manager
        manager.add_pending_transaction(multisig_tx);

        // Get pending transactions
        let pending = manager.get_pending_transactions(&wallet);
        assert_eq!(pending.len(), 1);

        // Add signature
        let tx_hash = pending[0].transaction.hash;
        let tx_hash_bytes: [u8; 32] = tx_hash.into();
        let (sig1, pk1) = sign_for(tx_hash_bytes, [2u8; 20]);
        assert!(manager
            .add_signature_to_pending(&wallet, &tx_hash, [2u8; 20], sig1, pk1)
            .is_ok());

        // Verify signature was added
        let pending = manager.get_pending_transactions(&wallet);
        assert_eq!(pending[0].signature_count(), 1);
    }

    #[tokio::test]
    async fn test_multisig_validation_errors() {
        let wallet = Address([1u8; 20]);
        let signers = vec![[2u8; 20], [3u8; 20], [4u8; 20]];
        let threshold = 2;

        let tx = Transaction::new(wallet, Address([5u8; 20]), 1000, 100, 0);
        let mut multisig_tx = MultiSigTransaction::new(wallet, tx, signers, threshold).unwrap();
        let tx_hash: [u8; 32] = multisig_tx.transaction.hash.into();

        // Try to add signature from unknown signer
        assert!(multisig_tx
            .add_signature([99u8; 20], vec![1; 64], vec![2; 32])
            .is_err());

        // Add valid signature
        let (sig1, pk1) = sign_for(tx_hash, [2u8; 20]);
        assert!(multisig_tx.add_signature([2u8; 20], sig1, pk1).is_ok());

        // Try to add duplicate signature
        assert!(multisig_tx
            .add_signature([2u8; 20], vec![3; 64], vec![4; 32])
            .is_err());
    }

    #[tokio::test]
    async fn test_multisig_pending_signers() {
        let wallet = Address([1u8; 20]);
        let signers = vec![[2u8; 20], [3u8; 20], [4u8; 20]];
        let threshold = 2;

        let tx = Transaction::new(wallet, Address([5u8; 20]), 1000, 100, 0);
        let mut multisig_tx =
            MultiSigTransaction::new(wallet, tx, signers.clone(), threshold).unwrap();
        let tx_hash: [u8; 32] = multisig_tx.transaction.hash.into();

        // Initially all signers are pending
        let pending = multisig_tx.pending_signers();
        assert_eq!(pending.len(), 3);

        // Add one signature
        let (sig1, pk1) = sign_for(tx_hash, [2u8; 20]);
        multisig_tx.add_signature([2u8; 20], sig1, pk1).unwrap();
        let pending = multisig_tx.pending_signers();
        assert_eq!(pending.len(), 2);
        assert!(!pending.contains(&[2u8; 20]));
        assert!(pending.contains(&[3u8; 20]));
        assert!(pending.contains(&[4u8; 20]));

        // Check signed_by
        let signed = multisig_tx.signed_by();
        assert_eq!(signed.len(), 1);
        assert_eq!(signed[0], [2u8; 20]);
    }
}
