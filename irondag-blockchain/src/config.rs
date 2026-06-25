//! Configuration management
//!
//! Provides configuration loading and validation for the node.

use crate::types::Address;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::warn;

/// Default chain ID for IronDAG mainnet
const DEFAULT_CHAIN_ID: u64 = 11567;

/// Node configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Data directory for blockchain storage
    pub data_dir: PathBuf,

    /// Network port for P2P communication
    pub port: u16,

    /// JSON-RPC API port
    pub rpc_port: u16,

    /// Miner address (receives block rewards)
    pub miner_address: Address,

    /// Enable EVM
    pub evm_enabled: bool,

    /// Maximum peers to connect to
    pub max_peers: u32,

    /// Bootstrap peers (initial peers to connect to)
    pub bootstrap_peers: Vec<String>,

    /// Advertise address for P2P handshake (e.g. public IP:port). If unset, 0.0.0.0 is advertised as 127.0.0.1.
    pub advertise_addr: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    pub log_level: String,

    /// Enable metrics
    pub metrics_enabled: bool,

    /// Metrics port
    pub metrics_port: u16,

    /// RPC rate limit (requests per second)
    pub rpc_rate_limit: u32,

    /// Maximum transaction pool size
    pub max_tx_pool_size: usize,

    /// Chain ID 11567)
    pub chain_id: u64,

    /// Enable experimental gRPC v2 server (some methods unimplemented)
    pub enable_grpc_v2: bool,

    /// gRPC port (defaults to rpc_port + 1, typically 8546)
    pub grpc_port: u16,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            port: 8080,
            rpc_port: 8545,
            miner_address: Address([1u8; 20]), // Default miner address
            evm_enabled: true,
            max_peers: 50,
            bootstrap_peers: vec![],
            advertise_addr: None,
            log_level: "info".to_string(),
            metrics_enabled: false,
            metrics_port: 9090,
            rpc_rate_limit: 100,
            max_tx_pool_size: 10_000,
            chain_id: DEFAULT_CHAIN_ID,
            enable_grpc_v2: false,
            grpc_port: 8546, // Default to rpc_port + 1
        }
    }
}

impl NodeConfig {
    /// Load configuration from file
    pub fn from_file(path: &str) -> Result<Self, String> {
        use std::fs;

        let content =
            fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {}", e))?;

        let config: NodeConfig =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;

        Ok(config)
    }

    /// Save configuration to file
    pub fn save_to_file(&self, path: &str) -> Result<(), String> {
        use std::fs;

        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(path, content).map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), String> {
        // Port validation with warnings
        if self.port == 0 {
            return Err("P2P port cannot be 0".to_string());
        }

        if self.rpc_port == 0 {
            return Err("RPC port cannot be 0".to_string());
        }

        if self.port == self.rpc_port {
            warn!(
                "P2P port ({}) and RPC port ({}) are the same. This will cause conflicts.",
                self.port, self.rpc_port
            );
            return Err("P2P port and RPC port must be different".to_string());
        }

        // Warn about potentially problematic port configurations
        if self.port < 1024 {
            warn!("P2P port {} is in the well-known port range (0-1023). This may require elevated privileges.", self.port);
        }

        if self.rpc_port < 1024 {
            warn!("RPC port {} is in the well-known port range (0-1023). This may require elevated privileges.", self.rpc_port);
        }

        if self.grpc_port == 0 {
            return Err("gRPC port cannot be 0".to_string());
        }

        if self.grpc_port < 1024 {
            warn!("gRPC port {} is in the well-known port range (0-1023). This may require elevated privileges.", self.grpc_port);
        }

        // Check for port conflicts
        if self.grpc_port == self.port {
            warn!(
                "gRPC port ({}) and P2P port ({}) are the same. This will cause conflicts.",
                self.grpc_port, self.port
            );
            return Err("gRPC port and P2P port must be different".to_string());
        }

        if self.grpc_port == self.rpc_port {
            warn!(
                "gRPC port ({}) and RPC port ({}) are the same. This will cause conflicts.",
                self.grpc_port, self.rpc_port
            );
            return Err("gRPC port and RPC port must be different".to_string());
        }

        if self.max_peers == 0 {
            return Err("Max peers must be greater than 0".to_string());
        }

        if self.max_tx_pool_size == 0 {
            return Err("Max transaction pool size must be greater than 0".to_string());
        }

        if self.chain_id == 0 {
            return Err("Chain ID must be greater than 0".to_string());
        }

        // Check for default miner address (sentinel value)
        if self.miner_address.0 == [1u8; 20] {
            warn!("Using default miner address [1u8; 20]. This is a sentinel value - please configure a proper miner address.");
        }

        Ok(())
    }
}
