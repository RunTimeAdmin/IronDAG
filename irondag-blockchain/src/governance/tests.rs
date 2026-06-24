//! Tests for governance and node longevity system

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::*;
    use crate::types::{Hash, StreamType};

    #[test]
    fn test_hardware_fingerprint_generation() {
        let private_key = [1u8; 32];
        let fingerprint = node_identity::HardwareFingerprint::generate(&private_key);

        // Verify fingerprint is not empty
        assert_ne!(fingerprint.fingerprint, Hash([0u8; 32]));

        // Verify signature
        let public_key = private_key; // Placeholder scheme uses same bytes
        assert!(fingerprint.verify(&public_key));
    }

    #[test]
    fn test_node_identity_creation() {
        let private_key = [1u8; 32];
        let public_key = private_key;

        let hardware_fingerprint = node_identity::HardwareFingerprint::generate(&private_key);
        let node_identity = node_identity::NodeIdentity {
            public_key,
            ip_address: None,
            hardware_fingerprint,
            zk_uniqueness_proof: None,
            created_at: 1000,
        };

        assert_eq!(node_identity.public_key, public_key);
        assert_eq!(node_identity.created_at, 1000);
    }

    #[test]
    fn test_node_registry() {
        let mut registry = registry::NodeRegistry::new();

        let private_key = [1u8; 32];
        let public_key = private_key;
        let hardware_fingerprint = node_identity::HardwareFingerprint::generate(&private_key);

        let node_identity = node_identity::NodeIdentity {
            public_key,
            ip_address: None,
            hardware_fingerprint,
            zk_uniqueness_proof: None,
            created_at: 1000,
        };

        // Register node
        assert!(registry.register_node(node_identity.clone()).is_ok());

        // Try to register same node again (should fail)
        assert!(registry.register_node(node_identity).is_err());

        // Check node count
        assert_eq!(registry.total_nodes(), 1);
    }

    #[test]
    fn test_node_longevity() {
        let private_key = [1u8; 32];
        let public_key = private_key;
        let hardware_fingerprint = node_identity::HardwareFingerprint::generate(&private_key);

        let node_identity = node_identity::NodeIdentity {
            public_key,
            ip_address: None,
            hardware_fingerprint,
            zk_uniqueness_proof: None,
            created_at: 1000,
        };

        let mut longevity = longevity::NodeLongevity::new(node_identity);

        // Initially, weight should be 0 (not enough active days)
        assert_eq!(longevity.calculate_weight(100), 0.0);

        // Simulate required thresholds for weight calculation
        longevity.active_days = 30;
        longevity.blocks_mined = 1;
        longevity.uptime_index = 0.9;

        // Now should have some weight (but still capped)
        let weight = longevity.calculate_weight(100);
        assert!(weight > 0.0);
        assert!(weight <= 0.001); // Capped at 0.1%
    }

    #[test]
    fn test_longevity_reset() {
        let private_key = [1u8; 32];
        let public_key = private_key;
        let hardware_fingerprint = node_identity::HardwareFingerprint::generate(&private_key);

        let node_identity = node_identity::NodeIdentity {
            public_key,
            ip_address: None,
            hardware_fingerprint,
            zk_uniqueness_proof: None,
            created_at: 1000,
        };

        let mut longevity = longevity::NodeLongevity::new(node_identity);

        // Record some activity
        let participation = longevity::ParticipationType::BlockMined {
            stream: StreamType::StreamA,
            block_hash: Hash([1u8; 32]),
        };

        longevity.record_activity_snapshot(participation);
        assert!(longevity.active_days > 0);

        // Simulate 31 days offline (force reset path)
        longevity.activity_snapshots.clear();
        longevity.consecutive_offline_days = 30;
        longevity.record_no_activity();

        // Should be reset
        assert_eq!(longevity.active_days, 0);
        assert_eq!(longevity.uptime_index, 0.0);
    }
}
