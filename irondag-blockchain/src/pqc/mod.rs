//! Post-Quantum Cryptography Module
//!
//! Provides PQ account types (ML-DSA-65/SLH-DSA) and ML-KEM-768 key exchange
//! for quantum-resistant blockchain operations.

pub mod accounts;
pub mod encryption;
pub mod kyber;
pub mod tooling;

pub use accounts::{PqAccount, PqAccountType, PqSignature};
pub use encryption::{EncryptedMessage, PqEncryption};
pub use kyber::{KemAlgorithm, KyberKeyExchange, SessionKey};
pub use tooling::{
    create_pq_transaction, derive_address_from_pq_account, format_pq_account, generate_pq_account,
    PqAccountExport,
};
