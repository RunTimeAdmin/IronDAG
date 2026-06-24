//! Node implementation

pub mod pool;

use crate::blockchain::Blockchain;
use crate::mining::MiningManager;
use crate::network::NetworkManager;
use crate::pow;
use crate::rpc::RpcServer;
use crate::sharding::{AssignmentStrategy, ShardConfig, ShardManager};
use crate::storage::{Database, StorageConfig};
use crate::types::{Address, GenesisConfig, DEFAULT_CHAIN_ID};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Node configuration
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub port: u16,
    pub rpc_port: u16,
    pub http_api_port: u16,
    pub miner_address: Address,
    pub data_dir: String,
    /// Chain ID for network identification and EIP-155
    pub chain_id: u64,
    /// Enable sharding
    pub enable_sharding: bool,
    /// Number of shards (if sharding enabled)
    pub shard_count: usize,
    /// Local shard ID for this node (when sharding enabled, defaults to 0)
    pub local_shard_id: usize,
    /// Enable Verkle tree (stateless mode)
    pub enable_verkle: bool,
    /// Enable privacy layer (zk-SNARK)
    pub enable_privacy: bool,
    /// Enable mining (set to false for RPC-only mode)
    pub enable_mining: bool,
    /// Single-stream mining mode (only Stream A)
    /// Reduces CPU usage by ~66% - useful for resource-constrained VPS
    pub single_stream: bool,
    /// Bootstrap peers (initial peers to connect to on start)
    pub bootstrap_peers: Vec<String>,
    /// Advertise address for P2P handshake (e.g. public IP:port). If unset, 0.0.0.0 is advertised as 127.0.0.1.
    pub advertise_addr: Option<String>,
    /// Max P2P peers (0 = no limit). Enforced when connecting.
    pub max_peers: u32,
    /// Enable Stream C mining (high frequency, fee-based). Default false for lower CPU on 4-core nodes.
    pub enable_stream_c: bool,
    /// Allow unsigned eth_sendTransaction (dev only). When false, eth_sendTransaction requires a signed tx; use eth_sendRawTransaction in production.
    pub allow_unsigned_eth_send: bool,
    /// When true and advertise_addr is unset, try STUN (stun.l.google.com) to discover public IP for handshake.
    pub try_stun_discovery: bool,
    /// Path to proving key from trusted setup (if set with privacy_verifying_key_path, load shared keys instead of generating per-node).
    pub privacy_proving_key_path: Option<String>,
    /// Path to verifying key from trusted setup.
    pub privacy_verifying_key_path: Option<String>,
    /// Genesis allocations for pre-funded addresses (loaded from --genesis-file)
    pub genesis_allocations: Vec<crate::types::GenesisAllocation>,
    /// Path to genesis allocations JSON file
    pub genesis_file: Option<String>,
    /// Path to TLS certificate file (PEM format) for HTTPS RPC
    pub tls_cert_path: Option<String>,
    /// Path to TLS private key file (PEM format) for HTTPS RPC
    pub tls_key_path: Option<String>,
    /// CORS allowed origins for RPC server (e.g., ["https://irondag.io", "https://explorer.irondag.io"])
    /// Use ["*"] to allow all origins (for local development only)
    pub cors_allowed_origins: Vec<String>,
    /// Disable RPC authentication (dev only, NOT for production)
    pub rpc_no_auth: bool,
    /// Static RPC API key. If set, used instead of the auto-generated key and survives restarts.
    /// Set via --rpc-api-key <KEY> or the IRONDAG_API_KEY environment variable.
    pub rpc_api_key: Option<String>,
    /// Disable TLS warning for non-localhost RPC (for operators running behind reverse proxy)
    pub disable_tls_warning: bool,
    /// Mining backend for Stream B: cpu, gpu, auto (default: cpu; GPU not yet implemented)
    pub mining_backend: String,
    /// Pruning interval in seconds (default: 60)
    pub prune_interval_secs: u64,
    /// Keep red blocks in DAG (default: false, prune them)
    pub keep_red_blocks: bool,
    /// Sled cache capacity in megabytes (default: 256)
    pub sled_cache_mb: u64,
    /// Sled flush interval in milliseconds (default: 1000)
    pub sled_flush_ms: u64,
    /// Use sled high-throughput mode (default: true)
    pub sled_high_throughput: bool,
    /// ZK proving/verifying keys directory (contains proving_key.bin and verifying_key.bin)
    /// If set, ZK proving is enabled for state transitions. If None, ZK proving is disabled.
    #[cfg(feature = "privacy")]
    pub zk_keys_dir: Option<String>,
    /// GhostDAG K parameter - maximum number of parents per block and tips selection
    /// Controls DAG security and throughput. Default: 4 (as per Kaspa/BlockDAG spec)
    /// Higher values = more parallelism but slower convergence
    pub ghostdag_k: usize,
    /// RPC rate limit: maximum requests per minute per IP (default: 1000)
    pub rpc_rate_limit: u32,
    /// RPC burst size: burst allowance for rate limiting (default: 50)
    pub rpc_burst_size: u32,
    /// Pruning batch size: number of blocks to process per pruning cycle (default: 200)
    pub prune_batch_size: usize,
    /// Enable experimental gRPC v2 server (some methods unimplemented)
    pub enable_grpc_v2: bool,
    /// QUIC connection idle timeout in seconds (default: 30)
    pub quic_idle_timeout_secs: u64,
    /// Explicitly configured public IP for P2P handshake (skips UDP discovery)
    pub public_ip: Option<std::net::IpAddr>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            rpc_port: 8545,
            http_api_port: 8081,
            miner_address: Address([1u8; 20]), // Default miner address
            data_dir: "data".to_string(),
            chain_id: DEFAULT_CHAIN_ID, // Changed from 1337 to force MetaMask reset
            enable_sharding: false,     // Disabled by default
            shard_count: 10,            // 10 shards if enabled
            local_shard_id: 0,          // Default to shard 0
            enable_verkle: false,       // Disabled by default
            enable_privacy: false,      // Disabled for v1.0 (ZK proving deferred to v2.0)
            enable_mining: true,        // Enabled by default
            single_stream: false,       // BraidCore by default; use --single-stream for reduced CPU
            bootstrap_peers: vec![],
            advertise_addr: None,
            max_peers: 50,
            enable_stream_c: false,
            allow_unsigned_eth_send: false,
            try_stun_discovery: false,
            privacy_proving_key_path: None,
            privacy_verifying_key_path: None,
            genesis_allocations: Vec::new(),
            genesis_file: None,
            tls_cert_path: None,
            tls_key_path: None,
            cors_allowed_origins: vec![
                "http://localhost:3000".to_string(),
                "http://localhost:8080".to_string(),
                "http://127.0.0.1:3000".to_string(),
                "http://127.0.0.1:8080".to_string(),
            ],
            rpc_no_auth: false, // Auth enabled by default (secure-by-default)
            rpc_api_key: std::env::var("IRONDAG_API_KEY").ok(),
            disable_tls_warning: false, // TLS warning enabled by default
            mining_backend: "cpu".to_string(), // Default to CPU mining
            prune_interval_secs: 60,    // Default: prune every 60 seconds
            keep_red_blocks: false,     // Default: prune red blocks
            sled_cache_mb: 256,         // Default: 256MB cache
            sled_flush_ms: 200,         // 200ms for faster durability on crash
            sled_high_throughput: true, // Default: high-throughput mode
            #[cfg(feature = "privacy")]
            zk_keys_dir: None, // Default: ZK proving disabled
            ghostdag_k: 4,              // Default: K=4 (Kaspa standard)
            rpc_rate_limit: 1000,       // Default: 1000 requests/minute per IP
            rpc_burst_size: 50,         // Default: 50 burst allowance
            prune_batch_size: 200,      // Default: 200 blocks per pruning batch
            enable_grpc_v2: false,      // Default: disabled (experimental)
            quic_idle_timeout_secs: 30, // Default: 30 seconds
            public_ip: None,            // Default: use UDP discovery
        }
    }
}

/// Node
pub struct Node {
    config: NodeConfig,
    blockchain: Arc<RwLock<Blockchain>>,
    mining_manager: Arc<MiningManager>,
    network_manager: Arc<NetworkManager>,
    rpc_server: Arc<RpcServer>,
    #[allow(dead_code)]
    shard_manager: Option<Arc<ShardManager>>,
    metrics: Option<crate::metrics::MetricsHandle>,
    shutdown_signal: Arc<tokio::sync::Notify>,
    ready: Arc<tokio::sync::RwLock<bool>>, // Ready flag for startup sequencing
    /// Active catch-up loop peers (deduplication to prevent multiple loops per peer)
    active_catchup_peers: Arc<tokio::sync::RwLock<HashSet<SocketAddr>>>,
    /// ZK proving key for state transition circuit (if loaded)
    #[cfg(feature = "privacy")]
    zk_proving_key: Option<Arc<ark_groth16::ProvingKey<ark_bn254::Bn254>>>,
    /// ZK verifying key for state transition circuit (if loaded)
    #[cfg(feature = "privacy")]
    zk_verifying_key: Option<Arc<ark_groth16::VerifyingKey<ark_bn254::Bn254>>>,
}

impl Node {
    pub async fn new(config: NodeConfig) -> Self {
        // Build storage config from node config
        let storage_config = StorageConfig {
            cache_capacity: config.sled_cache_mb * 1024 * 1024, // Convert MB to bytes
            flush_every_ms: Some(config.sled_flush_ms),
            high_throughput: config.sled_high_throughput,
            ..Default::default()
        };

        // Create or open database
        let database = match Database::open_with_config(&config.data_dir, storage_config) {
            Ok(db) => {
                info!(
                    "Opened database at: {} (cache: {}MB, flush: {}ms)",
                    config.data_dir, config.sled_cache_mb, config.sled_flush_ms
                );
                Some(Arc::new(db))
            }
            Err(e) => {
                warn!("Failed to open database: {}. Using in-memory mode.", e);
                None
            }
        };

        // Create metrics collector
        let shard_count_for_metrics = if config.enable_sharding {
            config.shard_count
        } else {
            0
        };
        let metrics = match crate::metrics::create_metrics(shard_count_for_metrics) {
            Ok(m) => {
                info!("Metrics collection enabled");
                Some(m)
            }
            Err(e) => {
                warn!("Failed to create metrics: {}. Metrics disabled.", e);
                None
            }
        };

        // Create shard manager if enabled (needed before blockchain creation)
        let shard_manager: Option<Arc<ShardManager>> =
            if config.enable_sharding && config.shard_count > 0 {
                let shard_config = ShardConfig {
                    shard_count: config.shard_count,
                    enable_cross_shard: true,
                    assignment_strategy: AssignmentStrategy::ConsistentHashing,
                    cross_shard_wal_path: None,
                    ..Default::default()
                };
                info!("Sharding enabled with {} shards", config.shard_count);
                let sm = Arc::new(ShardManager::new(shard_config));
                sm.clone().start_cross_shard_retry_worker();
                // Phase 6: Start receipt processing tasks for cross-shard message handling
                let started = sm.clone().start_receipt_processing().await;
                if started > 0 {
                    info!("Started {} cross-shard receipt processing tasks", started);
                }
                Some(sm)
            } else {
                None
            };

        // Create blockchain with or without storage and Verkle
        let ghostdag_k = config.ghostdag_k;
        let mut blockchain = if config.enable_verkle {
            // Verkle mode (stateless)
            if let Some(db) = database {
                let db_for_blockchain = db.clone();
                let db_for_evm = db.clone();
                match Blockchain::with_storage_verkle_and_k(db_for_blockchain, ghostdag_k) {
                    Ok(mut bc) => {
                        bc.evm_enabled = true;
                        bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::with_database(
                            db_for_evm,
                        ));
                        info!("Loaded blockchain state from storage");
                        info!("Verkle tree enabled (stateless mode)");
                        info!("EVM enabled for smart contract support with persistent storage");
                        info!("GhostDAG K={} for block selection", ghostdag_k);
                        bc
                    }
                    Err(e) => {
                        warn!(
                            "Failed to load from storage: {}. Using in-memory Verkle mode.",
                            e
                        );
                        let mut bc = Blockchain::with_verkle();
                        bc.evm_enabled = true;
                        bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::new());
                        info!("Verkle tree enabled (stateless mode)");
                        info!("EVM enabled for smart contract support (in-memory only)");
                        bc
                    }
                }
            } else {
                let mut bc = Blockchain::with_verkle();
                bc.evm_enabled = true;
                bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::new());
                info!("Verkle tree enabled (stateless mode)");
                info!("EVM enabled for smart contract support (in-memory only)");
                bc
            }
        } else {
            // Traditional mode (with storage)
            if let Some(db) = database {
                let db_for_blockchain = db.clone();
                let db_for_evm = db.clone();
                match Blockchain::with_storage_and_k(db_for_blockchain, ghostdag_k) {
                    Ok(mut bc) => {
                        bc.evm_enabled = true;
                        bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::with_database(
                            db_for_evm,
                        ));
                        info!("Loaded blockchain state from storage");
                        info!("EVM enabled for smart contract support with persistent storage");
                        info!("GhostDAG K={} for block selection", ghostdag_k);
                        bc
                    }
                    Err(e) => {
                        warn!("Failed to load from storage: {}. Using in-memory mode.", e);
                        let mut bc = Blockchain::new();
                        bc.evm_enabled = true;
                        bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::new());
                        info!("EVM enabled for smart contract support (in-memory only)");
                        bc
                    }
                }
            } else {
                let mut bc = Blockchain::new();
                bc.evm_enabled = true;
                bc.evm_executor = Some(crate::evm::EvmTransactionExecutor::new());
                info!("EVM enabled for smart contract support (in-memory only)");
                bc
            }
        };

        // Set shard manager in blockchain if sharding is enabled
        // Note: We don't actually need to set it in blockchain for now since
        // cross-shard transactions are handled at the shard manager level

        // Set chain_id from config
        blockchain.set_chain_id(config.chain_id);

        // Wire Parallel EVM executor — enables dependency-graph batching and the
        // irondag_setParallelEvm / irondag_getParallelEvmStatus RPC endpoints.
        let parallel_evm_executor = Arc::new(tokio::sync::RwLock::new(
            crate::evm::parallel::ParallelEvmExecutor::new(),
        ));
        blockchain.parallel_evm_executor = Some(parallel_evm_executor.clone());

        let blockchain_arc = Arc::new(RwLock::new(blockchain));

        // Create mining manager (with sharding if enabled)
        let mut mining_manager = if let Some(ref shard_mgr) = shard_manager {
            MiningManager::with_sharding(
                blockchain_arc.clone(),
                config.miner_address,
                shard_mgr.clone(),
                config.local_shard_id,
            )
        } else {
            MiningManager::new(blockchain_arc.clone(), config.miner_address)
        };

        // Configure mining backend for Stream B
        let backend_config = pow::MiningBackendConfig::from_str(&config.mining_backend);
        let backend = pow::create_mining_backend(backend_config);
        mining_manager.set_mining_backend(backend);

        // Configure pruning
        mining_manager.set_pruning_config(
            config.prune_interval_secs,
            config.keep_red_blocks,
            config.prune_batch_size,
        );

        // Share GhostDAG between Blockchain and MiningManager for parent selection
        {
            let bc = blockchain_arc.read().await;
            if let Some(ghostdag) = bc.ghostdag() {
                mining_manager.set_ghostdag(ghostdag);
                info!("GhostDAG consensus integrated with mining layer");
            }
        }

        // Initialize rustls crypto provider for QUIC (must be done before any TLS operations)
        crate::quic_transport::init_crypto_provider();

        // Create network manager
        // Bind to 0.0.0.0 to allow external P2P connections
        let listen_addr = format!("0.0.0.0:{}", config.port)
            .parse::<SocketAddr>()
            .unwrap_or_else(|_| "0.0.0.0:8080".parse().unwrap());

        let mut network_manager = NetworkManager::new(blockchain_arc.clone(), listen_addr);
        if let Some(ref addr) = config.advertise_addr {
            network_manager.set_advertise_addr(addr.clone());
        } else if config.try_stun_discovery {
            if let Some(public_addr) =
                crate::network::stun::discover_public_addr(listen_addr.port()).await
            {
                network_manager.set_advertise_addr(public_addr.clone());
                info!("STUN: advertising public address {}", public_addr);
            }
        }
        if config.max_peers > 0 {
            network_manager.set_max_peers(config.max_peers);
        }
        // Set public IP if configured (skips UDP discovery)
        if let Some(public_ip) = config.public_ip {
            network_manager.set_public_ip(public_ip);
        }
        // Set QUIC idle timeout from config
        network_manager.set_quic_idle_timeout(config.quic_idle_timeout_secs);
        // Noise Protocol encryption has been replaced by QUIC TLS 1.3
        // The noise_encryption config option is deprecated and ignored
        // Channel for peer exchange: when we receive Peers message we send addrs here; task calls connect_peer
        let (peer_connect_tx, mut peer_connect_rx) = mpsc::channel(64);
        network_manager.set_peer_connect_tx(peer_connect_tx);
        // Set shard manager in network manager if sharding is enabled
        if let Some(ref shard_mgr) = shard_manager {
            network_manager.set_shard_manager(shard_mgr.clone());
        }

        let network_manager = Arc::new(network_manager);
        let nm_for_peer = network_manager.clone();
        tokio::spawn(async move {
            while let Some(addr) = peer_connect_rx.recv().await {
                if let Err(e) = nm_for_peer.connect_peer(addr).await {
                    warn!("  Peer exchange connect to {}: {}", addr, e);
                }
            }
        });

        // Start periodic peer exchange task (every 5 minutes)
        network_manager.start_periodic_peer_exchange();
        info!("Started periodic peer exchange task (5 min interval)");

        // Start shard sync task for catch-up protocol (if sharding enabled)
        if let Some(ref shard_mgr) = shard_manager {
            shard_mgr
                .clone()
                .start_shard_sync_task(network_manager.clone());
            info!("Started shard sync catch-up task");
        }

        // CRITICAL: Set network manager in mining manager for block broadcasting
        // This enables block propagation when blocks are mined
        mining_manager.set_network(network_manager.clone()).await;

        // Wrap mining manager in Arc
        let mining_manager = Arc::new(mining_manager);

        // Create security scorer for fraud detection
        let security_scorer =
            Arc::new(tokio::sync::RwLock::new(crate::security::RiskScorer::new()));
        info!("Security scoring enabled (fraud detection)");

        // Create forensic analyzer (will be indexed as blocks are added)
        let forensic_analyzer = Arc::new(tokio::sync::RwLock::new(
            crate::security::ForensicAnalyzer::new(),
        ));
        info!("Forensic analyzer initialized");

        // Create light client for stateless mode
        let light_client = Arc::new(tokio::sync::RwLock::new(
            crate::light_client::LightClient::new(),
        ));
        if config.enable_verkle {
            info!("Light client initialized (will sync on first block)");
        }

        // Create RPC server (with sharding if enabled)
        // Use without_auth variants when rpc_no_auth is enabled (dev only)
        let mut rpc_server: RpcServer = if config.rpc_no_auth {
            warn!("RPC authentication disabled. Do not use in production.");
            if let Some(ref shard_mgr) = shard_manager {
                RpcServer::without_auth_with_chain_id_and_sharding(
                    blockchain_arc.clone(),
                    config.chain_id,
                    shard_mgr.clone(),
                )
            } else {
                RpcServer::without_auth_with_chain_id(blockchain_arc.clone(), config.chain_id)
            }
        } else {
            // Default: auth enabled (auto-generates API key)
            if let Some(ref shard_mgr) = shard_manager {
                RpcServer::with_chain_id_and_sharding(
                    blockchain_arc.clone(),
                    config.chain_id,
                    shard_mgr.clone(),
                )
            } else {
                RpcServer::new_with_chain_id(blockchain_arc.clone(), config.chain_id)
            }
        };

        // Apply static API key if configured (overrides auto-generated key, survives restarts)
        if !config.rpc_no_auth {
            if let Some(ref key) = config.rpc_api_key {
                rpc_server.set_api_key(key.clone());
                println!("RPC API Key loaded from configuration (static key active).");
            }
        }

        rpc_server.set_allow_unsigned_eth_send(config.allow_unsigned_eth_send);
        rpc_server.set_miner_address(config.miner_address);

        // Set security scorer in RPC server
        rpc_server.set_security_scorer(security_scorer.clone());

        // Set mining manager in RPC server for fairness metrics
        rpc_server.set_mining_manager(mining_manager.clone());

        // Set forensic analyzer in RPC server
        rpc_server.set_forensic_analyzer(forensic_analyzer.clone());

        // Set light client in RPC server
        rpc_server.set_light_client(light_client.clone());

        // Set network manager in RPC server for peer count
        rpc_server.set_network_manager(network_manager.clone());

        // Create and set policy manager
        let policy_manager = Arc::new(tokio::sync::RwLock::new(
            crate::security::SecurityPolicyManager::new(),
        ));
        rpc_server.set_policy_manager(policy_manager.clone());
        info!("Security policy manager initialized");

        // Configure per-IP rate limiter with configurable limits
        let rate_limit_tokens_per_second = config.rpc_rate_limit as f64 / 60.0;
        let per_ip_limiter = Arc::new(crate::rpc::rate_limit::PerIpRateLimiter::new(
            config.rpc_burst_size,
            rate_limit_tokens_per_second,
        ));
        rpc_server.set_per_ip_rate_limiter(per_ip_limiter);
        info!(
            "RPC rate limiter configured: {} requests/min per IP, burst size {}",
            config.rpc_rate_limit, config.rpc_burst_size
        );

        rpc_server.with_parallel_evm_executor(parallel_evm_executor.clone());
        info!("Parallel EVM executor wired (dependency-graph batching enabled)");

        let rpc_server = Arc::new(rpc_server);

        // Create ready flag (starts as false until initialization completes)
        let ready = Arc::new(tokio::sync::RwLock::new(false));

        // Load ZK proving/verifying keys if configured
        #[cfg(feature = "privacy")]
        let (zk_proving_key, zk_verifying_key) = {
            use crate::zk::{load_proving_key, load_verifying_key};
            use std::path::PathBuf;

            if let Some(ref zk_keys_dir) = config.zk_keys_dir {
                let zk_dir = PathBuf::from(zk_keys_dir);
                let pk_path = zk_dir.join("proving_key.bin");
                let vk_path = zk_dir.join("verifying_key.bin");

                match (
                    load_proving_key(pk_path.to_str().unwrap()),
                    load_verifying_key(vk_path.to_str().unwrap()),
                ) {
                    (Ok(pk), Ok(vk)) => {
                        info!("ZK proving keys loaded from {}", zk_keys_dir);
                        (Some(Arc::new(pk)), Some(Arc::new(vk)))
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        warn!(
                            "Failed to load ZK keys from {}: {}. ZK proving disabled.",
                            zk_keys_dir, e
                        );
                        (None, None)
                    }
                }
            } else {
                info!("ZK proving disabled (no keys configured)");
                (None, None)
            }
        };

        // Wire ZK verifying key to blockchain for proof verification
        #[cfg(feature = "privacy")]
        {
            if let Some(ref vk) = zk_verifying_key {
                let mut bc = blockchain_arc.write().await;
                bc.set_zk_verifying_key(vk.clone());
                tracing::debug!("ZK verifying key wired to blockchain");
            }
        }

        Self {
            config,
            blockchain: blockchain_arc,
            mining_manager,
            network_manager,
            rpc_server,
            shard_manager,
            metrics,
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            ready,
            active_catchup_peers: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            #[cfg(feature = "privacy")]
            zk_proving_key,
            #[cfg(feature = "privacy")]
            zk_verifying_key,
        }
    }

    /// Start the node
    pub async fn start(&self) -> Result<(), String> {
        info!("Starting IronDAG Node...");
        info!(
            "  Miner Address: {}",
            hex::encode(self.config.miner_address)
        );
        info!("  Data Directory: {}", self.config.data_dir);
        if self.config.enable_mining {
            info!("  Mining: ON");
        } else {
            info!("  Mining: OFF (sync-only / RPC-only node)");
        }

        // Initialize privacy manager if enabled
        #[cfg(feature = "privacy")]
        {
            if self.config.enable_privacy {
                info!("Initializing privacy layer (zk-SNARK)...");

                use crate::privacy::{generate_keys, load_keys_from_paths, PrivacyVerifier};
                use rand::thread_rng;

                let keys_result = match (
                    &self.config.privacy_proving_key_path,
                    &self.config.privacy_verifying_key_path,
                ) {
                    (Some(pk_path), Some(vk_path)) => load_keys_from_paths(
                        std::path::Path::new(pk_path),
                        std::path::Path::new(vk_path),
                    )
                    .map_err(|e| format!("Load keys from file: {}", e)),
                    _ => {
                        warn!("Privacy keys not loaded from file (proving_key_path/verifying_key_path unset). Using per-node keys; proofs will not verify across nodes. For production, use a trusted setup and set both paths.");
                        let mut rng = thread_rng();
                        generate_keys(&mut rng).map_err(|e| e.to_string())
                    }
                };

                match keys_result {
                    Ok((pk, vk)) => {
                        let verifier = PrivacyVerifier::new(vk);
                        let mut manager = crate::privacy::PrivacyManager::new(true);
                        manager.set_verifier(verifier);
                        let privacy_manager = Arc::new(tokio::sync::RwLock::new(manager));
                        use crate::privacy::PrivacyProver;
                        let privacy_prover = Arc::new(PrivacyProver::new(pk));
                        {
                            let mut bc = self.blockchain.write().await;
                            bc.set_privacy_manager(privacy_manager.clone());
                        }
                        // Safe async initialization through Arc<RpcServer>
                        self.rpc_server
                            .set_privacy_manager(privacy_manager.clone())
                            .await;
                        self.rpc_server.set_privacy_prover(privacy_prover).await;
                        if self.config.privacy_proving_key_path.is_some() {
                            info!("Privacy layer initialized (keys loaded from trusted setup)");
                        } else {
                            info!("Privacy layer initialized (keys generated)");
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to load/generate privacy keys: {}. Privacy layer disabled.",
                            e
                        );
                    }
                }
            }
        }

        // Create genesis block (only if blockchain is empty)
        // Use deterministic genesis so all nodes start with the same chain
        {
            let mut blockchain = self.blockchain.write().await;

            // GENESIS PERSISTENCE GUARD: Check both in-memory AND sled storage
            // This prevents recreating genesis with a different hash when:
            // - Sled has a genesis block but in-memory cache is empty (partial load)
            // - Node was restarted and load_from_storage hasn't run yet
            let needs_genesis = tokio::task::block_in_place(|| {
                let in_memory_empty = blockchain.get_block_count() == 0;
                if !in_memory_empty {
                    false // In-memory has blocks, no genesis needed
                } else {
                    // In-memory is empty, check sled storage
                    !blockchain.has_blocks_in_storage()
                }
            });

            if needs_genesis {
                // Load genesis config from file if specified, otherwise use defaults
                let genesis_config = if let Some(ref genesis_file) = self.config.genesis_file {
                    GenesisConfig::load_or_default(std::path::Path::new(genesis_file))
                } else {
                    // Use default genesis config (includes test account allocation)
                    GenesisConfig::default()
                };

                // Merge CLI-specified allocations with config file allocations
                let mut all_allocations = genesis_config.allocations.clone();
                for alloc in &self.config.genesis_allocations {
                    all_allocations.push(alloc.clone());
                }

                // Validate all allocations before applying
                for alloc in &all_allocations {
                    if let Err(e) = alloc.validate() {
                        return Err(format!(
                            "Invalid genesis allocation for '{}': {}",
                            alloc.address, e
                        ));
                    }
                }

                // Check for duplicate addresses
                let mut seen_addresses: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for alloc in &all_allocations {
                    let normalized = alloc.normalized_address();
                    if !seen_addresses.insert(normalized.clone()) {
                        return Err(format!(
                            "Duplicate address in genesis allocations: 0x{}",
                            normalized
                        ));
                    }
                }

                // Sort allocations by address for deterministic ordering
                all_allocations.sort();

                let genesis = create_deterministic_genesis(&genesis_config);
                let genesis_hash = genesis.hash;

                // Log genesis hash for debugging multi-node sync
                info!("GENESIS HASH: 0x{}", hex::encode(genesis_hash));

                // Add genesis block (async operation)
                blockchain
                    .add_block(genesis)
                    .await
                    .map_err(|e| e.to_string())?;

                // Apply genesis allocations from merged config
                if !all_allocations.is_empty() {
                    info!(
                        "Applying {} genesis allocation(s)...",
                        all_allocations.len()
                    );
                    blockchain
                        .apply_genesis_allocations(&all_allocations)
                        .map_err(|e| format!("Failed to apply genesis allocations: {}", e))?;
                }

                info!(
                    "Genesis block created (deterministic) with chain_id: {}",
                    genesis_config.chain_id
                );
            } else {
                let block_count = tokio::task::block_in_place(|| blockchain.get_block_count());
                info!("Loaded existing blockchain ({} blocks)", block_count);
            }
        }

        // Start network layer
        self.network_manager
            .start()
            .await
            .map_err(|e| e.to_string())?;
        info!("P2P Network started on port {}", self.config.port);

        // Load and reconnect to saved peers (peer persistence)
        if let Ok(saved) = load_peers(&self.config.data_dir).await {
            if !saved.is_empty() {
                info!("Loaded {} saved peer(s), reconnecting...", saved.len());
                for addr in saved {
                    if let Err(e) = self.connect_peer(addr).await {
                        warn!("  Reconnect {}: {}", addr, e);
                    }
                }
            }
        }

        // Connect to bootstrap peers from config with retry logic
        if !self.config.bootstrap_peers.is_empty() {
            info!(
                "Connecting to {} bootstrap peers...",
                self.config.bootstrap_peers.len()
            );
            let mut connected = 0;
            for peer_addr in &self.config.bootstrap_peers {
                let addr = match peer_addr.parse::<std::net::SocketAddr>() {
                    Ok(a) => a,
                    Err(e) => {
                        warn!("Invalid bootstrap peer address '{}': {}", peer_addr, e);
                        continue;
                    }
                };

                let mut success = false;
                for attempt in 1..=3 {
                    match self.connect_peer(addr).await {
                        Ok(_) => {
                            info!(
                                "Connected to bootstrap peer: {} (attempt {})",
                                addr, attempt
                            );
                            connected += 1;
                            success = true;
                            break;
                        }
                        Err(e) => {
                            warn!(
                                "Bootstrap peer {} attempt {}/3 failed: {}",
                                addr, attempt, e
                            );
                            if attempt < 3 {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
                if !success {
                    error!(
                        "Failed to connect to bootstrap peer {} after 3 attempts",
                        addr
                    );
                }
            }
            if connected == 0 {
                warn!("WARNING: Could not connect to any bootstrap peers. Node is isolated.");
            } else {
                info!(
                    "Connected to {}/{} bootstrap peers",
                    connected,
                    self.config.bootstrap_peers.len()
                );
            }
        }

        // Periodic peer discovery (request peers from connected peers every 5 minutes)
        let network_for_discovery = self.network_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                let peers = network_for_discovery.get_peers().await;
                for peer_addr in peers {
                    let nm = network_for_discovery.clone();
                    tokio::spawn(async move {
                        let _ = nm.request_peers_from(peer_addr).await;
                    });
                }
            }
        });
        info!("Periodic peer discovery enabled (every 5 min)");

        // Peer persistence: save peer list to disk every 5 minutes
        let network_save = self.network_manager.clone();
        let data_dir_save = self.config.data_dir.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                if let Err(e) = save_peers(&network_save, &data_dir_save).await {
                    warn!("Failed to save peers: {}", e);
                }
            }
        });

        // Note: Dedicated sync server removed - sync now uses QUIC stream type 0x02
        // Sync requests/responses flow over the main P2P QUIC connection

        // Start JSON-RPC server with ready flag
        let rpc_port = self.config.rpc_port;
        let rpc_server = self.rpc_server.clone();
        let metrics = self.metrics.clone();
        let ready_flag = self.ready.clone(); // Pass ready flag to RPC server
        let tls_cert_path = self.config.tls_cert_path.clone();
        let tls_key_path = self.config.tls_key_path.clone();
        let cors_allowed_origins = self.config.cors_allowed_origins.clone();
        let is_tls = tls_cert_path.is_some() && tls_key_path.is_some();
        let disable_tls_warning = self.config.disable_tls_warning;
        let network_for_rpc = self.network_manager.clone();
        let mining_for_rpc = self.mining_manager.clone();
        let blockchain_for_rpc = self.blockchain.clone();
        let data_dir_for_rpc = self.config.data_dir.clone();

        // Warn if RPC is bound to non-localhost without TLS
        if !is_tls && !disable_tls_warning {
            warn!("RPC server bound to 0.0.0.0:{} without TLS. This is insecure for production. Use --tls-cert and --tls-key for HTTPS, or use --disable-tls-warning if running behind a reverse proxy.", rpc_port);
        }

        // Log CORS configuration at startup
        if !cors_allowed_origins.is_empty() {
            if cors_allowed_origins.contains(&"*".to_string()) {
                info!("CORS origins: allowing all origins (development mode)");
            } else {
                info!("CORS origins: {:?}", cors_allowed_origins);
            }
        } else {
            info!("CORS origins: none allowed");
        }

        tokio::spawn(async move {
            start_rpc_server(
                rpc_port,
                rpc_server,
                metrics,
                ready_flag,
                tls_cert_path,
                tls_key_path,
                cors_allowed_origins,
                Some(network_for_rpc),
                Some(mining_for_rpc),
                Some(blockchain_for_rpc),
                data_dir_for_rpc,
            )
            .await;
        });
        let scheme = if is_tls { "https" } else { "http" };
        info!(
            "JSON-RPC API starting on {}://127.0.0.1:{}",
            scheme, rpc_port
        );

        // Start gRPC server (on port + 1000 for default)
        let grpc_port = rpc_port + 1000; // Default: 9545
        let rpc_server_grpc = self.rpc_server.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::rpc::grpc::start_grpc_server(grpc_port, rpc_server_grpc).await {
                warn!("gRPC server error: {}", e);
            }
        });
        info!("gRPC server starting on http://127.0.0.1:{}", grpc_port);
        info!("  Supports: HTTP/2, Binary Protobuf, Streaming");

        // Start gRPC v2 server (on port + 1001 for default) - only if explicitly enabled
        if self.config.enable_grpc_v2 {
            let grpc_v2_port = rpc_port + 1001; // Default: 10546
            let blockchain_v2 = self.blockchain.clone();
            let mm_v2 = Some(self.mining_manager.clone());
            let nm_v2 = Some(self.network_manager.clone());
            let chain_id_v2 = self.config.chain_id;
            warn!(
                "Starting experimental gRPC v2 server on port {}",
                grpc_v2_port
            );
            tokio::spawn(async move {
                if let Err(e) = crate::rpc::v2::start_grpc_v2_server(
                    grpc_v2_port,
                    blockchain_v2,
                    mm_v2,
                    nm_v2,
                    chain_id_v2,
                )
                .await
                {
                    warn!("gRPC v2 server error: {}", e);
                }
            });
            info!(
                "gRPC v2 server listening on http://127.0.0.1:{}",
                grpc_v2_port
            );
        }

        if self.metrics.is_some() {
            info!(
                "Metrics endpoint will be available at http://127.0.0.1:{}/metrics",
                rpc_port
            );
        }

        // Get references for block broadcasting (needed regardless of mining)
        let blockchain_broadcast = self.blockchain.clone();
        let network_broadcast = self.network_manager.clone();

        // Show configured chain ID for clarity (helps debug MetaMask mismatches)
        info!(
            "Chain ID: {} (eth_chainId: 0x{:x})",
            self.config.chain_id, self.config.chain_id
        );

        // Start mining (if enabled)
        if self.config.enable_mining {
            if self.config.single_stream {
                info!("Starting single-stream mining (Stream A only)...");
                info!("  This mode reduces CPU usage by ~66%");
                info!("  Stream A: 10s blocks, 10,000 txs, 50 IDAG reward");
            } else {
                info!("Starting BraidCore mining...");
                info!("  Stream A: 10s blocks, 10,000 txs, 50 IDAG reward");
                info!(
                    "  Stream B: {}s blocks, 5,000 txs, 25 IDAG reward",
                    pow::STREAM_B_TARGET_TIME
                );
                if self.config.enable_stream_c {
                    info!("  Stream C: 1s blocks, 1,000 txs, fee-based (high CPU)");
                } else {
                    info!("  Stream C: disabled (ZK proving deferred to v2.0)");
                }
            }

            let mining_manager = self.mining_manager.clone();
            let single_stream = self.config.single_stream;
            let enable_stream_c = self.config.enable_stream_c;

            tokio::spawn(async move {
                if single_stream {
                    mining_manager.start_mining_single_stream().await;
                } else {
                    mining_manager
                        .start_mining_streams(true, true, enable_stream_c)
                        .await;
                }
            });
        } else {
            info!("Mining disabled (RPC-only mode)");
        }

        // Broadcast blocks when mined (disabled for minimal/testing nodes)
        if self.config.enable_mining {
            tokio::spawn(async move {
                let mut last_block_count = 0;
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    let bc = blockchain_broadcast.read().await;
                    let current_count = bc.get_block_count();
                    if current_count > last_block_count {
                        // New blocks mined - broadcast them
                        let new_blocks: Vec<_> = bc.get_blocks_from(last_block_count);
                        drop(bc);

                        for block in new_blocks {
                            // is_own_block = true since these are blocks mined by this node
                            if let Err(e) = network_broadcast.broadcast_block(&block, true).await {
                                // Log error but don't fail - network errors are non-fatal
                                warn!("Failed to broadcast block: {}", e);
                            }
                        }
                        last_block_count = current_count;
                    }
                }
            });
        }

        // Stats reporting loop (disabled for minimal/testing nodes)
        if self.config.enable_mining {
            let blockchain_stats = self.blockchain.clone();
            let network_stats = self.network_manager.clone();
            let miner_address = self.config.miner_address;
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let blockchain = blockchain_stats.read().await;
                    let latest_num = blockchain.latest_block_number();
                    let tx_cnt = blockchain.transaction_count();
                    let miner_balance = blockchain.get_balance(miner_address);
                    let peer_count = network_stats.peer_count();
                    info!("Stats:");
                    info!("  Blocks: {}", latest_num + 1);
                    info!("  Transactions: {}", tx_cnt);
                    info!(
                        "  Miner Balance: {} IDAG",
                        miner_balance / 1_000_000_000_000_000_000
                    );
                    info!("  Connected Peers: {}", peer_count);
                }
            });
        }

        // Mark node as ready - initialization complete!
        *self.ready.write().await = true;
        info!("Node initialization complete - ready to accept RPC requests");

        Ok(())
    }

    /// Get mining manager reference
    pub fn mining_manager(&self) -> Arc<MiningManager> {
        self.mining_manager.clone()
    }

    /// Get blockchain reference
    pub fn blockchain(&self) -> Arc<RwLock<Blockchain>> {
        self.blockchain.clone()
    }

    /// Get network manager reference
    pub fn network_manager(&self) -> Arc<NetworkManager> {
        self.network_manager.clone()
    }

    /// Connect to a peer and trigger sync
    pub async fn connect_peer(&self, addr: SocketAddr) -> Result<(), String> {
        self.network_manager
            .connect_peer(addr)
            .await
            .map_err(|e| e.to_string())?;

        // Check if a catch-up loop is already running for this peer (deduplication)
        {
            let mut active = self.active_catchup_peers.write().await;
            if active.contains(&addr) {
                debug!("Catch-up loop already active for {}, skipping spawn", addr);
                return Ok(());
            }
            active.insert(addr);
        }

        // Trigger sync in background (uses QUIC stream type 0x02)
        let blockchain = self.blockchain.clone();
        let mining_manager = self.mining_manager.clone();
        let network_manager = self.network_manager.clone();
        let active_catchup_peers = self.active_catchup_peers.clone();
        tokio::spawn(async move {
            const CATCHUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
            // Wait for QUIC + gossip handshake
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            loop {
                let connection = {
                    let mut conn = None;
                    for attempt in 1..=10 {
                        if let Some(c) = network_manager.get_peer_connection(addr).await {
                            conn = Some(c);
                            break;
                        }
                        if attempt < 10 {
                            debug!(
                                "Waiting for QUIC connection to {} (attempt {}/10)",
                                addr, attempt
                            );
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        }
                    }
                    match conn {
                        Some(c) => c,
                        None => {
                            warn!("No QUIC connection to {} — attempting reconnect", addr);
                            let _ = network_manager.connect_peer(addr).await;
                            tokio::time::sleep(CATCHUP_INTERVAL).await;
                            continue;
                        }
                    }
                };

                // VULN-006: Get peer's expected public key for signature verification
                let expected_pubkey = {
                    let mut pubkey = None;
                    for attempt in 1..=5 {
                        if let Some(pk) = network_manager.get_peer_public_key(addr).await {
                            pubkey = Some(pk);
                            break;
                        }
                        if attempt < 5 {
                            debug!(
                                "Waiting for public key from {} (attempt {}/5)",
                                addr, attempt
                            );
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        }
                    }
                    match pubkey {
                        Some(pk) => pk,
                        None => {
                            warn!("No public key for {} yet — will retry next cycle", addr);
                            tokio::time::sleep(CATCHUP_INTERVAL).await;
                            continue;
                        }
                    }
                };

                match crate::network::sync::SyncClient::full_sync_quic(
                    &connection,
                    blockchain.clone(),
                    Some(mining_manager.clone()),
                    &expected_pubkey,
                )
                .await
                {
                    Ok(blocks_synced) => {
                        if blocks_synced > 0 {
                            info!("Catch-up: synced {} blocks from {}", blocks_synced, addr);
                            mining_manager.sync_block_allocator_to_chain_height().await;
                        }
                    }
                    Err(e) => {
                        warn!("Catch-up sync failed for {}: {}", addr, e);
                    }
                }

                tokio::time::sleep(CATCHUP_INTERVAL).await;
            }

            // Cleanup: remove from active set when loop exits
            active_catchup_peers.write().await.remove(&addr);
            debug!("Removed {} from active catch-up peers", addr);
        });

        Ok(())
    }

    /// Shutdown the node gracefully
    pub async fn shutdown(&self) -> Result<(), String> {
        info!("Shutting down node gracefully...");

        // 1. Stop mining
        info!("  Stopping mining...");
        self.mining_manager.stop_mining().await;

        // 2. Wait a bit for current operations to complete
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 3. Close peer connections
        info!("  Closing peer connections...");
        self.network_manager.shutdown().await;

        // 4. Flush database writes
        info!("  Flushing database...");
        {
            let blockchain = self.blockchain.read().await;
            if let Err(e) = blockchain.flush_database() {
                warn!("Failed to flush database: {}", e);
            }
        }

        // 5. Notify shutdown
        self.shutdown_signal.notify_waiters();

        info!("Node shutdown complete");
        Ok(())
    }
}

/// Start JSON-RPC HTTP server with port conflict detection and automatic fallback
async fn start_rpc_server(
    initial_port: u16,
    rpc_server: Arc<crate::rpc::RpcServer>,
    metrics: Option<crate::metrics::MetricsHandle>,
    ready: Arc<tokio::sync::RwLock<bool>>, // Ready flag for startup sequencing
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    cors_allowed_origins: Vec<String>,
    network_manager: Option<Arc<NetworkManager>>,
    mining_manager: Option<Arc<MiningManager>>,
    blockchain: Option<Arc<RwLock<Blockchain>>>,
    data_dir: String, // Data directory for error log path
) {
    use chrono::Utc;
    use std::path::PathBuf;
    use tokio::net::TcpListener;

    // Clone data_dir for the log_error closure
    let data_dir_for_log = data_dir.clone();

    // Helper function to log errors with timestamp and append to log file
    let log_error = move |msg: &str| {
        // Format timestamp as readable date/time
        let datetime = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let error_msg = format!("[{}] {}", datetime, msg);
        error!("{}", error_msg);

        // Append to error log file in data_dir (don't overwrite)
        let log_path = PathBuf::from(&data_dir_for_log).join("node.err");
        use std::fs::OpenOptions;
        use std::io::Write;

        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            if let Err(e) = writeln!(file, "{}", error_msg) {
                error!("Failed to write to error log: {}", e);
            }
        } else {
            // Fallback: try writing directly (will overwrite, but better than nothing)
            if let Err(e) = std::fs::write(&log_path, format!("{}\n", error_msg)) {
                error!("Failed to write error log: {}", e);
            }
        }
    };

    // Try to bind to the requested port, with automatic fallback
    let mut port = initial_port;
    let max_attempts = 10;
    let mut listener = None;

    for attempt in 0..max_attempts {
        let addr = format!("0.0.0.0:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("JSON-RPC server listening on http://{}", addr);
                listener = Some((l, port));
                break;
            }
            Err(e) => {
                if attempt == 0 {
                    log_error(&format!(
                        "RPC port {} is in use, trying alternative ports...",
                        port
                    ));
                }

                // Try next port
                port = initial_port + (attempt as u16) + 1;

                // Avoid conflicts with common ports
                if port == 8080 || port == 8081 {
                    port += 1;
                }

                if attempt == max_attempts - 1 {
                    log_error(&format!(
                        "Failed to start RPC server after {} attempts. Last error: {}",
                        max_attempts, e
                    ));
                    return;
                }
            }
        }
    }

    let (listener, actual_port) = match listener {
        Some((l, p)) => (l, p),
        None => {
            log_error("Failed to bind to any port for RPC server");
            return;
        }
    };

    // If we used a different port, log it
    if actual_port != initial_port {
        log_error(&format!(
            "RPC server using port {} instead of {} (port conflict resolved)",
            actual_port, initial_port
        ));
        warn!(
            "Note: RPC server is using port {} instead of requested port {}",
            actual_port, initial_port
        );
    }

    // Load TLS configuration if provided
    let tls_acceptor: Option<tokio_rustls::TlsAcceptor> = match (&tls_cert_path, &tls_key_path) {
        (Some(cert_path), Some(key_path)) => {
            use rustls::pki_types::pem::PemObject;
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
            use std::fs::File;
            use std::io::BufReader;
            use std::sync::Arc;

            // Load certificate from PEM file
            let cert_file = match File::open(cert_path) {
                Ok(f) => f,
                Err(e) => {
                    log_error(&format!(
                        "Failed to open TLS certificate file {}: {}",
                        cert_path, e
                    ));
                    return;
                }
            };
            let mut cert_reader = BufReader::new(cert_file);
            let certs: Vec<CertificateDer<'static>> =
                match CertificateDer::pem_reader_iter(&mut cert_reader).collect() {
                    Ok(c) => c,
                    Err(e) => {
                        log_error(&format!("Failed to parse TLS certificate: {}", e));
                        return;
                    }
                };

            // Load private key from PEM file
            let key_file = match File::open(key_path) {
                Ok(f) => f,
                Err(e) => {
                    log_error(&format!(
                        "Failed to open TLS private key file {}: {}",
                        key_path, e
                    ));
                    return;
                }
            };
            let mut key_reader = BufReader::new(key_file);
            let key: PrivateKeyDer<'static> =
                match PrivatePkcs8KeyDer::from_pem_reader(&mut key_reader) {
                    Ok(k) => PrivateKeyDer::Pkcs8(k),
                    Err(e) => {
                        log_error(&format!("Failed to parse TLS private key: {}", e));
                        return;
                    }
                };

            // Create TLS server config
            let config = match rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)
            {
                Ok(cfg) => cfg,
                Err(e) => {
                    log_error(&format!("Failed to create TLS config: {}", e));
                    return;
                }
            };

            info!("TLS enabled for RPC server");
            Some(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
        }
        _ => None,
    };

    // Update scheme based on TLS status
    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    info!(
        "JSON-RPC server ready on {}://0.0.0.0:{}",
        scheme, actual_port
    );

    // Log CORS configuration
    if !cors_allowed_origins.is_empty() {
        if cors_allowed_origins.contains(&"*".to_string()) {
            info!("CORS enabled: allowing all origins (development mode)");
        } else {
            info!("CORS enabled for origins: {:?}", cors_allowed_origins);
        }
    } else {
        info!("CORS disabled: no origins allowed");
    }

    // Clone CORS allowed origins for use in request handler
    let cors_allowed_origins_clone = cors_allowed_origins.clone();

    loop {
        // Accept TCP connection
        let accept_result = listener.accept().await;
        let (tcp_stream, peer_addr) = match accept_result {
            Ok((s, addr)) => (s, addr),
            Err(e) => {
                warn!("Error accepting connection: {}", e);
                continue;
            }
        };

        // Clone necessary data for the spawned task
        let rpc_server_clone = rpc_server.clone();
        let metrics_clone = metrics.clone();
        let ready_clone = ready.clone();
        let tls_acceptor_clone = tls_acceptor.clone();
        let cors_allowed_origins_inner = cors_allowed_origins_clone.clone();
        let network_manager_clone = network_manager.clone();
        let mining_manager_clone = mining_manager.clone();
        let blockchain_clone = blockchain.clone();

        tokio::spawn(async move {
            // Wrap with TLS if enabled, otherwise use plain TCP
            use std::pin::Pin;
            use std::task::{Context, Poll};
            use tokio::io::ReadBuf;
            use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

            // Define an enum to hold either a plain TCP stream or TLS stream
            enum Stream {
                Plain(tokio::net::TcpStream),
                Tls(tokio_rustls::server::TlsStream<tokio::net::TcpStream>),
            }

            impl AsyncRead for Stream {
                fn poll_read(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &mut ReadBuf<'_>,
                ) -> Poll<std::io::Result<()>> {
                    match self.get_mut() {
                        Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
                        Stream::Tls(s) => Pin::new(s).poll_read(cx, buf),
                    }
                }
            }

            impl AsyncWrite for Stream {
                fn poll_write(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                    buf: &[u8],
                ) -> Poll<std::io::Result<usize>> {
                    match self.get_mut() {
                        Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
                        Stream::Tls(s) => Pin::new(s).poll_write(cx, buf),
                    }
                }

                fn poll_flush(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                ) -> Poll<std::io::Result<()>> {
                    match self.get_mut() {
                        Stream::Plain(s) => Pin::new(s).poll_flush(cx),
                        Stream::Tls(s) => Pin::new(s).poll_flush(cx),
                    }
                }

                fn poll_shutdown(
                    self: Pin<&mut Self>,
                    cx: &mut Context<'_>,
                ) -> Poll<std::io::Result<()>> {
                    match self.get_mut() {
                        Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
                        Stream::Tls(s) => Pin::new(s).poll_shutdown(cx),
                    }
                }
            }

            let mut stream = if let Some(acceptor) = tls_acceptor_clone {
                // TLS enabled - perform TLS handshake
                match acceptor.accept(tcp_stream).await {
                    Ok(tls_stream) => Stream::Tls(tls_stream),
                    Err(e) => {
                        warn!("TLS handshake failed from {}: {}", peer_addr, e);
                        return;
                    }
                }
            } else {
                // Plain TCP
                Stream::Plain(tcp_stream)
            };

            // Increase buffer size to 1MB to handle large contract deployments
            let mut buffer = vec![0u8; 1048576];

            // Read HTTP headers efficiently (read in chunks, not byte-by-byte)
            // Add per-connection read timeout for header reading phase (30 seconds)
            const HEADER_READ_TIMEOUT_SECS: u64 = 30;
            let mut header_buffer = vec![0u8; 8192];
            let mut total_read = 0;
            let mut header_end = 0;
            let mut headers_complete = false;

            // Read headers in chunks until we find the empty line separator
            while total_read < header_buffer.len() - 4 {
                let chunk_size = (header_buffer.len() - total_read).min(512); // Read 512 bytes at a time
                let read_future =
                    stream.read(&mut header_buffer[total_read..total_read + chunk_size]);
                match tokio::time::timeout(
                    tokio::time::Duration::from_secs(HEADER_READ_TIMEOUT_SECS),
                    read_future,
                )
                .await
                {
                    Ok(Ok(0)) => {
                        // Connection closed
                        return;
                    }
                    Ok(Ok(n)) => {
                        total_read += n;
                        // Check for end of headers (\r\n\r\n or \n\n)
                        if total_read >= 4 {
                            // Search backwards from current position for header separator
                            for i in (3..total_read).rev() {
                                if i >= 3 && &header_buffer[i - 3..=i] == b"\r\n\r\n" {
                                    header_end = i + 1;
                                    headers_complete = true;
                                    break;
                                } else if i >= 1 && &header_buffer[i - 1..=i] == b"\n\n" {
                                    header_end = i + 1;
                                    headers_complete = true;
                                    break;
                                }
                            }
                            if headers_complete {
                                break;
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        warn!("Error reading HTTP headers: {}", e);
                        return;
                    }
                    Err(_) => {
                        warn!(
                            "HTTP header read timeout ({}s) exceeded from {}",
                            HEADER_READ_TIMEOUT_SECS, peer_addr
                        );
                        return;
                    }
                }
            }

            if !headers_complete {
                warn!("Incomplete HTTP headers received (no separator found)");
                return;
            }

            // Parse headers to find Content-Length
            let header_str = match String::from_utf8(header_buffer[..header_end].to_vec()) {
                Ok(s) => s,
                Err(_) => {
                    warn!("Invalid UTF-8 in HTTP headers");
                    return;
                }
            };

            // Log incoming request (first line only)
            if let Some(first_line) = header_str.lines().next() {
                debug!("[HTTP] Incoming request: {}", first_line);
            }

            // Extract Content-Length from headers
            let content_length = header_str.lines().find_map(|line| {
                if line.starts_with("Content-Length:") || line.starts_with("content-length:") {
                    line.split(':').nth(1)?.trim().parse::<usize>().ok()
                } else {
                    None
                }
            });

            // Extract X-API-Key header if present
            let api_key = header_str.lines().find_map(|line| {
                if line.starts_with("X-API-Key:") || line.starts_with("x-api-key:") {
                    line.splitn(2, ':').nth(1).map(|k| k.trim().to_string())
                } else {
                    None
                }
            });

            // Extract Origin header for CORS validation
            let request_origin = header_str.lines().find_map(|line| {
                if line.to_lowercase().starts_with("origin:") {
                    line.splitn(2, ':').nth(1).map(|o| o.trim().to_string())
                } else {
                    None
                }
            });

            // Helper function to check if origin is allowed and return the appropriate CORS header
            let check_cors_origin = |origin: &str, allowed: &[String]| -> Option<String> {
                if allowed.is_empty() {
                    return None;
                }
                // Allow all origins if "*" is in the list
                if allowed.contains(&"*".to_string()) {
                    return Some(format!("Access-Control-Allow-Origin: {}", origin));
                }
                // Check if the origin is in the allowed list
                if allowed.contains(&origin.to_string()) {
                    return Some(format!("Access-Control-Allow-Origin: {}", origin));
                }
                None
            };

            // Build CORS headers for this request
            let cors_allow_origin = request_origin
                .as_ref()
                .and_then(|origin| check_cors_origin(origin, &cors_allowed_origins_inner));

            // Read body based on Content-Length (more efficient than reading until newline)
            let body = if let Some(len) = content_length {
                if len > buffer.len() {
                    warn!(
                        "Content-Length {} exceeds buffer size {}",
                        len,
                        buffer.len()
                    );
                    return;
                }

                // Check if we already have body bytes in the header buffer
                // total_read contains the total bytes read into header_buffer
                // header_end is where headers end
                // So body bytes are from header_end to total_read
                let body_in_headers = total_read.saturating_sub(header_end);
                let body_already_read = body_in_headers.min(len);

                // Copy already-read body bytes to buffer
                if body_already_read > 0 {
                    buffer[..body_already_read].copy_from_slice(
                        &header_buffer[header_end..header_end + body_already_read],
                    );
                }

                // Read remaining body bytes if needed
                // Use timeout to prevent hanging on slow/stalled connections
                if body_already_read < len {
                    let read_future = async {
                        let mut bytes_read = body_already_read;
                        while bytes_read < len {
                            match stream.read(&mut buffer[bytes_read..len]).await {
                                Ok(0) => {
                                    // Connection closed before receiving all data
                                    warn!(
                                        "Connection closed early: expected {} bytes, got {}",
                                        len, bytes_read
                                    );
                                    return Err("Connection closed early");
                                }
                                Ok(n) => {
                                    bytes_read += n;
                                }
                                Err(e) => {
                                    warn!("Error reading request body: {}", e);
                                    return Err("Read error");
                                }
                            }
                        }
                        Ok(bytes_read)
                    };

                    // 30 second timeout for large contract deployments
                    match tokio::time::timeout(std::time::Duration::from_secs(30), read_future)
                        .await
                    {
                        Ok(Ok(_)) => &buffer[..len],
                        Ok(Err(_)) => {
                            warn!("Failed to read complete request body");
                            return;
                        }
                        Err(_) => {
                            warn!("Timeout reading request body (30s limit exceeded)");
                            return;
                        }
                    }
                } else {
                    &buffer[..len]
                }
            } else {
                // No Content-Length header - read until connection closes (legacy support)
                // But limit to buffer size for safety
                match stream.read(&mut buffer).await {
                    Ok(n) if n > 0 => &buffer[..n],
                    _ => {
                        warn!("No Content-Length header and empty body");
                        return;
                    }
                }
            };

            // Parse JSON body directly from bytes (more efficient)
            let json_body = match std::str::from_utf8(body) {
                Ok(s) => s.trim(),
                Err(e) => {
                    warn!("Invalid UTF-8 in request body: {}", e);
                    return;
                }
            };

            // Handle OPTIONS preflight requests (CORS)
            if header_str.starts_with("OPTIONS") {
                debug!("[HTTP] Handling OPTIONS preflight request");
                if let Some(cors_header) = cors_allow_origin {
                    let http_response = format!(
                            "HTTP/1.1 204 No Content\r\n{}\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, X-API-Key\r\nAccess-Control-Max-Age: 86400\r\nConnection: close\r\n\r\n",
                            cors_header
                        );
                    let _ = stream.write_all(http_response.as_bytes()).await;
                } else {
                    // No CORS configured - return 204 without CORS headers
                    let http_response = "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(http_response.as_bytes()).await;
                }
                return;
            }

            // Check for /health endpoint
            if header_str.starts_with("GET /health") {
                let health_status = serde_json::json!({
                    "status": "healthy",
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                });
                let response_json = serde_json::to_string(&health_status).unwrap();
                let cors_part = cors_allow_origin
                    .as_ref()
                    .map(|h| format!("\r\n{}", h))
                    .unwrap_or_default();
                let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        cors_part,
                        response_json.len(),
                        response_json
                    );
                let _ = stream.write_all(http_response.as_bytes()).await;
                return;
            }

            // Check for /ready endpoint
            if header_str.starts_with("GET /ready") {
                // Use the proper ready flag for readiness check
                // This flag is set when the node has completed initialization and sync
                let ready_guard = ready_clone.read().await;
                let is_ready = *ready_guard;
                drop(ready_guard);
                let status_code = if is_ready {
                    "200 OK"
                } else {
                    "503 Service Unavailable"
                };
                let ready_status = serde_json::json!({
                    "ready": is_ready,
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                });
                let response_json = serde_json::to_string(&ready_status).unwrap();
                let cors_part = cors_allow_origin
                    .as_ref()
                    .map(|h| format!("\r\n{}", h))
                    .unwrap_or_default();
                let http_response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status_code,
                        cors_part,
                        response_json.len(),
                        response_json
                    );
                let _ = stream.write_all(http_response.as_bytes()).await;
                return;
            }

            // Check for /metrics endpoint
            if header_str.starts_with("GET /metrics") {
                // Collect data synchronously before locking metrics
                let peer_count = network_manager_clone
                    .as_ref()
                    .map(|nm| nm.peer_count())
                    .unwrap_or(0);
                let banned_count = network_manager_clone
                    .as_ref()
                    .map(|nm| nm.banned_count_blocking())
                    .unwrap_or(0);
                let peer_latencies = network_manager_clone
                    .as_ref()
                    .map(|nm| nm.get_peer_latencies_for_metrics_blocking())
                    .unwrap_or_default();
                let mempool_sizes = mining_manager_clone
                    .as_ref()
                    .map(|mm| mm.get_mempool_sizes())
                    .unwrap_or((0, 0, 0, 0));
                let (block_height, tx_count, fees_burned) = blockchain_clone
                    .as_ref()
                    .and_then(|bc| bc.try_read().ok())
                    .map(|bc| {
                        (
                            bc.latest_block_number(),
                            bc.transaction_count(),
                            bc.get_total_fees_burned(),
                        )
                    })
                    .unwrap_or((0, 0, 0));
                let rpc_requests_total = rpc_server_clone.get_rpc_requests_total();
                let rpc_errors_total = rpc_server_clone.get_rpc_errors_total();

                let metrics_result = if let Some(ref metrics_handle) = metrics_clone {
                    let metrics_guard = metrics_handle.lock().unwrap_or_else(|e| e.into_inner());

                    // Update network metrics
                    metrics_guard.update_peers_connected(peer_count);
                    metrics_guard.update_peers_banned(banned_count);
                    for (peer, latency) in peer_latencies {
                        metrics_guard.update_peer_latency(&peer, latency);
                    }

                    // Update mempool metrics
                    metrics_guard.update_mempool_sizes(
                        mempool_sizes.0,
                        mempool_sizes.1,
                        mempool_sizes.2,
                        mempool_sizes.3,
                    );

                    // Update blockchain metrics
                    metrics_guard.update_block_height(block_height);
                    metrics_guard.update_transaction_pool_size(tx_count);

                    // Update fee metrics
                    metrics_guard.record_fees_burned(fees_burned);

                    // Update RPC metrics (using direct counter values)
                    metrics_guard
                        .rpc_requests_total
                        .with_label_values(&["all"])
                        .inc_by(rpc_requests_total as f64);
                    metrics_guard
                        .rpc_errors_total
                        .with_label_values(&["all", "all"])
                        .inc_by(rpc_errors_total as f64);

                    metrics_guard.gather()
                } else {
                    Err(prometheus::Error::Msg("Metrics not enabled".to_string()))
                };

                let cors_part = cors_allow_origin
                    .as_ref()
                    .map(|h| format!("\r\n{}", h))
                    .unwrap_or_default();

                match metrics_result {
                    Ok(metrics_text) => {
                        let http_response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                cors_part,
                                metrics_text.len(),
                                metrics_text
                            );
                        let _ = stream.write_all(http_response.as_bytes()).await;
                    }
                    Err(_) => {
                        let error_msg = "Metrics unavailable";
                        let http_response = format!(
                                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                error_msg.len(),
                                error_msg
                            );
                        let _ = stream.write_all(http_response.as_bytes()).await;
                    }
                }
                return;
            }

            // Validate JSON body
            if json_body.is_empty() {
                warn!("Empty JSON body in request");
                let error_response = r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error: Empty request body"},"id":null}"#;
                let cors_part = cors_allow_origin
                    .as_ref()
                    .map(|h| format!("\r\n{}", h))
                    .unwrap_or_default();
                let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        cors_part,
                        error_response.len(),
                        error_response
                    );
                let _ = stream.write_all(http_response.as_bytes()).await;
                return;
            }

            // Check if node is ready before processing RPC requests
            let is_node_ready = *ready_clone.read().await;
            if !is_node_ready {
                // Node is still initializing - return 503 Service Unavailable
                let error_response = r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Node is initializing, please try again in a few seconds"},"id":null}"#;
                let http_response = format!(
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nRetry-After: 5\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        error_response.len(),
                        error_response
                    );
                let _ = stream.write_all(http_response.as_bytes()).await;
                return;
            }

            // Parse JSON-RPC request (optimized: parse directly from bytes)
            let client_ip = Some(peer_addr.ip());

            // Parse the request body into single request or batch
            let parse_result = parse_jsonrpc_request(body);

            match parse_result {
                Ok(JsonRpcRequestType::Single(request)) => {
                    // Only log method name for performance (avoid full debug output)
                    let response = rpc_server_clone
                        .handle_request(request, api_key.as_deref(), client_ip)
                        .await;

                    // Serialize response directly to buffer (avoid intermediate string allocation)
                    let mut response_buffer = Vec::with_capacity(512); // Pre-allocate for common responses
                    serde_json::to_writer(&mut response_buffer, &response).unwrap_or_else(|_| {
                            // Fallback if serialization fails
                            response_buffer.clear();
                            response_buffer.extend_from_slice(r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error"},"id":null}"#.as_bytes());
                        });

                    // Pre-format HTTP response (more efficient than format! macro)
                    let content_len = response_buffer.len();
                    let cors_header_bytes = cors_allow_origin
                        .as_ref()
                        .map(|h| format!("\r\n{}\r\n", h))
                        .unwrap_or_else(|| "\r\n".to_string());
                    let mut http_response = Vec::with_capacity(200 + content_len);
                    http_response
                        .extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/json");
                    http_response.extend_from_slice(cors_header_bytes.as_bytes());
                    http_response.extend_from_slice(b"Content-Length: ");
                    http_response.extend_from_slice(content_len.to_string().as_bytes());
                    http_response.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
                    http_response.extend_from_slice(&response_buffer);

                    let _ = stream.write_all(&http_response).await;
                }
                Ok(JsonRpcRequestType::Batch(requests)) => {
                    let api_key_clone = api_key.clone();
                    let rpc_server_for_batch = rpc_server_clone.clone();
                    let responses: Vec<_> =
                        futures::future::join_all(requests.into_iter().map(move |req| {
                            let rpc_server = rpc_server_for_batch.clone();
                            let api_key_for_req = api_key_clone.clone();
                            let client_ip_for_req = client_ip;
                            async move {
                                rpc_server
                                    .handle_request(
                                        req,
                                        api_key_for_req.as_deref(),
                                        client_ip_for_req,
                                    )
                                    .await
                            }
                        }))
                        .await;

                    let response_json =
                        serde_json::to_string(&responses).unwrap_or_else(|_| "[]".to_string());
                    let content_len = response_json.len();
                    let cors_header_bytes = cors_allow_origin
                        .as_ref()
                        .map(|h| format!("\r\n{}\r\n", h))
                        .unwrap_or_else(|| "\r\n".to_string());
                    let mut http_response = Vec::with_capacity(200 + content_len);
                    http_response
                        .extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/json");
                    http_response.extend_from_slice(cors_header_bytes.as_bytes());
                    http_response.extend_from_slice(b"Content-Length: ");
                    http_response.extend_from_slice(content_len.to_string().as_bytes());
                    http_response.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
                    http_response.extend_from_slice(response_json.as_bytes());

                    let _ = stream.write_all(&http_response).await;
                }
                Err(e) => {
                    // Invalid request - provide better error message
                    warn!("[HTTP] Failed to parse JSON-RPC request: {}", e);
                    let error_response = format!(
                        r#"{{"jsonrpc":"2.0","error":{{"code":-32700,"message":"Parse error: {}"}},"id":null}}"#,
                        e.to_string().chars().take(100).collect::<String>()
                    );
                    let content_len = error_response.len();
                    let cors_header_bytes = cors_allow_origin
                        .as_ref()
                        .map(|h| format!("\r\n{}\r\n", h))
                        .unwrap_or_else(|| "\r\n".to_string());
                    let mut http_response = Vec::with_capacity(200 + content_len);
                    http_response
                        .extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/json");
                    http_response.extend_from_slice(cors_header_bytes.as_bytes());
                    http_response.extend_from_slice(b"Content-Length: ");
                    http_response.extend_from_slice(content_len.to_string().as_bytes());
                    http_response.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
                    http_response.extend_from_slice(error_response.as_bytes());

                    let _ = stream.write_all(&http_response).await;
                }
            }
        });
    }
}

/// Load peer addresses from data_dir/peers.json (peer persistence).
async fn load_peers(data_dir: &str) -> Result<Vec<SocketAddr>, String> {
    let path = PathBuf::from(data_dir).join("peers.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let json = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| e.to_string())?;
    let list: Vec<String> = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let peers: Vec<SocketAddr> = list.into_iter().filter_map(|s| s.parse().ok()).collect();
    if !peers.is_empty() {
        info!("Loaded {} peer(s) from {:?}", peers.len(), path);
    }
    Ok(peers)
}

/// Save peer list to data_dir/peers.json (peer persistence).
async fn save_peers(network: &Arc<NetworkManager>, data_dir: &str) -> Result<(), String> {
    let peers = network.get_peers().await;
    if peers.is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(data_dir).join("peers.json");
    if let Err(e) = tokio::fs::create_dir_all(data_dir).await {
        return Err(e.to_string());
    }
    let list: Vec<String> = peers.into_iter().map(|a| a.to_string()).collect();
    let json = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// JSON-RPC request type (single or batch)
enum JsonRpcRequestType {
    Single(crate::rpc::JsonRpcRequest),
    Batch(Vec<crate::rpc::JsonRpcRequest>),
}

/// Parse JSON-RPC request body into single request or batch
/// Returns Ok(RequestType) on success, Err on parse failure
fn parse_jsonrpc_request(body: &[u8]) -> Result<JsonRpcRequestType, serde_json::Error> {
    // Try single request first (most common case)
    match serde_json::from_slice::<crate::rpc::JsonRpcRequest>(body) {
        Ok(request) => Ok(JsonRpcRequestType::Single(request)),
        Err(_) => {
            // Try batch request
            serde_json::from_slice::<Vec<crate::rpc::JsonRpcRequest>>(body)
                .map(JsonRpcRequestType::Batch)
        }
    }
}

/// Create a deterministic genesis block that all nodes will share
/// This ensures all nodes start from the same chain state
fn create_deterministic_genesis(config: &GenesisConfig) -> crate::blockchain::Block {
    use crate::blockchain::{Block, BlockHeader};
    use crate::types::StreamType;

    // Fixed genesis timestamp for deterministic genesis block across all nodes
    // Jan 1, 2026 00:00:00 UTC
    const IRONDAG_GENESIS_TIMESTAMP: u64 = 1735689600;

    // Create genesis header with fixed parameters
    // EIP-1559: Genesis block uses initial base fee
    let mut header = BlockHeader::new(
        vec![], // No parent hashes
        0,      // Block number 0
        StreamType::StreamA,
        4,                               // K parameter
        crate::mining::BASE_FEE_INITIAL, // Initial base fee for EIP-1559
    );

    // Use fixed genesis timestamp when config.timestamp is 0 for deterministic genesis
    // This ensures all nodes create identical genesis blocks
    header.timestamp = if config.timestamp == 0 {
        IRONDAG_GENESIS_TIMESTAMP
    } else {
        config.timestamp
    };

    // Create genesis block with no transactions (balances set separately)
    Block::new(header, vec![])
}
