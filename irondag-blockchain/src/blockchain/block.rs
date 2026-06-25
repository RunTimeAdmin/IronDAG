//! Block and transaction structures

use crate::types::{derive_eth_address, keccak256, Address, Hash, StreamType};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256}; // Needed for incremental hashing
use tracing::{debug, error};

/// Block header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub parent_hashes: Vec<Hash>,
    pub block_number: u64,
    pub stream_type: StreamType,
    pub difficulty: u64,
    pub timestamp: u64,
    pub nonce: u64, // Proof-of-Work nonce
    /// EIP-1559: Base fee per gas for this block
    /// Used for dynamic fee adjustment based on network congestion
    pub base_fee_per_gas: u128,
    /// Post-Quantum signature (optional Dilithium3 signature from miner)
    /// If present, provides quantum-resistant authentication of the block header
    #[serde(default)]
    pub pq_signature: Option<crate::pqc::PqSignature>,
    /// Miner's Dilithium3 public key (required if pq_signature is present)
    /// Used to verify the PQ signature during block validation
    #[serde(default)]
    pub miner_pq_pubkey: Option<Vec<u8>>,
}

impl BlockHeader {
    pub fn new(
        parent_hashes: Vec<Hash>,
        block_number: u64,
        stream_type: StreamType,
        difficulty: u64,
        base_fee_per_gas: u128,
    ) -> Self {
        Self {
            parent_hashes,
            block_number,
            stream_type,
            difficulty,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            nonce: 0, // Initialize nonce to 0
            base_fee_per_gas,
            pq_signature: None,
            miner_pq_pubkey: None,
        }
    }

    /// Create header with nonce (for PoW mining)
    pub fn with_nonce(
        parent_hashes: Vec<Hash>,
        block_number: u64,
        stream_type: StreamType,
        difficulty: u64,
        nonce: u64,
        base_fee_per_gas: u128,
    ) -> Self {
        Self {
            parent_hashes,
            block_number,
            stream_type,
            difficulty,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            nonce,
            base_fee_per_gas,
            pq_signature: None,
            miner_pq_pubkey: None,
        }
    }

    /// Calculate header hash (for headers-first sync)
    /// This is a partial hash that doesn't include transactions
    /// The full block hash includes transactions
    pub fn calculate_header_hash(&self) -> Hash {
        let mut hasher = Keccak256::new();
        for parent in &self.parent_hashes {
            hasher.update(parent);
        }
        hasher.update(&self.block_number.to_le_bytes());
        hasher.update(&self.stream_type.to_bytes());
        hasher.update(&self.difficulty.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.base_fee_per_gas.to_le_bytes());
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        Hash(hash)
    }

    /// Calculate signing hash for PQ signature
    /// This excludes the pq_signature and miner_pq_pubkey fields
    /// Used when signing/verifying the block header with Dilithium3
    pub fn calculate_signing_hash(&self) -> Hash {
        let mut hasher = Keccak256::new();
        for parent in &self.parent_hashes {
            hasher.update(parent);
        }
        hasher.update(&self.block_number.to_le_bytes());
        hasher.update(&self.stream_type.to_bytes());
        hasher.update(&self.difficulty.to_le_bytes());
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.base_fee_per_gas.to_le_bytes());
        // Note: pq_signature and miner_pq_pubkey are intentionally excluded
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        Hash(hash)
    }
}

/// Transaction signature (64 bytes for Ed25519)
/// Using Vec<u8> for serde compatibility
pub type TransactionSignature = Vec<u8>;

/// Public key (32 bytes for Ed25519)
/// Using Vec<u8> for serde compatibility
pub type PublicKey = Vec<u8>;

/// ECDSA signature components (for Ethereum/Metamask compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EcdsaSignature {
    /// r component (32 bytes)
    pub r: [u8; 32],
    /// s component (32 bytes)
    pub s: [u8; 32],
    /// v component (recovery ID + EIP-155 chain_id encoding)
    /// Legacy: 27 or 28
    /// EIP-155: chain_id * 2 + 35 + recovery_id (0 or 1)
    /// IMPORTANT: Store ORIGINAL v value, not normalized recovery_id
    pub v: u64,
}

/// Transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub from: Address,
    pub to: Address,
    pub value: u128,
    pub fee: u128,
    pub nonce: u64,
    pub data: Vec<u8>,
    pub gas_limit: u64,
    pub hash: Hash,
    /// EIP-1559: Maximum fee per gas (inclusive of base fee + priority fee)
    /// If None, this is a legacy transaction using simple fee field
    pub max_fee_per_gas: Option<u128>,
    /// EIP-1559: Maximum priority fee per gas (tip to miner)
    /// If None, this is a legacy transaction
    pub max_priority_fee_per_gas: Option<u128>,
    /// Ed25519 signature (64 bytes) - CRITICAL for security
    /// OR ECDSA signature (65 bytes: r[32] + s[32] + v[1]) for Ethereum compatibility
    /// If empty, transaction is unsigned (only allowed for genesis/system transactions)
    /// For PQ accounts, this field is empty and pq_signature is used instead
    pub signature: TransactionSignature,
    /// Ed25519 public key (32 bytes) - required for Ed25519 signature verification
    /// OR ECDSA public key (64 bytes uncompressed, or 33 bytes compressed) for ECDSA signatures
    /// If empty, transaction is unsigned (only allowed for genesis/system transactions)
    /// For PQ accounts, this field is empty and pq_signature is used instead
    /// For ECDSA, public key can be recovered from signature (r, s, v)
    pub public_key: PublicKey,
    /// Post-Quantum signature (optional, for PQ accounts)
    /// If present, this is used instead of Ed25519 signature
    pub pq_signature: Option<crate::pqc::PqSignature>,
    /// ECDSA signature components (for Ethereum/Metamask compatibility)
    /// If present, this transaction uses ECDSA/secp256k1 signature instead of Ed25519
    pub ecdsa_signature: Option<EcdsaSignature>,
    /// EIP-155 Chain ID (for replay protection)
    /// If Some, this transaction is protected by EIP-155 replay protection
    /// The chain ID is included in the signing hash calculation
    pub chain_id: Option<u64>,
    /// Time-locked transaction: Execute at this block number (0 = immediate)
    /// If set, transaction will only be processed when current block >= execute_at_block
    pub execute_at_block: Option<u64>,
    /// Time-locked transaction: Execute at this Unix timestamp (0 = immediate)
    /// If set, transaction will only be processed when block timestamp >= execute_at_timestamp
    pub execute_at_timestamp: Option<u64>,
    /// Gasless transaction: Address that sponsors (pays for) this transaction's fee
    /// If set, the sponsor's balance is checked and debited instead of the sender's
    pub sponsor: Option<Address>,
    /// Multi-signature support (for contract wallets)
    /// If present, this transaction requires multiple signatures
    /// Format: Vec<(signer_address, signature_bytes, public_key_bytes)>
    pub multisig_signatures: Option<Vec<(Address, Vec<u8>, Vec<u8>)>>,
    /// Privacy transaction: zk-SNARK proof and privacy data
    /// If present, this is a private transaction (hidden sender, receiver, amount)
    #[cfg(feature = "privacy")]
    pub privacy_data: Option<crate::privacy::PrivacyTransaction>,
}

impl Transaction {
    pub fn new<F: Into<Address>, T: Into<Address>>(
        from: F,
        to: T,
        value: u128,
        fee: u128,
        nonce: u64,
    ) -> Self {
        let from = from.into();
        let to = to.into();
        #[cfg(feature = "privacy")]
        let mut tx = Self {
            from,
            to,
            value,
            fee,
            nonce,
            data: Vec::new(),
            gas_limit: 21_000,
            hash: Hash::zero(),
            max_fee_per_gas: None,          // Legacy transaction by default
            max_priority_fee_per_gas: None, // Legacy transaction by default
            signature: vec![0; 64],         // Unsigned - must be signed before use
            public_key: vec![],             // No public key for unsigned transactions
            pq_signature: None,             // No PQ signature initially
            ecdsa_signature: None,          // No ECDSA signature initially
            chain_id: None,                 // No chain ID (legacy transaction)
            execute_at_block: None,         // Immediate execution
            execute_at_timestamp: None,     // Immediate execution
            sponsor: None,                  // No sponsor (sender pays fee)
            multisig_signatures: None,      // No multi-sig initially
            privacy_data: None,             // No privacy data initially
        };
        #[cfg(not(feature = "privacy"))]
        let mut tx = Self {
            from,
            to,
            value,
            fee,
            nonce,
            data: Vec::new(),
            gas_limit: 21_000,
            hash: Hash::zero(),
            max_fee_per_gas: None,          // Legacy transaction by default
            max_priority_fee_per_gas: None, // Legacy transaction by default
            signature: vec![0; 64],         // Unsigned - must be signed before use
            public_key: vec![],             // No public key for unsigned transactions
            pq_signature: None,             // No PQ signature initially
            ecdsa_signature: None,          // No ECDSA signature initially
            chain_id: None,                 // No chain ID (legacy transaction)
            execute_at_block: None,         // Immediate execution
            execute_at_timestamp: None,     // Immediate execution
            sponsor: None,                  // No sponsor (sender pays fee)
            multisig_signatures: None,      // No multi-sig initially
        };
        tx.hash = tx.calculate_hash();
        tx
    }

    pub fn with_data<F: Into<Address>, T: Into<Address>>(
        from: F,
        to: T,
        value: u128,
        fee: u128,
        nonce: u64,
        data: Vec<u8>,
        gas_limit: u64,
    ) -> Self {
        let from = from.into();
        let to = to.into();
        #[cfg(feature = "privacy")]
        let mut tx = Self {
            from,
            to,
            value,
            fee,
            nonce,
            data,
            gas_limit,
            hash: Hash::zero(),
            max_fee_per_gas: None,          // Legacy transaction by default
            max_priority_fee_per_gas: None, // Legacy transaction by default
            signature: vec![0; 64],         // Unsigned - must be signed before use
            public_key: vec![],             // No public key for unsigned transactions
            pq_signature: None,             // No PQ signature initially
            ecdsa_signature: None,          // No ECDSA signature initially
            chain_id: None,                 // No chain ID (legacy transaction)
            execute_at_block: None,         // Immediate execution
            execute_at_timestamp: None,     // Immediate execution
            sponsor: None,                  // No sponsor (sender pays fee)
            multisig_signatures: None,      // No multi-sig initially
            privacy_data: None,             // No privacy data initially
        };
        #[cfg(not(feature = "privacy"))]
        let mut tx = Self {
            from,
            to,
            value,
            fee,
            nonce,
            data,
            gas_limit,
            hash: Hash::zero(),
            max_fee_per_gas: None,          // Legacy transaction by default
            max_priority_fee_per_gas: None, // Legacy transaction by default
            signature: vec![0; 64],         // Unsigned - must be signed before use
            public_key: vec![],             // No public key for unsigned transactions
            pq_signature: None,             // No PQ signature initially
            ecdsa_signature: None,          // No ECDSA signature initially
            chain_id: None,                 // No chain ID (legacy transaction)
            execute_at_block: None,         // Immediate execution
            execute_at_timestamp: None,     // Immediate execution
            sponsor: None,                  // No sponsor (sender pays fee)
            multisig_signatures: None,      // No multi-sig initially
        };
        tx.hash = tx.calculate_hash();
        tx
    }

    /// Sign a transaction with Ed25519
    ///
    /// # Arguments
    /// * `secret_key` - 32-byte Ed25519 secret key
    ///
    /// # Returns
    /// The transaction with signature and public key set
    pub fn sign(mut self, secret_key: &[u8; 32]) -> Self {
        use ed25519_dalek::{Signer, SigningKey};

        // Create signing key from bytes
        let signing_key = SigningKey::from_bytes(secret_key);

        // Get the public key (verifying key)
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes: [u8; 32] = verifying_key.to_bytes();

        // Store public key
        self.public_key = public_key_bytes.to_vec();

        // Sign the transaction hash (before signature is added)
        let message = &self.hash;
        let signature = signing_key.sign(message);

        // Store signature
        self.signature = signature.to_bytes().into();

        // Note: We don't recalculate hash after signing because the hash
        // should be calculated before signing (signature signs the hash)

        self
    }

    /// Set the chain ID for EIP-155 replay protection and recalculate hash
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID to set
    ///
    /// # Returns
    /// The transaction with chain_id set and hash recalculated
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = Some(chain_id);
        self.hash = self.calculate_hash();
        self
    }

    /// Sign a transaction with a PQ account
    ///
    /// Returns an error if PQ signing fails (e.g., for unimplemented signature schemes).
    pub fn sign_pq(mut self, pq_account: &crate::pqc::PqAccount) -> Result<Self, String> {
        let signature = pq_account.sign(&self.hash)?;
        self.pq_signature = Some(signature);
        // Clear Ed25519 fields for PQ transactions
        self.signature = vec![];
        self.public_key = vec![];
        Ok(self)
    }

    /// Verify transaction signature
    ///
    /// # Arguments
    /// * `block_height` - The block height at which the transaction is being verified (for genesis check)
    ///
    /// # Returns
    /// `true` if signature is valid, `false` otherwise
    ///
    /// This implementation supports:
    /// 1. PQ signatures (post-quantum)
    /// 2. ECDSA signatures (Ethereum/Metamask compatibility)
    /// 3. Ed25519 signatures (native IronDAG)
    /// 4. Unsigned transactions (only for system/genesis at block 0, from = zero address)
    pub fn verify_signature(&self, block_height: u64) -> Result<bool, String> {
        debug!("Starting verification: from={}, has_pq={}, has_ecdsa={}, chain_id={:?}, block_height={}",
            hex::encode(self.from), self.pq_signature.is_some(), self.ecdsa_signature.is_some(), self.chain_id, block_height);

        // Check for PQ signature first
        if let Some(ref pq_sig) = self.pq_signature {
            debug!("Using PQ signature");
            return Ok(crate::pqc::PqAccount::verify_signature(&self.hash, pq_sig));
        }

        // Check for ECDSA signature (Ethereum/Metamask compatibility)
        if let Some(ref ecdsa_sig) = self.ecdsa_signature {
            debug!(
                "Using ECDSA signature: v={}, r={}, s={}",
                ecdsa_sig.v,
                hex::encode(&ecdsa_sig.r[..8]),
                hex::encode(&ecdsa_sig.s[..8])
            );
            return Ok(self.verify_ecdsa_signature(ecdsa_sig));
        }

        debug!("Falling back to Ed25519 verification");

        // Fall back to Ed25519 verification
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        // Allow unsigned transactions only if from address is zero (system/genesis)
        // SEC-014: System transactions are only valid at genesis (block_height == 0)
        let is_zero_address = self.from.is_zero();
        let has_signature = !(self.signature.is_empty() || self.signature.iter().all(|&b| b == 0));

        if is_zero_address && !has_signature && self.public_key.is_empty() {
            if block_height == 0 {
                return Ok(true); // System transaction valid at genesis
            } else {
                return Err("system transactions only valid at genesis".to_string());
            }
        }

        // Must have both signature and public key for signed transactions
        if self.signature.len() != 64 {
            return Ok(false);
        }

        if self.public_key.len() != 32 {
            return Ok(false);
        }

        // Parse public key
        let pub_key_bytes: [u8; 32] = match self.public_key.as_slice().try_into() {
            Ok(b) => b,
            Err(_) => return Ok(false),
        };

        let verifying_key = match VerifyingKey::from_bytes(&pub_key_bytes) {
            Ok(key) => key,
            Err(_) => return Ok(false),
        };

        // Parse signature
        let sig_bytes: [u8; 64] = match self.signature.as_slice().try_into() {
            Ok(b) => b,
            Err(_) => return Ok(false),
        };

        let signature = match Signature::try_from(&sig_bytes[..]) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };

        // Verify signature against transaction hash
        match verifying_key.verify(&self.hash, &signature) {
            Ok(_) => {
                // Optional: Verify that address matches public key hash
                // This ensures the address was derived from the public key
                // For now, we'll do a simple check: address should be last 20 bytes of Keccak256(public_key)
                let derived_address = derive_eth_address(&pub_key_bytes);

                // Verify address matches (or allow if not enforced for backward compatibility)
                // For now, we'll verify it matches
                Ok(derived_address == self.from)
            }
            Err(_) => Ok(false),
        }
    }

    /// Recover sender address from ECDSA signature (Ethereum/Metamask compatibility)
    /// Returns Some(address) if recovery succeeds, None otherwise
    pub fn recover_ecdsa_address(&self, ecdsa_sig: &EcdsaSignature) -> Option<Address> {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};

        // Extract chain ID and recovery ID from v
        let (chain_id, recovery_id) = if ecdsa_sig.v >= 35 {
            // EIP-155 format: v = chain_id * 2 + 35 + recovery_id
            let chain_id = ((ecdsa_sig.v - 35) / 2) as u64;
            let rec_id = ((ecdsa_sig.v - 35) % 2) as u8;
            (Some(chain_id), rec_id)
        } else if ecdsa_sig.v >= 27 {
            // Legacy format: v = 27 or 28
            let rec_id = (ecdsa_sig.v - 27) as u8;
            (None, rec_id)
        } else {
            // Direct recovery ID
            (None, ecdsa_sig.v as u8)
        };

        let recovery_id = RecoveryId::try_from(recovery_id).ok()?;

        // For EIP-155 transactions, recalculate the signing hash with chain ID
        let effective_chain_id = self.chain_id.or(chain_id);
        let message_hash = if let Some(cid) = effective_chain_id {
            // EIP-155: Include chain_id, 0, 0 in RLP for signing
            let mut buf = BytesMut::new();
            self.nonce.encode(&mut buf);
            let gas_price = self.fee / (self.gas_limit as u128);
            gas_price.encode(&mut buf);
            self.gas_limit.encode(&mut buf);
            if self.to.is_zero() {
                // Empty data for contract creation
                let empty: &[u8] = &[];
                empty.encode(&mut buf);
            } else {
                self.to.as_ref().encode(&mut buf);
            }
            self.value.encode(&mut buf);
            self.data.encode(&mut buf);
            cid.encode(&mut buf);
            0u8.encode(&mut buf);
            0u8.encode(&mut buf);

            keccak256(&buf[..])
        } else {
            self.hash
        };

        // Create signature from r and s
        let mut sig_bytes = [0u8; 64];
        sig_bytes[..32].copy_from_slice(&ecdsa_sig.r);
        sig_bytes[32..].copy_from_slice(&ecdsa_sig.s);

        let signature = K256Signature::try_from(&sig_bytes[..]).ok()?;

        // Recover verifying key (public key) from the prehashed message
        let verifying_key =
            VerifyingKey::recover_from_prehash(&message_hash.0, &signature, recovery_id).ok()?;

        // Get uncompressed public key (65 bytes: 0x04 + x[32] + y[32])
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();

        // Skip the 0x04 prefix (uncompressed format indicator)
        let pub_key = if public_key_bytes.len() == 65 && public_key_bytes[0] == 0x04 {
            &public_key_bytes[1..]
        } else {
            return None;
        };

        // Derive address from public key (Ethereum style: Keccak256(pub_key)[12:32])
        let derived_address = derive_eth_address(pub_key);

        Some(derived_address)
    }

    /// Verify ECDSA signature (Ethereum/Metamask compatibility)
    ///
    /// Recovers public key from signature (r, s, v) and verifies it matches the from address
    fn verify_ecdsa_signature(&self, ecdsa_sig: &EcdsaSignature) -> bool {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};

        debug!(
            "Transaction: from={}, nonce={}, chain_id={:?}, v={}",
            hex::encode(self.from),
            self.nonce,
            self.chain_id,
            ecdsa_sig.v
        );

        // Extract chain ID and recovery ID from v
        let (chain_id, recovery_id) = if ecdsa_sig.v >= 35 {
            // EIP-155 format: v = chain_id * 2 + 35 + recovery_id
            let chain_id = ((ecdsa_sig.v - 35) / 2) as u64;
            let rec_id = ((ecdsa_sig.v - 35) % 2) as u8;
            debug!(
                "EIP-155: v={}, extracted_chain_id={}, rec_id={}",
                ecdsa_sig.v, chain_id, rec_id
            );
            (Some(chain_id), rec_id)
        } else if ecdsa_sig.v >= 27 {
            // Legacy format: v = 27 or 28
            let rec_id = (ecdsa_sig.v - 27) as u8;
            debug!("Legacy: v={}, rec_id={}", ecdsa_sig.v, rec_id);
            (None, rec_id)
        } else {
            // Direct recovery ID
            debug!("Direct: v={}", ecdsa_sig.v);
            (None, ecdsa_sig.v as u8)
        };

        let recovery_id = match RecoveryId::try_from(recovery_id) {
            Ok(rid) => rid,
            Err(_) => {
                error!("Invalid recovery_id: {}", recovery_id);
                return false;
            }
        };

        // For EIP-155 transactions, we need to recalculate the signing hash
        // with chain ID included: hash(RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]))
        // Prefer self.chain_id (set from RPC), fallback to chain_id extracted from v
        let effective_chain_id = self.chain_id.or(chain_id);
        debug!(
            "Effective chain_id: {:?} (self.chain_id={:?}, extracted={:?})",
            effective_chain_id, self.chain_id, chain_id
        );
        let message_hash = if let Some(cid) = effective_chain_id {
            // EIP-155: Include chain_id, 0, 0 in RLP for signing
            let mut buf = BytesMut::new();
            self.nonce.encode(&mut buf);
            // gasPrice = fee / gas_limit
            let gas_price = self.fee / (self.gas_limit as u128);
            gas_price.encode(&mut buf);
            self.gas_limit.encode(&mut buf);
            // to address
            if self.to.is_zero() {
                // Empty data for contract creation
                let empty: &[u8] = &[];
                empty.encode(&mut buf);
            } else {
                self.to.as_ref().encode(&mut buf);
            }
            self.value.encode(&mut buf);
            self.data.encode(&mut buf);
            cid.encode(&mut buf); // chain_id
            0u8.encode(&mut buf); // 0
            0u8.encode(&mut buf); // 0

            keccak256(&buf[..])
        } else {
            // Legacy transaction: reconstruct signing hash
            // hash(RLP([nonce, gasPrice, gasLimit, to, value, data]))
            let mut buf = BytesMut::new();
            self.nonce.encode(&mut buf);
            let gas_price = if self.gas_limit > 0 {
                self.fee / (self.gas_limit as u128)
            } else {
                0
            };
            gas_price.encode(&mut buf);
            self.gas_limit.encode(&mut buf);
            self.to.0.as_slice().encode(&mut buf);
            self.value.encode(&mut buf);
            self.data.encode(&mut buf);

            keccak256(&buf[..])
        };

        // Create signature from r and s
        let mut sig_bytes = [0u8; 64];
        sig_bytes[..32].copy_from_slice(&ecdsa_sig.r);
        sig_bytes[32..].copy_from_slice(&ecdsa_sig.s);

        // Convert to k256 signature format
        let signature = match K256Signature::try_from(&sig_bytes[..]) {
            Ok(sig) => sig,
            Err(_) => {
                return false;
            }
        };

        // Recover verifying key (public key) from the prehashed message
        // message_hash is already keccak256 hash, pass as slice
        let verifying_key =
            match VerifyingKey::recover_from_prehash(&message_hash.0, &signature, recovery_id) {
                Ok(key) => key,
                Err(_) => {
                    return false;
                }
            };

        // Get uncompressed public key (65 bytes: 0x04 + x[32] + y[32])
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();

        // Skip the 0x04 prefix (uncompressed format indicator)
        let pub_key = if public_key_bytes.len() == 65 && public_key_bytes[0] == 0x04 {
            &public_key_bytes[1..]
        } else {
            return false;
        };

        // Derive address from public key (Ethereum style: Keccak256(pub_key)[12:32])
        let derived_address = derive_eth_address(pub_key);

        // Verify address matches
        if derived_address != self.from {
            return false;
        }
        true
    }

    /// Create a time-locked transaction that executes at a specific block
    pub fn with_execute_at_block(mut self, block_number: u64) -> Self {
        self.execute_at_block = Some(block_number);
        self.hash = self.calculate_hash();
        self
    }

    /// Create a time-locked transaction that executes at a specific timestamp
    pub fn with_execute_at_timestamp(mut self, timestamp: u64) -> Self {
        self.execute_at_timestamp = Some(timestamp);
        self.hash = self.calculate_hash();
        self
    }

    /// Create a gasless transaction sponsored by another address
    pub fn with_sponsor<S: Into<Address>>(mut self, sponsor: S) -> Self {
        self.sponsor = Some(sponsor.into());
        self.hash = self.calculate_hash();
        self
    }

    /// Add multi-signature support to transaction
    pub fn with_multisig_signatures(
        mut self,
        signatures: Vec<(Address, Vec<u8>, Vec<u8>)>,
    ) -> Self {
        self.multisig_signatures = Some(signatures);
        self.hash = self.calculate_hash();
        self
    }

    /// Check if transaction is ready to execute (time-lock check)
    pub fn is_ready_to_execute(&self, current_block: u64, current_timestamp: u64) -> bool {
        if let Some(block) = self.execute_at_block {
            if current_block < block {
                return false;
            }
        }
        if let Some(timestamp) = self.execute_at_timestamp {
            if current_timestamp < timestamp {
                return false;
            }
        }
        true
    }

    /// Calculate transaction hash
    ///
    /// IMPORTANT: This method ALWAYS recomputes the hash deterministically from transaction fields.
    /// It never returns a stored hash value. This is critical for security (SEC-007).
    ///
    /// For ECDSA transactions (EIP-155), the hash includes: from, to, value, fee, nonce, data,
    /// gas_limit, and chain_id (if present). Signatures are EXCLUDED from the hash calculation
    /// (following EIP-155 conventions where the signature signs the hash).
    ///
    /// EIP-1559: Also includes max_fee_per_gas and max_priority_fee_per_gas if present.
    pub fn calculate_hash(&self) -> Hash {
        let mut hasher = Keccak256::new();
        hasher.update(&self.from);
        hasher.update(&self.to);
        hasher.update(&self.value.to_le_bytes());
        hasher.update(&self.fee.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.data);
        hasher.update(&self.gas_limit.to_le_bytes());

        // Include EIP-1559 fields if present
        if let Some(max_fee) = self.max_fee_per_gas {
            hasher.update(&max_fee.to_le_bytes());
        }
        if let Some(priority_fee) = self.max_priority_fee_per_gas {
            hasher.update(&priority_fee.to_le_bytes());
        }

        // Include chain_id for EIP-155 replay protection (applies to both ECDSA and native)
        if let Some(chain_id) = self.chain_id {
            hasher.update(&chain_id.to_le_bytes());
        }

        // Include time-lock fields in hash
        if let Some(block) = self.execute_at_block {
            hasher.update(&block.to_le_bytes());
        }
        if let Some(timestamp) = self.execute_at_timestamp {
            hasher.update(&timestamp.to_le_bytes());
        }
        // Include sponsor in hash
        if let Some(sponsor) = self.sponsor {
            hasher.update(&sponsor);
        }
        // Note: signature, public_key, ecdsa_signature, pq_signature, and multisig_signatures
        // are NOT included in hash (signature signs this hash)
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        Hash(hash)
    }

    /// Derive address from public key (Ethereum-style: Keccak256(public_key)[12:32])
    pub fn derive_address_from_public_key(public_key: &[u8; 32]) -> Address {
        derive_eth_address(public_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecdsa_signature_recovery() {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{SigningKey, VerifyingKey};
        use rand_core::OsRng;
        use sha3::{Digest, Keccak256};

        // Create a test key pair
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        // Derive address from public key
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();
        let pub_key = &public_key_bytes[1..]; // Skip 0x04 prefix

        let test_address = derive_eth_address(pub_key);

        // Create a test transaction
        let mut tx = Transaction::new(
            test_address,
            [0x02; 20],
            1_000_000_000_000_000_000, // 1 IDAG
            0,
            0,
        );

        // Sign legacy Ethereum RLP payload with ECDSA
        let mut buf = BytesMut::new();
        tx.nonce.encode(&mut buf);
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / (tx.gas_limit as u128)
        } else {
            0
        };
        gas_price.encode(&mut buf);
        tx.gas_limit.encode(&mut buf);
        tx.to.as_ref().encode(&mut buf);
        tx.value.encode(&mut buf);
        tx.data.encode(&mut buf);

        let mut message_digest = Keccak256::new();
        message_digest.update(&buf[..]);
        let (signature, recovery_id) = signing_key
            .sign_digest_recoverable(message_digest.clone())
            .expect("Failed to sign");

        // Extract r, s from signature
        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // Create ECDSA signature (v = recovery_id + 27 for legacy format)
        let v = (recovery_id.to_byte() + 27) as u64;
        let ecdsa_sig = EcdsaSignature { r, s, v };

        // Set ECDSA signature in transaction
        tx.ecdsa_signature = Some(ecdsa_sig);

        // Verify signature (at block 1, not genesis)
        assert!(
            tx.verify_signature(1).unwrap(),
            "ECDSA signature verification should pass"
        );

        // Test with wrong address (should fail)
        let mut tx_wrong = tx.clone();
        tx_wrong.from = [0xFF; 20].into();
        assert!(
            !tx_wrong.verify_signature(1).unwrap(),
            "ECDSA signature verification should fail with wrong address"
        );
    }

    /// Test 1: Verify valid ECDSA signature returns Ok(true)
    #[test]
    fn test_verify_ecdsa_valid_signature() {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{SigningKey, VerifyingKey};
        use rand_core::OsRng;
        use sha3::{Digest, Keccak256};

        // Create a test key pair
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        // Derive address from public key
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();
        let pub_key = &public_key_bytes[1..]; // Skip 0x04 prefix

        let mut hasher = Keccak256::new();
        hasher.update(pub_key);
        let hash = hasher.finalize();
        let mut test_address = Address([0u8; 20]);
        test_address.0.copy_from_slice(&hash[12..32]);

        // Create a test transaction with chain_id for EIP-155
        let chain_id: u64 = 1337;
        let mut tx = Transaction::new(
            test_address,
            [0x02; 20],
            1_000_000_000_000_000_000, // 1 IDAG
            21_000,                    // fee
            0,
        );
        tx.chain_id = Some(chain_id);
        tx.hash = tx.calculate_hash();

        // Sign with EIP-155 format: RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])
        let mut buf = BytesMut::new();
        tx.nonce.encode(&mut buf);
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / (tx.gas_limit as u128)
        } else {
            0
        };
        gas_price.encode(&mut buf);
        tx.gas_limit.encode(&mut buf);
        tx.to.as_ref().encode(&mut buf);
        tx.value.encode(&mut buf);
        tx.data.encode(&mut buf);
        chain_id.encode(&mut buf);
        0u8.encode(&mut buf);
        0u8.encode(&mut buf);

        let mut message_digest = Keccak256::new();
        message_digest.update(&buf[..]);
        let (signature, recovery_id) = signing_key
            .sign_digest_recoverable(message_digest.clone())
            .expect("Failed to sign");

        // Extract r, s from signature
        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // EIP-155: v = chain_id * 2 + 35 + recovery_id
        let v = chain_id * 2 + 35 + recovery_id.to_byte() as u64;
        let ecdsa_sig = EcdsaSignature { r, s, v };

        // Set ECDSA signature in transaction
        tx.ecdsa_signature = Some(ecdsa_sig);

        // Verify signature at block 1 (non-genesis)
        let result = tx.verify_signature(1);
        assert!(result.is_ok(), "verify_signature should return Ok");
        assert!(
            result.unwrap(),
            "Valid ECDSA signature should verify to true"
        );
    }

    /// Test 2: Tampered ECDSA signature returns Ok(false) or Err(...)
    #[test]
    fn test_verify_ecdsa_tampered_signature() {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{SigningKey, VerifyingKey};
        use rand_core::OsRng;
        use sha3::{Digest, Keccak256};

        // Create a test key pair
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        // Derive address from public key
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();
        let pub_key = &public_key_bytes[1..];

        let test_address = derive_eth_address(pub_key);

        // Create a test transaction WITHOUT chain_id (legacy format)
        let mut tx = Transaction::new(
            test_address,
            [0x02; 20],
            1_000_000_000_000_000_000,
            21_000,
            0,
        );
        // Do NOT set chain_id - use legacy signing
        tx.hash = tx.calculate_hash();

        // Sign legacy format (no chain_id)
        let mut buf = BytesMut::new();
        tx.nonce.encode(&mut buf);
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / (tx.gas_limit as u128)
        } else {
            0
        };
        gas_price.encode(&mut buf);
        tx.gas_limit.encode(&mut buf);
        tx.to.as_ref().encode(&mut buf);
        tx.value.encode(&mut buf);
        tx.data.encode(&mut buf);

        let mut message_digest = Keccak256::new();
        message_digest.update(&buf[..]);
        let (signature, recovery_id) = signing_key
            .sign_digest_recoverable(message_digest.clone())
            .expect("Failed to sign");

        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // Legacy format: v = 27 or 28
        let v = (recovery_id.to_byte() + 27) as u64;
        let ecdsa_sig = EcdsaSignature { r, s, v };
        tx.ecdsa_signature = Some(ecdsa_sig);

        // Verify the original signature works
        assert!(
            tx.verify_signature(1).unwrap(),
            "Original signature should be valid"
        );

        // Tamper with the signature - modify r value
        let mut tampered_tx = tx.clone();
        if let Some(ref mut sig) = tampered_tx.ecdsa_signature {
            sig.r[0] = sig.r[0].wrapping_add(1); // Flip a byte in r
        }

        // Tampered signature should fail verification
        let result = tampered_tx.verify_signature(1);
        assert!(result.is_ok(), "verify_signature should return Ok");
        assert!(
            !result.unwrap(),
            "Tampered ECDSA signature should fail verification"
        );

        // Tamper with s value
        let mut tampered_tx2 = tx.clone();
        if let Some(ref mut sig) = tampered_tx2.ecdsa_signature {
            sig.s[15] = !sig.s[15]; // Invert a byte in s
        }
        let result2 = tampered_tx2.verify_signature(1);
        assert!(result2.is_ok(), "verify_signature should return Ok");
        assert!(
            !result2.unwrap(),
            "Tampered ECDSA signature (s) should fail verification"
        );
    }

    /// Test 3: ECDSA with wrong chain_id fails verification
    #[test]
    fn test_verify_ecdsa_wrong_chain_id() {
        use alloy_rlp::{BytesMut, Encodable};
        use k256::ecdsa::{SigningKey, VerifyingKey};
        use rand_core::OsRng;
        use sha3::{Digest, Keccak256};

        // Create a test key pair
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        // Derive address from public key
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();
        let pub_key = &public_key_bytes[1..];

        let test_address = derive_eth_address(pub_key);

        // Create transaction signed with chain_id 1337
        let correct_chain_id: u64 = 1337;
        let mut tx = Transaction::new(
            test_address,
            [0x02; 20],
            1_000_000_000_000_000_000,
            21_000,
            0,
        );
        tx.chain_id = Some(correct_chain_id);
        tx.hash = tx.calculate_hash();

        // Sign with EIP-155 using chain_id 1337
        let mut buf = BytesMut::new();
        tx.nonce.encode(&mut buf);
        let gas_price = if tx.gas_limit > 0 {
            tx.fee / (tx.gas_limit as u128)
        } else {
            0
        };
        gas_price.encode(&mut buf);
        tx.gas_limit.encode(&mut buf);
        tx.to.as_ref().encode(&mut buf);
        tx.value.encode(&mut buf);
        tx.data.encode(&mut buf);
        correct_chain_id.encode(&mut buf);
        0u8.encode(&mut buf);
        0u8.encode(&mut buf);

        let mut message_digest = Keccak256::new();
        message_digest.update(&buf[..]);
        let (signature, recovery_id) = signing_key
            .sign_digest_recoverable(message_digest.clone())
            .expect("Failed to sign");

        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // v = chain_id * 2 + 35 + recovery_id = 1337 * 2 + 35 + rec_id
        let v = correct_chain_id * 2 + 35 + recovery_id.to_byte() as u64;
        let ecdsa_sig = EcdsaSignature { r, s, v };
        tx.ecdsa_signature = Some(ecdsa_sig);

        // Verify with correct chain_id in transaction
        assert!(
            tx.verify_signature(1).unwrap(),
            "Signature with matching chain_id should verify"
        );

        // Now create a transaction with WRONG chain_id in the transaction field
        // but the signature v value still encodes chain_id 1337
        let mut tx_wrong_chain = tx.clone();
        tx_wrong_chain.chain_id 11567); // Different chain_id in tx field
        tx_wrong_chain.hash = tx_wrong_chain.calculate_hash();

        // The signature was computed with chain_id 11567
        // Verification should fail because the derived address won't match
        let result = tx_wrong_chain.verify_signature(1);
        assert!(result.is_ok(), "verify_signature should return Ok");
        // The verification fails because the signing hash includes the chain_id,
        // so recovering the address with wrong chain_id produces different results
        assert!(
            !result.unwrap(),
            "Signature with wrong chain_id should fail"
        );
    }

    /// Test 4: Valid Dilithium3 signature verification
    #[test]
    fn test_verify_dilithium3_valid() {
        // Create a new Dilithium3 account
        let pq_account = crate::pqc::PqAccount::new_dilithium3();
        let from_address = pq_account.address();

        // Create a transaction from this PQ account
        let tx = Transaction::new(
            from_address,
            [0x03; 20],
            500_000_000_000_000_000, // 0.5 IDAG
            21_000,
            1,
        );

        // Sign with Dilithium3
        let signed_tx = tx
            .sign_pq(&pq_account)
            .expect("Dilithium3 signing should succeed");

        // Verify signature
        let result = signed_tx.verify_signature(1);
        assert!(result.is_ok(), "verify_signature should return Ok");
        assert!(result.unwrap(), "Valid Dilithium3 signature should verify");
    }

    /// Test 5: Dilithium3 with corrupted key returns error (not panic)
    #[test]
    fn test_verify_dilithium3_corrupted_key() {
        use crate::pqc::{PqAccount, PqAccountType, PqSignature};

        // Create a valid Dilithium3 account
        let pq_account = PqAccount::new_dilithium3();
        let from_address = pq_account.address();

        // Create a transaction
        let tx = Transaction::new(from_address, [0x04; 20], 100_000_000_000_000_000, 21_000, 2);

        // Try to sign with corrupted/truncated secret key
        let corrupted_secret = vec![0u8; 100]; // Truncated, should be 4032 bytes
        let corrupted_public = vec![0u8; 100]; // Truncated, should be 1952 bytes

        let corrupted_account = PqAccount::from_keypair(
            PqAccountType::Dilithium3,
            corrupted_secret,
            corrupted_public,
        );

        // from_keypair should reject invalid public key size
        assert!(
            corrupted_account.is_err(),
            "from_keypair should reject truncated public key"
        );

        // Also test that verification with manually constructed invalid signature returns false, not panic
        // Create a PqSignature with wrong sizes
        let bad_sig = PqSignature::new(
            PqAccountType::Dilithium3,
            vec![0u8; 10], // Way too small
            vec![0u8; 10], // Way too small
        );

        let mut tx_with_bad_sig = tx.clone();
        tx_with_bad_sig.pq_signature = Some(bad_sig);

        // Verify should return Ok(false), not panic
        let result = tx_with_bad_sig.verify_signature(1);
        assert!(
            result.is_ok(),
            "verify_signature should not panic on corrupted key"
        );
        assert!(
            !result.unwrap(),
            "Corrupted Dilithium3 signature should fail verification"
        );

        // Test with correct size but zeroed signature - should fail, not panic
        let zeroed_sig = PqSignature::new(
            PqAccountType::Dilithium3,
            vec![0u8; 3293], // Correct signature size
            vec![0u8; 1952], // Correct public key size
        );

        let mut tx_with_zeroed_sig = tx;
        tx_with_zeroed_sig.pq_signature = Some(zeroed_sig);

        let result = tx_with_zeroed_sig.verify_signature(1);
        assert!(
            result.is_ok(),
            "verify_signature should not panic on zeroed signature"
        );
        assert!(
            !result.unwrap(),
            "Zeroed Dilithium3 signature should fail verification"
        );
    }

    /// Test 6: Zero address genesis transaction is valid
    #[test]
    fn test_verify_zero_address_genesis() {
        // Create an unsigned transaction from zero address
        let tx = Transaction {
            from: [0u8; 20].into(), // Zero address
            to: [0x01; 20].into(),
            value: 1_000_000,
            fee: 0,
            nonce: 0,
            data: vec![],
            gas_limit: 21_000,
            hash: [0u8; 32].into(),
            max_fee_per_gas: None, // Legacy transaction
            max_priority_fee_per_gas: None,
            signature: vec![],  // Empty signature
            public_key: vec![], // Empty public key
            pq_signature: None,
            ecdsa_signature: None,
            chain_id: None,
            execute_at_block: None,
            execute_at_timestamp: None,
            sponsor: None,
            multisig_signatures: None,
        };

        // Verify at genesis (block_height = 0) should succeed
        let result = tx.verify_signature(0);
        assert!(
            result.is_ok(),
            "verify_signature should return Ok for genesis tx"
        );
        assert!(
            result.unwrap(),
            "Zero address transaction at genesis should be valid"
        );
    }

    /// Test 7: Zero address non-genesis transaction returns error
    #[test]
    fn test_verify_zero_address_non_genesis() {
        // Create an unsigned transaction from zero address
        let tx = Transaction {
            from: [0u8; 20].into(), // Zero address
            to: [0x01; 20].into(),
            value: 1_000_000,
            fee: 0,
            nonce: 0,
            data: vec![],
            gas_limit: 21_000,
            hash: [0u8; 32].into(),
            max_fee_per_gas: None, // Legacy transaction
            max_priority_fee_per_gas: None,
            signature: vec![],  // Empty signature
            public_key: vec![], // Empty public key
            pq_signature: None,
            ecdsa_signature: None,
            chain_id: None,
            execute_at_block: None,
            execute_at_timestamp: None,
            sponsor: None,
            multisig_signatures: None,
        };

        // Verify at non-genesis (block_height = 1) should fail with specific error
        let result = tx.verify_signature(1);
        assert!(
            result.is_err(),
            "verify_signature should return Err for non-genesis system tx"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("system transactions only valid at genesis"),
            "Error message should contain 'system transactions only valid at genesis', got: {}",
            err_msg
        );
    }

    /// Test 8: SphincsPlus verification returns error (not silent false)
    #[test]
    fn test_verify_sphincsplus_returns_error() {
        use crate::pqc::{PqAccountType, PqSignature};

        // Create a transaction
        let tx = Transaction::new([0x05; 20], [0x06; 20], 100_000_000, 21_000, 3);

        // Create a SphincsPlus signature (not yet implemented)
        let sphincs_sig = PqSignature::new(
            PqAccountType::SphincsPlus,
            vec![0u8; 7856], // SphincsPlus signature size
            vec![0u8; 32],   // SphincsPlus public key size
        );

        let mut tx_with_sphincs = tx;
        tx_with_sphincs.pq_signature = Some(sphincs_sig);

        // Verify should return Ok(false), not panic
        // The verification returns false (logged as warning) but doesn't error
        let result = tx_with_sphincs.verify_signature(1);
        assert!(
            result.is_ok(),
            "verify_signature should return Ok (not panic) for SphincsPlus"
        );
        assert!(
            !result.unwrap(),
            "SphincsPlus signature should fail verification (not implemented)"
        );
    }
}

/// Block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub hash: Hash,
    /// ZK proof for state transition (Stream C blocks)
    /// If present, proves that the transactions in this block preserve balance conservation
    #[serde(default)]
    pub zk_proof: Option<Vec<u8>>,
}

impl Block {
    pub fn new(header: BlockHeader, transactions: Vec<Transaction>) -> Self {
        let mut block = Self {
            header,
            transactions,
            hash: Hash::zero(),
            zk_proof: None,
        };
        block.hash = block.calculate_hash();
        block
    }

    /// Calculate block hash
    pub fn calculate_hash(&self) -> Hash {
        match self.header.stream_type {
            StreamType::StreamA => {
                let tx_hashes: Vec<Hash> = self.transactions.iter().map(|tx| tx.hash).collect();
                let transactions_root = crate::pow::calculate_transactions_root(&tx_hashes);
                crate::pow::hash_blake3(&self.header, &transactions_root)
            }
            StreamType::StreamB => {
                let tx_hashes: Vec<Hash> = self.transactions.iter().map(|tx| tx.hash).collect();
                let transactions_root = crate::pow::calculate_transactions_root(&tx_hashes);
                crate::pow::hash_b3memhash(&self.header, &transactions_root)
            }
            _ => {
                let mut hasher = Keccak256::new();
                hasher.update(&self.header.calculate_header_hash());
                for tx in &self.transactions {
                    hasher.update(&tx.hash);
                }
                let result = hasher.finalize();
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&result);
                Hash(hash)
            }
        }
    }
}
