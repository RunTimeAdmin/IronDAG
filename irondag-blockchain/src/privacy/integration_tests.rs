#[cfg(test)]
mod integration_tests {
    use crate::privacy::circuit::PrivacyCircuit;
    use crate::privacy::{
        generate_keys, Nullifier, PedersenCommitment, PrivacyProver, PrivacyVerifier,
        PrivateTransferCircuit,
    };
    use ark_bn254::Fr;
    use ark_ec::AffineRepr;
    use ark_ff::Zero;
    use ark_std::rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn test_key_generation() {
        let mut rng = StdRng::seed_from_u64(42);
        let (pk, vk) = generate_keys(&mut rng).unwrap();

        // Keys should be generated successfully
        assert!(!pk.vk.alpha_g1.is_zero());
        assert!(!vk.beta_g2.is_zero());
    }

    #[test]
    fn test_proof_generation_and_verification() {
        let mut rng = StdRng::seed_from_u64(42);

        // Generate keys
        let (pk, vk) = generate_keys(&mut rng).unwrap();

        // Create prover and verifier
        let prover = PrivacyProver::new(pk);
        let verifier = PrivacyVerifier::new(vk);

        // Create circuit with witness values
        let old_balance = 1000u128;
        let amount = 100u128;
        let new_balance = 900u128;
        let nullifier = Fr::from(42u64);
        let commitment = Fr::from(123u64);

        // Generate proof
        let proof = prover
            .prove_private_transfer(
                old_balance,
                amount,
                new_balance,
                nullifier,
                commitment,
                &mut rng,
            )
            .unwrap();

        // Verify proof
        let circuit = PrivateTransferCircuit {
            old_balance: Some(old_balance),
            amount: Some(amount),
            new_balance: Some(new_balance),
            nullifier,
            commitment: Some(commitment),
        };

        let public_inputs = circuit.public_inputs();
        let verified = verifier.verify_with_inputs(&proof, &public_inputs);

        assert!(verified, "Proof verification should succeed");
    }

    #[test]
    fn test_commitment_and_nullifier() {
        let amount = 1000u128;
        let blinding = [42u8; 32];
        let receiver = [1u8; 20];

        // Create commitment
        let commitment = PedersenCommitment::commit(amount, &blinding);
        assert_ne!(commitment.point, ark_bn254::G1Projective::zero());

        // Generate nullifier
        let nullifier = Nullifier::generate(&receiver, &blinding);
        assert_ne!(nullifier.hash, [0u8; 32]);

        // Serialize/deserialize commitment
        let bytes = commitment.to_bytes();
        let deserialized = crate::privacy::Commitment::from_bytes(&bytes);
        assert!(deserialized.is_some());
    }
}
