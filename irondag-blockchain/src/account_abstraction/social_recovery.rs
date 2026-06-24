//! Social Recovery System for Smart Contract Wallets
//!
//! Enables wallet recovery via trusted guardians with time-delayed security.

use crate::types::{derive_eth_address, keccak256, Address};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Recovery request status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// Recovery initiated, waiting for guardian approvals
    Pending,
    /// Sufficient approvals received, waiting for time delay
    Approved,
    /// Time delay expired, recovery can be completed
    Ready,
    /// Recovery completed
    Completed,
    /// Recovery cancelled or expired
    Cancelled,
}

/// Recovery request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRequest {
    /// Wallet address being recovered
    pub wallet_address: Address,
    /// New owner address (who will control the wallet after recovery)
    pub new_owner: Address,
    /// List of guardian addresses
    pub guardians: Vec<Address>,
    /// Number of guardian approvals required
    pub recovery_threshold: u8,
    /// Guardian approvals received (guardian -> approval timestamp)
    pub approvals: HashMap<Address, u64>,
    /// Recovery initiation timestamp
    pub initiated_at: u64,
    /// Time delay in seconds (e.g., 7 days = 604800)
    pub time_delay: u64,
    /// Current status
    pub status: RecoveryStatus,
}

impl RecoveryRequest {
    /// Create a new recovery request
    pub fn new(
        wallet_address: Address,
        new_owner: Address,
        guardians: Vec<Address>,
        recovery_threshold: u8,
        time_delay: u64,
        current_timestamp: u64,
    ) -> Self {
        Self {
            wallet_address,
            new_owner,
            guardians,
            recovery_threshold,
            approvals: HashMap::new(),
            initiated_at: current_timestamp,
            time_delay,
            status: RecoveryStatus::Pending,
        }
    }

    /// Add a guardian approval (requires cryptographic signature)
    pub fn add_approval(
        &mut self,
        guardian: Address,
        signature: &[u8],
        timestamp: u64,
    ) -> Result<(), String> {
        // Check if guardian is valid
        if !self.guardians.contains(&guardian) {
            return Err("Guardian not in guardian list".to_string());
        }

        // Check if already approved
        if self.approvals.contains_key(&guardian) {
            return Err("Guardian already approved".to_string());
        }

        // Check if recovery is still pending
        if self.status != RecoveryStatus::Pending {
            return Err("Recovery is not in pending status".to_string());
        }

        // Verify guardian signature
        let mut message = b"RECOVER:".to_vec();
        message.extend_from_slice(&self.wallet_address.0);
        message.extend_from_slice(&self.new_owner.0);
        message.extend_from_slice(&timestamp.to_le_bytes());

        let message_hash = keccak256(&message);

        // Parse signature (65 bytes: r[32] + s[32] + v[1])
        if signature.len() != 65 {
            return Err("Invalid signature length".to_string());
        }

        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&signature[0..32]);
        s.copy_from_slice(&signature[32..64]);
        let v = signature[64];

        let recovered = recover_ecdsa_address(&message_hash.0, &r, &s, v)
            .ok_or("Invalid signature: could not recover address")?;

        if recovered != guardian {
            return Err("Signature does not match guardian address".to_string());
        }

        // Add approval
        self.approvals.insert(guardian, timestamp);

        // Check if threshold is met
        if self.approvals.len() >= self.recovery_threshold as usize {
            self.status = RecoveryStatus::Approved;
        }

        Ok(())
    }

    /// Check if recovery is ready to complete (time delay expired)
    pub fn is_ready(&self, current_timestamp: u64) -> bool {
        match self.status {
            RecoveryStatus::Approved => {
                let elapsed = current_timestamp.saturating_sub(self.initiated_at);
                elapsed >= self.time_delay
            }
            RecoveryStatus::Ready => true,
            _ => false,
        }
    }

    /// Update status based on current timestamp
    pub fn update_status(&mut self, current_timestamp: u64) {
        if self.status == RecoveryStatus::Approved {
            if self.is_ready(current_timestamp) {
                self.status = RecoveryStatus::Ready;
            }
        }
    }

    /// Get number of approvals received
    pub fn approval_count(&self) -> usize {
        self.approvals.len()
    }

    /// Check if threshold is met
    pub fn threshold_met(&self) -> bool {
        self.approvals.len() >= self.recovery_threshold as usize
    }

    /// Cancel the recovery request
    pub fn cancel(&mut self) {
        self.status = RecoveryStatus::Cancelled;
    }
}

/// Recover an Ethereum-style address from an ECDSA signature over a keccak256 hash.
fn recover_ecdsa_address(
    message_hash: &[u8; 32],
    r: &[u8; 32],
    s: &[u8; 32],
    v: u8,
) -> Option<Address> {
    use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};

    let recovery_id = RecoveryId::try_from(if v >= 27 { v - 27 } else { v }).ok()?;

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let signature = K256Signature::try_from(&sig_bytes[..]).ok()?;
    let verifying_key =
        VerifyingKey::recover_from_prehash(message_hash, &signature, recovery_id).ok()?;

    let public_key_point = verifying_key.to_encoded_point(false);
    let public_key_bytes = public_key_point.as_bytes();

    let pub_key = if public_key_bytes.len() == 65 && public_key_bytes[0] == 0x04 {
        &public_key_bytes[1..]
    } else {
        return None;
    };

    Some(derive_eth_address(pub_key))
}

/// Social Recovery Manager
pub struct SocialRecoveryManager {
    /// Active recovery requests (wallet_address -> RecoveryRequest)
    requests: HashMap<Address, RecoveryRequest>,
    /// Default time delay (7 days in seconds)
    default_time_delay: u64,
}

impl SocialRecoveryManager {
    /// Create a new social recovery manager
    pub fn new() -> Self {
        Self {
            requests: HashMap::new(),
            default_time_delay: 7 * 24 * 60 * 60, // 7 days
        }
    }

    /// Create a new recovery request
    pub fn initiate_recovery(
        &mut self,
        wallet_address: Address,
        new_owner: Address,
        guardians: Vec<Address>,
        recovery_threshold: u8,
        time_delay: Option<u64>,
        current_timestamp: u64,
    ) -> Result<RecoveryRequest, String> {
        // Check if recovery already exists
        if self.requests.contains_key(&wallet_address) {
            return Err("Recovery request already exists for this wallet".to_string());
        }

        // Validate threshold
        if recovery_threshold == 0 || recovery_threshold > guardians.len() as u8 {
            return Err("Invalid recovery threshold".to_string());
        }

        // Validate guardians
        if guardians.is_empty() {
            return Err("Guardians list cannot be empty".to_string());
        }

        // Use default time delay if not provided
        let delay = time_delay.unwrap_or(self.default_time_delay);

        // Create recovery request
        let request = RecoveryRequest::new(
            wallet_address,
            new_owner,
            guardians,
            recovery_threshold,
            delay,
            current_timestamp,
        );

        // Store request
        self.requests.insert(wallet_address, request.clone());

        Ok(request)
    }

    /// Add a guardian approval to a recovery request
    pub fn approve_recovery(
        &mut self,
        wallet_address: Address,
        guardian: Address,
        signature: &[u8],
        current_timestamp: u64,
    ) -> Result<(), String> {
        let request = self
            .requests
            .get_mut(&wallet_address)
            .ok_or("Recovery request not found")?;

        request.add_approval(guardian, signature, current_timestamp)?;

        // Update status
        request.update_status(current_timestamp);

        Ok(())
    }

    /// Get recovery request status
    pub fn get_recovery_status(&self, wallet_address: &Address) -> Option<&RecoveryRequest> {
        self.requests.get(wallet_address)
    }

    /// Complete recovery (transfer wallet ownership)
    pub fn complete_recovery(
        &mut self,
        wallet_address: Address,
        current_timestamp: u64,
    ) -> Result<Address, String> {
        let request = self
            .requests
            .get_mut(&wallet_address)
            .ok_or("Recovery request not found")?;

        // Update status
        request.update_status(current_timestamp);

        // Check if ready
        if request.status != RecoveryStatus::Ready {
            return Err("Recovery is not ready to complete".to_string());
        }

        // Get new owner
        let new_owner = request.new_owner;

        // Mark as completed
        request.status = RecoveryStatus::Completed;

        Ok(new_owner)
    }

    /// Cancel a recovery request
    pub fn cancel_recovery(&mut self, wallet_address: Address) -> Result<(), String> {
        let request = self
            .requests
            .get_mut(&wallet_address)
            .ok_or("Recovery request not found")?;

        request.cancel();
        Ok(())
    }

    /// Remove completed or cancelled recovery requests (cleanup)
    pub fn cleanup(&mut self) {
        self.requests.retain(|_, req| {
            matches!(
                req.status,
                RecoveryStatus::Pending | RecoveryStatus::Approved | RecoveryStatus::Ready
            )
        });
    }

    /// Get all active recovery requests
    pub fn get_all_requests(&self) -> Vec<&RecoveryRequest> {
        self.requests.values().collect()
    }

    /// Update all recovery request statuses based on current timestamp
    pub fn update_all_statuses(&mut self, current_timestamp: u64) {
        for request in self.requests.values_mut() {
            request.update_status(current_timestamp);
        }
    }
}

impl Default for SocialRecoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_keypair() -> (k256::ecdsa::SigningKey, Address) {
        use k256::ecdsa::SigningKey;
        use rand_core::OsRng;
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = k256::ecdsa::VerifyingKey::from(&signing_key);
        let public_key_point = verifying_key.to_encoded_point(false);
        let public_key_bytes = public_key_point.as_bytes();
        let pub_key = &public_key_bytes[1..];
        let address = derive_eth_address(pub_key);
        (signing_key, address)
    }

    fn sign_recovery_approval(
        signing_key: &k256::ecdsa::SigningKey,
        wallet_address: Address,
        new_owner: Address,
        timestamp: u64,
    ) -> Vec<u8> {
        use sha3::{Digest, Keccak256};
        let mut message = b"RECOVER:".to_vec();
        message.extend_from_slice(&wallet_address.0);
        message.extend_from_slice(&new_owner.0);
        message.extend_from_slice(&timestamp.to_le_bytes());

        let mut hasher = Keccak256::new();
        hasher.update(&message);
        let (signature, recovery_id) = signing_key
            .sign_digest_recoverable(hasher)
            .expect("Failed to sign");

        let sig_bytes = signature.to_bytes();
        let mut full_sig = Vec::with_capacity(65);
        full_sig.extend_from_slice(&sig_bytes[..32]);
        full_sig.extend_from_slice(&sig_bytes[32..64]);
        full_sig.push(recovery_id.to_byte() + 27);
        full_sig
    }

    #[test]
    fn test_recovery_request_creation() {
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let guardians = vec![
            Address::from([3; 20]),
            Address::from([4; 20]),
            Address::from([5; 20]),
        ];
        let timestamp = 1000;

        let request = RecoveryRequest::new(
            wallet,
            new_owner,
            guardians.clone(),
            2,
            604800, // 7 days
            timestamp,
        );

        assert_eq!(request.wallet_address, wallet);
        assert_eq!(request.new_owner, new_owner);
        assert_eq!(request.guardians, guardians);
        assert_eq!(request.recovery_threshold, 2);
        assert_eq!(request.status, RecoveryStatus::Pending);
        assert_eq!(request.approval_count(), 0);
    }

    #[test]
    fn test_guardian_approval() {
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let (sk1, guardian1) = create_test_keypair();
        let (sk2, guardian2) = create_test_keypair();
        let guardians = vec![guardian1, guardian2];
        let timestamp = 1000;

        let mut request = RecoveryRequest::new(wallet, new_owner, guardians, 2, 604800, timestamp);

        // Add first approval
        let sig1 = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 100);
        assert!(request.add_approval(guardian1, &sig1, timestamp + 100).is_ok());
        assert_eq!(request.approval_count(), 1);
        assert_eq!(request.status, RecoveryStatus::Pending);

        // Add second approval (threshold met)
        let sig2 = sign_recovery_approval(&sk2, wallet, new_owner, timestamp + 200);
        assert!(request.add_approval(guardian2, &sig2, timestamp + 200).is_ok());
        assert_eq!(request.approval_count(), 2);
        assert_eq!(request.status, RecoveryStatus::Approved);

        // Try to add duplicate approval
        let sig3 = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 300);
        assert!(request.add_approval(guardian1, &sig3, timestamp + 300).is_err());
    }

    #[test]
    fn test_recovery_time_delay() {
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let (sk1, guardian1) = create_test_keypair();
        let (sk2, guardian2) = create_test_keypair();
        let guardians = vec![guardian1, guardian2];
        let timestamp = 1000;
        let time_delay = 604800; // 7 days

        let mut request =
            RecoveryRequest::new(wallet, new_owner, guardians, 2, time_delay, timestamp);

        // Add approvals
        let sig1 = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 100);
        request.add_approval(guardian1, &sig1, timestamp + 100).unwrap();
        let sig2 = sign_recovery_approval(&sk2, wallet, new_owner, timestamp + 200);
        request.add_approval(guardian2, &sig2, timestamp + 200).unwrap();

        // Check immediately (not ready)
        assert!(!request.is_ready(timestamp + 1000));

        // Check after time delay (ready)
        assert!(request.is_ready(timestamp + time_delay + 1));
    }

    #[test]
    fn test_social_recovery_manager() {
        let mut manager = SocialRecoveryManager::new();
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let (sk1, guardian1) = create_test_keypair();
        let (sk2, guardian2) = create_test_keypair();
        let (_sk3, guardian3) = create_test_keypair();
        let guardians = vec![guardian1, guardian2, guardian3];
        let timestamp = 1000;

        // Initiate recovery
        let request = manager
            .initiate_recovery(wallet, new_owner, guardians.clone(), 2, None, timestamp)
            .unwrap();

        assert_eq!(request.status, RecoveryStatus::Pending);

        // Add approvals
        let sig1 = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 100);
        manager.approve_recovery(wallet, guardian1, &sig1, timestamp + 100).unwrap();
        let sig2 = sign_recovery_approval(&sk2, wallet, new_owner, timestamp + 200);
        manager.approve_recovery(wallet, guardian2, &sig2, timestamp + 200).unwrap();

        // Check status
        let status = manager.get_recovery_status(&wallet).unwrap();
        assert_eq!(status.status, RecoveryStatus::Approved);

        // Complete recovery after time delay
        let final_timestamp = timestamp + manager.default_time_delay + 1;
        let completed_owner = manager.complete_recovery(wallet, final_timestamp).unwrap();
        assert_eq!(completed_owner, new_owner);

        // Verify status
        let status = manager.get_recovery_status(&wallet).unwrap();
        assert_eq!(status.status, RecoveryStatus::Completed);
    }

    #[test]
    fn test_invalid_guardian_approval() {
        let mut manager = SocialRecoveryManager::new();
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let (sk1, guardian1) = create_test_keypair();
        let guardians = vec![guardian1];
        let timestamp = 1000;

        manager
            .initiate_recovery(wallet, new_owner, guardians, 2, None, timestamp)
            .unwrap();

        // Try to approve with valid signature but wrong guardian address
        let (_sk_other, other_guardian) = create_test_keypair();
        let sig = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 100);
        let result = manager.approve_recovery(wallet, other_guardian, &sig, timestamp + 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_signature() {
        let mut manager = SocialRecoveryManager::new();
        let wallet = Address::from([1; 20]);
        let new_owner = Address::from([2; 20]);
        let (sk1, guardian1) = create_test_keypair();
        let guardians = vec![guardian1];
        let timestamp = 1000;

        manager
            .initiate_recovery(wallet, new_owner, guardians, 2, None, timestamp)
            .unwrap();

        // Try to approve with invalid signature bytes
        let bad_sig = vec![0u8; 65];
        let result = manager.approve_recovery(wallet, guardian1, &bad_sig, timestamp + 100);
        assert!(result.is_err());

        // Try to approve with signature for different message
        let bad_sig2 = sign_recovery_approval(&sk1, wallet, new_owner, timestamp + 999);
        let result2 = manager.approve_recovery(wallet, guardian1, &bad_sig2, timestamp + 100);
        assert!(result2.is_err());
    }
}
