//! KZG Polynomial Commitment Scheme
//!
//! Implements KZG commitments for Verkle tree proof verification.
//! Uses BN254 curve for efficient pairing operations.
//!
//! This module is gated behind the `privacy` feature flag.

#[cfg(feature = "privacy")]
use ark_bn254::{Bn254, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
#[cfg(feature = "privacy")]
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
#[cfg(feature = "privacy")]
use ark_ff::{AdditiveGroup, Field};
#[cfg(feature = "privacy")]
use ark_poly::polynomial::univariate::DensePolynomial;
#[cfg(feature = "privacy")]
use ark_poly::{DenseUVPolynomial, Polynomial};
#[cfg(feature = "privacy")]
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
#[cfg(feature = "privacy")]
use ark_std::rand::rngs::StdRng;
#[cfg(feature = "privacy")]
use ark_std::rand::SeedableRng;
#[cfg(feature = "privacy")]
use ark_std::{UniformRand, Zero};

/// Maximum polynomial degree supported (256 for Verkle tree branching factor)
#[cfg(feature = "privacy")]
pub const MAX_DEGREE: usize = 256;

// ============================================================================
// KZG Types
// ============================================================================

/// KZG Structured Reference String (trusted setup)
///
/// Contains the powers of tau in G1 and G2 groups:
/// - G1 powers: [g, g*s, g*s^2, ..., g*s^d]
/// - G2 powers: [g2, g2*s]
#[cfg(feature = "privacy")]
#[derive(Debug, Clone)]
pub struct KzgSrs {
    /// Powers of tau in G1: [g, g*s, g*s^2, ..., g*s^d]
    pub g1_powers: Vec<G1Affine>,
    /// G2 generator
    pub g2_gen: G2Affine,
    /// tau * g2 (used for verification)
    pub g2_tau: G2Affine,
}

/// KZG commitment to a polynomial
#[cfg(feature = "privacy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, CanonicalSerialize, CanonicalDeserialize)]
pub struct KzgCommitment(pub G1Affine);

/// KZG opening proof
#[cfg(feature = "privacy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, CanonicalSerialize, CanonicalDeserialize)]
pub struct KzgProof(pub G1Affine);

// ============================================================================
// KZG Operations
// ============================================================================

#[cfg(feature = "privacy")]
impl KzgSrs {
    /// Generate KZG parameters with a deterministic seed
    ///
    /// WARNING: This is NOT production-safe. In production, use a proper
    /// trusted setup ceremony. This is for development/testing only.
    ///
    /// # Arguments
    /// * `max_degree` - Maximum polynomial degree to support
    /// * `seed` - Seed for deterministic parameter generation
    pub fn generate_deterministic(max_degree: usize, seed: u64) -> Self {
        let mut rng = StdRng::seed_from_u64(seed);

        // Generate random tau (the secret)
        let tau = Fr::rand(&mut rng);

        // Generate G1 powers: [g, g*tau, g*tau^2, ..., g*tau^d]
        let mut g1_powers = Vec::with_capacity(max_degree + 1);
        let mut tau_power = Fr::ONE;
        let g1_gen = G1Projective::generator();

        for _ in 0..=max_degree {
            let point = g1_gen * tau_power;
            g1_powers.push(point.into_affine());
            tau_power *= tau;
        }

        // Generate G2: [g2, g2*tau]
        let g2_gen = G2Projective::generator();
        let g2_tau = (g2_gen * tau).into_affine();

        Self {
            g1_powers,
            g2_gen: g2_gen.into_affine(),
            g2_tau,
        }
    }

    /// Commit to a polynomial represented by its coefficients
    ///
    /// # Arguments
    /// * `coeffs` - Polynomial coefficients [a_0, a_1, ..., a_n]
    ///              representing p(x) = a_0 + a_1*x + ... + a_n*x^n
    ///
    /// # Returns
    /// The KZG commitment C = sum(a_i * [tau^i]_G1)
    pub fn commit(&self, coeffs: &[Fr]) -> Option<KzgCommitment> {
        if coeffs.is_empty() || coeffs.len() > self.g1_powers.len() {
            return None;
        }

        // C = sum_{i=0}^{n} a_i * g1_powers[i]
        let mut commitment = G1Projective::zero();
        for (i, coeff) in coeffs.iter().enumerate() {
            if *coeff != Fr::ZERO {
                commitment += self.g1_powers[i] * coeff;
            }
        }

        Some(KzgCommitment(commitment.into_affine()))
    }

    /// Open a polynomial at a specific point
    ///
    /// Creates a proof that p(z) = v
    ///
    /// # Arguments
    /// * `coeffs` - Polynomial coefficients
    /// * `z` - The evaluation point
    ///
    /// # Returns
    /// The quotient polynomial commitment (the opening proof)
    pub fn open(&self, coeffs: &[Fr], z: Fr) -> Option<(Fr, KzgProof)> {
        if coeffs.is_empty() || coeffs.len() > self.g1_powers.len() {
            return None;
        }

        // Evaluate p(z)
        let poly = DensePolynomial::from_coefficients_vec(coeffs.to_vec());
        let v = poly.evaluate(&z);

        // Compute quotient polynomial q(x) = (p(x) - v) / (x - z)
        // We need to interpolate:
        // p(x) - v = (x - z) * q(x)
        // q(x) = (p(x) - v) / (x - z)

        let proof = self.compute_quotient_commitment(coeffs, v, z)?;

        Some((v, proof))
    }

    /// Compute the commitment to the quotient polynomial
    fn compute_quotient_commitment(&self, coeffs: &[Fr], v: Fr, z: Fr) -> Option<KzgProof> {
        // Compute quotient polynomial: q(x) = (p(x) - v) / (x - z)
        // Using the direct formula: q_i = sum_{j=i+1}^{n} a_j * z^{j-i-1}
        // This is derived from polynomial long division of (p(x) - v) by (x - z)

        let n = coeffs.len();
        let mut q_coeffs = vec![Fr::ZERO; n - 1]; // quotient has degree n-1

        // Compute each coefficient of the quotient polynomial
        for i in 0..(n - 1) {
            let mut sum = Fr::ZERO;
            let mut z_power = Fr::ONE;
            for j in (i + 1)..n {
                sum += coeffs[j] * z_power;
                z_power *= z;
            }
            q_coeffs[i] = sum;
        }

        // Commit to the quotient polynomial
        let mut proof = G1Projective::zero();
        for (i, coeff) in q_coeffs.iter().enumerate() {
            if *coeff != Fr::ZERO && i < self.g1_powers.len() {
                proof += self.g1_powers[i] * coeff;
            }
        }

        Some(KzgProof(proof.into_affine()))
    }
}

// ============================================================================
// Verification
// ============================================================================

#[cfg(feature = "privacy")]
impl KzgCommitment {
    /// Verify a KZG opening proof
    ///
    /// Checks that the polynomial committed to evaluates to `v` at point `z`
    ///
    /// # Arguments
    /// * `proof` - The KZG opening proof
    /// * `z` - The evaluation point
    /// * `v` - The claimed evaluation
    /// * `srs` - The structured reference string
    ///
    /// # Returns
    /// true if the proof is valid, false otherwise
    ///
    /// # Verification equation
    /// e(C - [v]G1, G2) == e(proof, [s - z]G2)
    ///
    /// This is equivalent to checking:
    /// e(C - v*G1, G2) = e(proof, tau*G2 - z*G2)
    pub fn verify(&self, proof: &KzgProof, z: Fr, v: Fr, srs: &KzgSrs) -> bool {
        // C - [v]G1
        let c_minus_v = {
            let c = self.0.into_group();
            let v_g1 = srs.g1_powers[0] * v;
            (c - v_g1).into_affine()
        };

        // [tau - z]G2
        let tau_minus_z_g2 = {
            let tau_g2 = srs.g2_tau.into_group();
            let z_g2 = srs.g2_gen * z;
            (tau_g2 - z_g2).into_affine()
        };

        // Pairing check: e(C - v*G1, G2) == e(proof, (tau - z)*G2)
        // Uses the bilinearity of the pairing

        // We need to check: e(c_minus_v, G2) * e(proof, tau_minus_z_g2)^{-1} = 1
        // Which is equivalent to: e(c_minus_v, G2) = e(proof, tau_minus_z_g2)

        // arkworks pairing check
        use ark_ec::pairing::Pairing;

        let lhs = Bn254::pairing(c_minus_v, srs.g2_gen);
        let rhs = Bn254::pairing(proof.0, tau_minus_z_g2);

        lhs == rhs
    }
}

// ============================================================================
// Serialization helpers
// ============================================================================

#[cfg(feature = "privacy")]
impl KzgCommitment {
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.serialize_compressed(&mut bytes).ok();
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut cursor = ark_std::io::Cursor::new(data);
        Self::deserialize_compressed(&mut cursor).ok()
    }
}

#[cfg(feature = "privacy")]
impl KzgProof {
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.serialize_compressed(&mut bytes).ok();
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let mut cursor = ark_std::io::Cursor::new(data);
        Self::deserialize_compressed(&mut cursor).ok()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(feature = "privacy")]
mod tests {
    use super::*;

    #[test]
    fn test_kzg_setup() {
        let srs = KzgSrs::generate_deterministic(256, 42);
        assert_eq!(srs.g1_powers.len(), 257); // 0 to 256 inclusive
        assert!(!srs.g2_gen.is_zero());
        assert!(!srs.g2_tau.is_zero());
    }

    #[test]
    fn test_kzg_commit_and_verify() {
        let srs = KzgSrs::generate_deterministic(256, 42);

        // Create a simple polynomial: p(x) = 3 + 5x + 7x^2
        let coeffs = vec![
            Fr::from(3u64), // a_0
            Fr::from(5u64), // a_1
            Fr::from(7u64), // a_2
        ];

        // Commit
        let commitment = srs.commit(&coeffs).expect("Commit should succeed");

        // Open at z = 2
        let z = Fr::from(2u64);
        let (v, proof) = srs.open(&coeffs, z).expect("Open should succeed");

        // Expected: p(2) = 3 + 5*2 + 7*4 = 3 + 10 + 28 = 41
        let expected = Fr::from(41u64);
        assert_eq!(v, expected, "Evaluation should be correct");

        // Verify
        let is_valid = commitment.verify(&proof, z, v, &srs);
        assert!(is_valid, "Valid proof should verify");
    }

    #[test]
    fn test_kzg_invalid_proof_rejected() {
        let srs = KzgSrs::generate_deterministic(256, 42);

        // Create a polynomial
        let coeffs = vec![Fr::from(3u64), Fr::from(5u64), Fr::from(7u64)];

        let commitment = srs.commit(&coeffs).expect("Commit should succeed");

        // Open at z = 2
        let z = Fr::from(2u64);
        let (_v, proof) = srs.open(&coeffs, z).expect("Open should succeed");

        // Try to verify with wrong value
        let wrong_v = Fr::from(999u64);
        let is_valid = commitment.verify(&proof, z, wrong_v, &srs);
        assert!(!is_valid, "Invalid proof should be rejected");

        // Try to verify with wrong point
        let wrong_z = Fr::from(5u64);
        let is_valid2 = commitment.verify(&proof, wrong_z, Fr::from(41u64), &srs);
        assert!(!is_valid2, "Proof at wrong point should be rejected");
    }

    #[test]
    fn test_kzg_zero_polynomial() {
        let srs = KzgSrs::generate_deterministic(256, 42);

        // Zero polynomial
        let coeffs = vec![Fr::ZERO];
        let commitment = srs.commit(&coeffs).expect("Commit should succeed");

        let z = Fr::from(5u64);
        let (v, proof) = srs.open(&coeffs, z).expect("Open should succeed");

        assert_eq!(v, Fr::ZERO, "Zero polynomial evaluates to zero");
        assert!(
            commitment.verify(&proof, z, v, &srs),
            "Zero proof should verify"
        );
    }

    #[test]
    fn test_kzg_constant_polynomial() {
        let srs = KzgSrs::generate_deterministic(256, 42);

        // Constant polynomial: p(x) = 42
        let coeffs = vec![Fr::from(42u64)];
        let commitment = srs.commit(&coeffs).expect("Commit should succeed");

        // Should evaluate to 42 at any point
        for z_val in [0u64, 1u64, 100u64, 255u64] {
            let z = Fr::from(z_val);
            let (v, proof) = srs.open(&coeffs, z).expect("Open should succeed");
            assert_eq!(v, Fr::from(42u64), "Constant should evaluate to itself");
            assert!(
                commitment.verify(&proof, z, v, &srs),
                "Constant proof should verify"
            );
        }
    }

    #[test]
    fn test_kzg_large_degree() {
        // Test with degree 256 (for Verkle tree)
        let srs = KzgSrs::generate_deterministic(256, 42);

        // Create polynomial with 257 coefficients (degree 256)
        let coeffs: Vec<Fr> = (0..=256).map(|i| Fr::from(i as u64)).collect();

        let commitment = srs.commit(&coeffs).expect("Commit should succeed");

        // Open at various points
        for z_val in [0u64, 10u64, 100u64] {
            let z = Fr::from(z_val);
            let (v, proof) = srs.open(&coeffs, z).expect("Open should succeed");

            // Verify evaluation is correct
            let poly = DensePolynomial::from_coefficients_vec(coeffs.clone());
            let expected = poly.evaluate(&z);
            assert_eq!(v, expected, "Evaluation should match polynomial evaluation");

            assert!(commitment.verify(&proof, z, v, &srs), "Proof should verify");
        }
    }

    #[test]
    fn test_commitment_serialization() {
        let srs = KzgSrs::generate_deterministic(256, 42);
        let coeffs = vec![Fr::from(3u64), Fr::from(5u64)];

        let commitment = srs.commit(&coeffs).expect("Commit should succeed");
        let bytes = commitment.to_bytes();

        let restored = KzgCommitment::from_bytes(&bytes).expect("Deserialization should work");
        assert_eq!(commitment, restored, "Roundtrip should preserve commitment");
    }

    #[test]
    fn test_proof_serialization() {
        let srs = KzgSrs::generate_deterministic(256, 42);
        let coeffs = vec![Fr::from(3u64), Fr::from(5u64)];

        let (_, proof) = srs
            .open(&coeffs, Fr::from(2u64))
            .expect("Open should succeed");
        let bytes = proof.to_bytes();

        let restored = KzgProof::from_bytes(&bytes).expect("Deserialization should work");
        assert_eq!(proof, restored, "Roundtrip should preserve proof");
    }
}

// ============================================================================
// Non-privacy fallback types
// ============================================================================

/// Placeholder KZG commitment type for non-privacy builds
#[cfg(not(feature = "privacy"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KzgCommitment(pub [u8; 32]);

/// Placeholder KZG proof type for non-privacy builds
#[cfg(not(feature = "privacy"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KzgProof(pub [u8; 32]);

#[cfg(not(feature = "privacy"))]
impl KzgCommitment {
    /// Create a zero commitment
    pub fn zero() -> Self {
        Self([0u8; 32])
    }

    /// Convert to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

#[cfg(not(feature = "privacy"))]
impl KzgProof {
    /// Create a zero proof
    pub fn zero() -> Self {
        Self([0u8; 32])
    }
}
