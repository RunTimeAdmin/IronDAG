//! Security module for rule-based fraud detection and risk scoring
//!
//! Provides protocol-level security services including:
//! - Fraud and anomaly detection
//! - Risk scoring for addresses, transactions, and contracts
//! - Security labels and threat classification
//! - Forensic analysis and fund tracing
//! - Security hardening (DoS protection, rate limiting, IP filtering)

pub mod forensics;
pub mod fraud_detection;
pub mod hardening;
pub mod policies;
pub mod risk_scoring;

pub use forensics::{
    AddressSummary, Anomaly, AnomalyDetection, AnomalyType, ForensicAnalyzer, FundFlow,
};
pub use fraud_detection::{FraudAnalysis, FraudDetector, PatternRule};
pub use hardening::{IpSecurityStats, SecurityConfig, SecurityError, SecurityHardening};
pub use policies::{
    PolicyAction, PolicyEvaluation, PolicyType, SecurityPolicy, SecurityPolicyManager,
};
pub use risk_scoring::{AddressHistory, RiskScore, RiskScorer};
