//! IronDAG Blockchain Node
//!
//! A working blockchain node with BraidCore mining
//!
//! Features:
//! - Real-time console dashboard
//! - HTTP API server (port 8080)
//! - Web dashboard support
//!
//! Copyright (c) 2024-2025 IronDAG Contributors
//! Licensed under the BUSL-1.1 License (see LICENSE file)

use irondag::node::Node;
use irondag::node::NodeConfig;
use irondag::types::{Address, GenesisAllocation};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::fs;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

/// TOML configuration file structure
/// All fields are Option<T> so unset values don't override CLI defaults
#[derive(Deserialize, Default, Debug)]
struct TomlConfig {
    // Network
    port: Option<u16>,
    rpc_port: Option<u16>,
    p2p_port: Option<u16>,
    http_api_port: Option<u16>,
    max_peers: Option<u32>,

    // Mining
    miner_address: Option<String>,
    disable_mining: Option<bool>,
    single_stream: Option<bool>,
    enable_stream_c: Option<bool>,
    mining_backend: Option<String>,

    // Chain
    chain_id: Option<u64>,
    genesis_file: Option<String>,

    // Data
    data_dir: Option<String>,

    // Peers
    bootstrap_peers: Option<Vec<String>>,
    peers: Option<Vec<String>>,
    advertise: Option<String>,

    // TLS
    tls_cert: Option<String>,
    tls_key: Option<String>,

    // CORS (supports multiple origins)
    cors_origins: Option<Vec<String>>,

    // TLS warning
    disable_tls_warning: Option<bool>,

    // Test mode
    test_txs: Option<bool>,

    // RPC Auth
    rpc_no_auth: Option<bool>,
    rpc_api_key: Option<String>,

    // Pruning
    prune_interval_secs: Option<u64>,
    keep_red_blocks: Option<bool>,
    prune_batch_size: Option<usize>,

    // Sled storage
    sled_cache_mb: Option<u64>,
    sled_flush_ms: Option<u64>,
    sled_high_throughput: Option<bool>,

    // ZK keys
    #[allow(dead_code)]
    zk_keys_dir: Option<String>,

    // Consensus
    ghostdag_k: Option<usize>,

    // RPC Rate Limiting
    rpc_rate_limit: Option<u32>,
    rpc_burst_size: Option<u32>,

    // Public IP
    public_ip: Option<String>,

    // QUIC
    quic_idle_timeout_secs: Option<u64>,
}

/// Validate a path for security (path traversal protection)
/// This is a basic validation for relative paths - use validate_path_within for
/// stronger containment checks when a base directory is known.
fn validate_path(path: &str, label: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(path);
    if path.trim().is_empty() {
        return Err(format!("{}: path cannot be empty", label));
    }
    if path.contains("..") {
        return Err(format!("{}: path traversal ('..') not allowed", label));
    }
    if p.is_absolute() {
        return Err(format!("{}: absolute paths not allowed", label));
    }
    Ok(p)
}

/// Validate that a path resolves to a location within a base directory.
/// Uses canonicalization to resolve symlinks and normalize the path.
/// Returns the canonicalized path if it's within the base directory.
#[allow(dead_code)]
fn validate_path_within(
    path: &str,
    base: &std::path::Path,
    label: &str,
) -> Result<std::path::PathBuf, String> {
    // First apply basic validation
    let _ = validate_path(path, label)?;

    // Canonicalize the base directory
    let canonical_base = base.canonicalize().map_err(|e| {
        format!(
            "{}: failed to canonicalize base directory '{}': {}",
            label,
            base.display(),
            e
        )
    })?;

    // Resolve the path relative to base (or as absolute if given)
    let full_path = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        canonical_base.join(path)
    };

    // Canonicalize the target path (this resolves .. and symlinks)
    let canonical_path = full_path
        .canonicalize()
        .map_err(|e| format!("{}: failed to canonicalize path '{}': {}", label, path, e))?;

    // Verify the path is within the base directory
    if !canonical_path.starts_with(&canonical_base) {
        return Err(format!("{}: path escapes base directory", label));
    }

    Ok(canonical_path)
}

/// Print help text to stdout
fn print_help() {
    println!("IronDAG Blockchain Node v0.1.0");
    println!();
    println!("USAGE:");
    println!("    irondag-node [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help                       Show this help message");
    println!("    --config <PATH>                  Load configuration from TOML file");
    println!("    --port <PORT>                    P2P network port - UDP/QUIC (default: 8080)");
    println!("    --p2p-port <PORT>                Alias for --port");
    println!("    --rpc-port <PORT>                JSON-RPC port (default: 8546)");
    println!("    --http-api-port <PORT>           HTTP API port (default: derived from P2P port)");
    println!("    --data-dir <PATH>                Blockchain data directory");
    println!("    --peer <ADDR>                    Connect to peer at startup (IP:PORT)");
    println!(
        "    --bootstrap-peer <ADDR>          Add bootstrap peer for initial connection (IP:PORT)"
    );
    println!(
        "    --advertise <ADDR>               P2P handshake address for external peers (IP:PORT)"
    );
    println!("    --max-peers <N>                  Maximum peer connections (default: 50)");
    println!("    --chain-id <N>                  Chain ID for EIP-155 replay protection (default: 11567)");
    println!("    --genesis-file <PATH>            Load genesis allocations from JSON file");
    println!("    --miner-address <HEX>            Miner reward address (40 hex chars, optional 0x prefix)");
    println!("    --tls-cert <PATH>                TLS certificate file for HTTPS RPC");
    println!("    --tls-key <PATH>                 TLS private key file for HTTPS RPC");
    println!("    --disable-tls-warning            Disable TLS warning for non-localhost RPC (use behind reverse proxy)");
    println!("    --cors-origin <ORIGIN>           CORS origin for RPC (e.g., \"*\", \"http://localhost:3000\")");
    println!("    --disable-mining, --no-mining    Disable mining (RPC-only mode)");
    println!("    --single-stream                  Enable single-stream mining (Stream A only, ~66% less CPU)");
    println!("    --quic-idle-timeout <SECS>       QUIC connection idle timeout in seconds (default: 30)");
    println!("    --mining-backend <BACKEND>       Mining backend for Stream B: cpu, gpu, auto (default: cpu)");
    println!("    --test-txs                       Enable test transaction generation (dev only)");
    println!("    --no-test-txs                    Disable test transactions (default)");
    println!("    --allow-unsigned-eth-send        Allow unsigned eth_sendTransaction (debug builds only)");
    println!("    --try-stun                       Enable STUN discovery for public IP");
    println!("    --rpc-no-auth                    Disable RPC authentication (dev only, NOT for production)");
    println!("    --rpc-api-key <KEY>              Set a static RPC API key (persists across restarts; also via IRONDAG_API_KEY env var)");
    println!("    --sled-cache-mb <MB>             Sled page cache in MB (default: 256)");
    println!("    --sled-flush-ms <MS>             Sled flush interval in ms (default: 1000)");
    println!("    --ghostdag-k <K>                 GhostDAG K parameter: max tips/parents (default: 4, range: 1-64)");
    println!("    --rpc-rate-limit <N>             RPC rate limit: requests per minute per IP (default: 1000)");
    println!("    --rpc-burst-size <N>             RPC burst size: burst allowance for rate limiting (default: 50)");
    println!("    --enable-grpc-v2                 Enable experimental gRPC v2 server (some methods unimplemented)");
    println!(
        "    --public-ip <IP>                 Public IP for P2P handshake (skips UDP discovery)"
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // === INITIALIZATION ===
    // ASCII art startup banner - kept as println for console output
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║      IronDAG Protocol (IDAG) - BraidCore Mining        ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("irondag=info".parse().unwrap()),
        )
        .init();

    // Create node with default config
    let miner_address: Address = Address([1u8; 20]);
    let mut config = NodeConfig {
        miner_address,
        ..Default::default()
    };

    // === CLI ARGUMENT PARSING ===
    // Usage: node [--port <p2p>] [--rpc-port <rpc>] [--data-dir <path>] [--no-mining] [--single-stream] [peer_addr] ...
    // Legacy: node [p2p_port] [rpc_port] [http_api_port] [peer_addr] ...
    // Flags:
    //   --port <port>                   P2P network port (default: 8080)
    //   --p2p-port <port>               Alias for --port
    //   --rpc-port <port>               JSON-RPC port (default: 8545)
    //   --http-api-port <port>          HTTP API port (default: derived from P2P port)
    //   --disable-mining, --no-mining   Disable mining (RPC-only mode)
    //   --single-stream                 Enable single-stream mining (Stream A only, ~66% less CPU)
    //   --test-txs                      Enable test transaction generation (dev only)
    //   --no-test-txs                   Disable test transactions (default)
    //   --data-dir <path>               Custom data directory
    //   --peer <addr>                   Connect to peer at startup
    //   --bootstrap-peer <addr>         Add bootstrap peer (can repeat); node connects on start
    //   --advertise <addr>              P2P handshake address (e.g. public IP:8080)
    //   --chain-id <N>              Chain ID for EIP-155 (default: 11567)
    let args: Vec<String> = std::env::args().collect();
    #[cfg(debug_assertions)]
    let mut generate_test_txs = false; // Disabled by default for production; use --test-txs to enable
    let mut skip_next = false;
    let mut http_api_port_set = false;
    let mut initial_peers: Vec<std::net::SocketAddr> = Vec::new();
    let mut quic_idle_timeout_secs: u64 = 30; // Default 30 seconds
    let mut skip_indices: HashSet<usize> = HashSet::new(); // Track consumed flag value indices

    // === CONFIG LOADING ===
    // Pre-pass: Find --config flag and load TOML config file FIRST
    // This allows config file values to be used as defaults, then CLI flags override them
    let mut toml_config: Option<TomlConfig> = None;
    let mut first_config_idx: Option<usize> = None;
    for (idx, arg) in args.iter().enumerate() {
        if arg == "--config" && idx + 1 < args.len() && first_config_idx.is_none() {
            first_config_idx = Some(idx);
            let config_path = &args[idx + 1];
            info!("Loading configuration from: {}", config_path);

            // Validate the path
            let validated_path = match validate_path(config_path, "--config") {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            };

            // Check if file exists
            if !validated_path.exists() {
                eprintln!("Config file not found: {}", config_path);
                std::process::exit(1);
            }

            // Read and parse the TOML file (async)
            let content = match fs::read_to_string(&validated_path).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to read config file: {}", e);
                    std::process::exit(1);
                }
            };

            match toml::from_str::<TomlConfig>(&content) {
                Ok(cfg) => {
                    info!("Config file parsed successfully");
                    toml_config = Some(cfg);
                }
                Err(e) => {
                    eprintln!("Failed to parse TOML config: {}", e);
                    std::process::exit(1);
                }
            }
        }
        // Continue scanning for additional --config flags (warn below)
    }

    // Warn if multiple --config flags found
    if let Some(first_idx) = first_config_idx {
        for (_idx, arg) in args.iter().enumerate().skip(first_idx + 2) {
            if arg == "--config" {
                warn!("Multiple --config flags found; only the first is used");
                break;
            }
        }
    }

    // Apply TOML config values as defaults (before CLI parsing)
    if let Some(ref cfg) = toml_config {
        // Conflict detection: port and p2p_port are aliases
        if cfg.port.is_some() && cfg.p2p_port.is_some() {
            eprintln!("Config file: 'port' and 'p2p_port' are aliases; specify only one");
            std::process::exit(1);
        }
        if let Some(port) = cfg.port {
            config.port = port;
            info!("[config] port = {}", port);
        }
        if let Some(p2p_port) = cfg.p2p_port {
            config.port = p2p_port;
            info!("[config] p2p_port = {}", p2p_port);
        }
        if let Some(rpc_port) = cfg.rpc_port {
            config.rpc_port = rpc_port;
            info!("[config] rpc_port = {}", rpc_port);
        }
        if let Some(http_api_port) = cfg.http_api_port {
            config.http_api_port = http_api_port;
            http_api_port_set = true;
            info!("[config] http_api_port = {}", http_api_port);
        }
        if let Some(max_peers) = cfg.max_peers {
            config.max_peers = max_peers;
            info!("[config] max_peers = {}", max_peers);
        }
        if let Some(chain_id) = cfg.chain_id {
            config.chain_id = chain_id;
            info!("[config] chain_id = {}", chain_id);
        }
        if let Some(ref data_dir) = cfg.data_dir {
            config.data_dir = data_dir.clone();
            info!("[config] data_dir = {}", data_dir);
        }
        if let Some(true) = cfg.disable_mining {
            config.enable_mining = false;
            info!("[config] disable_mining = true");
        }
        if let Some(true) = cfg.single_stream {
            config.single_stream = true;
            info!("[config] single_stream = true");
        }
        if let Some(true) = cfg.enable_stream_c {
            config.enable_stream_c = true;
            info!("[config] enable_stream_c = true");
        }
        if let Some(ref mining_backend) = cfg.mining_backend {
            config.mining_backend = mining_backend.clone();
            info!("[config] mining_backend = {}", mining_backend);
        }
        if let Some(ref miner_addr) = cfg.miner_address {
            let addr_hex = miner_addr.strip_prefix("0x").unwrap_or(miner_addr);
            if addr_hex.len() == 40 && addr_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                if let Ok(bytes) = hex::decode(addr_hex) {
                    let mut addr = [0u8; 20];
                    addr.copy_from_slice(&bytes);
                    config.miner_address = Address(addr);
                    info!("[config] miner_address = 0x{}", hex::encode(addr));
                }
            }
        }
        if let Some(ref genesis_file) = cfg.genesis_file {
            // Will be processed later if not overridden by CLI
            config.genesis_file = Some(genesis_file.clone());
            info!("[config] genesis_file = {}", genesis_file);
        }
        if let Some(ref advertise) = cfg.advertise {
            if let Ok(addr) = advertise.parse::<std::net::SocketAddr>() {
                config.advertise_addr = Some(addr.to_string());
                info!("[config] advertise = {}", advertise);
            }
        }
        if let Some(ref bootstrap_peers) = cfg.bootstrap_peers {
            for peer in bootstrap_peers {
                if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
                    config.bootstrap_peers.push(addr.to_string());
                    info!("[config] bootstrap_peer = {}", peer);
                }
            }
        }
        if let Some(ref peers) = cfg.peers {
            for peer in peers {
                if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
                    config.bootstrap_peers.push(addr.to_string());
                    info!("[config] peer = {}", peer);
                }
            }
        }
        if let Some(ref tls_cert) = cfg.tls_cert {
            config.tls_cert_path = Some(tls_cert.clone());
            info!("[config] tls_cert = {}", tls_cert);
        }
        if let Some(ref tls_key) = cfg.tls_key {
            config.tls_key_path = Some(tls_key.clone());
            info!("[config] tls_key = {}", tls_key);
        }
        if let Some(ref cors_origins) = cfg.cors_origins {
            config.cors_allowed_origins = cors_origins.clone();
            info!("[config] cors_origins = {:?}", cors_origins);
        }
        if let Some(true) = cfg.disable_tls_warning {
            config.disable_tls_warning = true;
            info!("[config] disable_tls_warning = true");
        }
        #[cfg(debug_assertions)]
        if let Some(true) = cfg.test_txs {
            generate_test_txs = true;
            info!("[config] test_txs = true");
        }
        if let Some(true) = cfg.rpc_no_auth {
            config.rpc_no_auth = true;
            info!("[config] rpc_no_auth = true");
        }
        if let Some(key) = &cfg.rpc_api_key {
            config.rpc_api_key = Some(key.clone());
            info!("[config] rpc_api_key = <set>");
        }

        // Pruning config
        if let Some(prune_interval) = cfg.prune_interval_secs {
            config.prune_interval_secs = prune_interval;
            info!("[config] prune_interval_secs = {}", prune_interval);
        }
        if let Some(keep_red) = cfg.keep_red_blocks {
            config.keep_red_blocks = keep_red;
            info!("[config] keep_red_blocks = {}", keep_red);
        }
        if let Some(batch_size) = cfg.prune_batch_size {
            config.prune_batch_size = batch_size;
            info!("[config] prune_batch_size = {}", batch_size);
        }

        // Sled storage config
        if let Some(sled_cache) = cfg.sled_cache_mb {
            config.sled_cache_mb = sled_cache;
            info!("[config] sled_cache_mb = {}", sled_cache);
        }
        if let Some(sled_flush) = cfg.sled_flush_ms {
            config.sled_flush_ms = sled_flush;
            info!("[config] sled_flush_ms = {}", sled_flush);
        }
        if let Some(sled_ht) = cfg.sled_high_throughput {
            config.sled_high_throughput = sled_ht;
            info!("[config] sled_high_throughput = {}", sled_ht);
        }

        // ZK keys config
        #[cfg(feature = "privacy")]
        if let Some(ref zk_keys_dir) = cfg.zk_keys_dir {
            config.zk_keys_dir = Some(zk_keys_dir.clone());
            info!("[config] zk_keys_dir = {}", zk_keys_dir);
        }

        // Consensus config
        if let Some(ghostdag_k) = cfg.ghostdag_k {
            config.ghostdag_k = ghostdag_k;
            info!("[config] ghostdag_k = {}", ghostdag_k);
        }

        // RPC Rate Limiting config
        if let Some(rpc_rate_limit) = cfg.rpc_rate_limit {
            config.rpc_rate_limit = rpc_rate_limit;
            info!("[config] rpc_rate_limit = {}", rpc_rate_limit);
        }
        if let Some(rpc_burst_size) = cfg.rpc_burst_size {
            config.rpc_burst_size = rpc_burst_size;
            info!("[config] rpc_burst_size = {}", rpc_burst_size);
        }

        // Public IP config
        // Maps TOML `public_ip` field to NodeConfig::public_ip (Option<IpAddr>)
        // The advertise field maps to NodeConfig::advertise_addr (Option<String>)
        if let Some(ref public_ip) = cfg.public_ip {
            if let Ok(ip) = public_ip.parse::<std::net::IpAddr>() {
                config.public_ip = Some(ip);
                info!("[config] public_ip = {}", public_ip);
            } else {
                warn!("[config] Invalid public_ip: {}", public_ip);
            }
        }

        // QUIC idle timeout config
        if let Some(timeout) = cfg.quic_idle_timeout_secs {
            quic_idle_timeout_secs = timeout;
            info!("[config] quic_idle_timeout_secs = {}", timeout);
        }
    }

    // Parse flags first (can appear at any position)
    for (idx, arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }

        // Track flag indices themselves
        if arg.starts_with("--") {
            skip_indices.insert(idx);
        }

        // Handle --help flag
        if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        }

        if (arg == "--port" || arg == "--p2p-port") && idx + 1 < args.len() {
            if let Ok(port) = args[idx + 1].parse::<u16>() {
                config.port = port;
                info!("Using P2P port: {}", port);
                if port < 1024 {
                    warn!("port {} is in the privileged range (< 1024), may require elevated permissions", port);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--rpc-port" && idx + 1 < args.len() {
            if let Ok(rpc_port) = args[idx + 1].parse::<u16>() {
                config.rpc_port = rpc_port;
                info!("Using RPC port: {}", rpc_port);
                if rpc_port < 1024 {
                    warn!("port {} is in the privileged range (< 1024), may require elevated permissions", rpc_port);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--http-api-port" && idx + 1 < args.len() {
            if let Ok(http_port) = args[idx + 1].parse::<u16>() {
                config.http_api_port = http_port;
                http_api_port_set = true;
                info!("Using HTTP API port: {}", http_port);
                if http_port < 1024 {
                    warn!("port {} is in the privileged range (< 1024), may require elevated permissions", http_port);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--peer" && idx + 1 < args.len() {
            if let Ok(peer_addr) = args[idx + 1].parse::<std::net::SocketAddr>() {
                info!("Will connect to peer: {}", peer_addr);
                initial_peers.push(peer_addr);
                // Also add to bootstrap_peers for initial connection during node.start()
                config.bootstrap_peers.push(peer_addr.to_string());
            } else {
                warn!("Invalid peer address: {}", args[idx + 1]);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--bootstrap-peer" && idx + 1 < args.len() {
            let peer_str = args[idx + 1].clone();
            match peer_str.parse::<std::net::SocketAddr>() {
                Ok(addr) => {
                    config.bootstrap_peers.push(addr.to_string());
                    info!("Bootstrap peer: {}", addr);
                }
                Err(_) => {
                    eprintln!("Invalid --bootstrap-peer address '{}'. Expected format: IP:PORT (e.g., 192.168.1.1:8080)", peer_str);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--advertise" && idx + 1 < args.len() {
            let addr_str = args[idx + 1].clone();
            match addr_str.parse::<std::net::SocketAddr>() {
                Ok(addr) => {
                    config.advertise_addr = Some(addr.to_string());
                    info!("Advertise address: {}", addr);
                }
                Err(_) => {
                    eprintln!("Invalid --advertise address '{}'. Expected format: IP:PORT (e.g., 192.168.1.1:8080)", addr_str);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--max-peers" && idx + 1 < args.len() {
            if let Ok(n) = args[idx + 1].parse::<u32>() {
                if n > 1000 {
                    eprintln!(
                        "--max-peers exceeds maximum allowed value of 1000, got {}",
                        n
                    );
                    std::process::exit(1);
                }
                config.max_peers = n;
                info!("Max peers: {}", n);
            } else {
                eprintln!("Invalid --max-peers value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--chain-id" && idx + 1 < args.len() {
            if let Ok(id) = args[idx + 1].parse::<u64>() {
                if id == 0 {
                    eprintln!("chain_id must be > 0");
                    std::process::exit(1);
                }
                config.chain_id = id;
                info!("Chain ID: {}", id);
            } else {
                eprintln!("Invalid chain ID: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--genesis-file" && idx + 1 < args.len() {
            let genesis_path = args[idx + 1].clone();
            match validate_path(&genesis_path, "--genesis-file") {
                Ok(validated_path) => {
                    if !validated_path.exists() {
                        eprintln!("--genesis-file not found: {}", genesis_path);
                        std::process::exit(1);
                    }
                    // Load and parse the genesis file
                    // NOTE: Using std::fs here because this is inside a synchronous CLI parsing loop.
                    // The config file loading above uses tokio::fs::read_to_string since it's in an async context.

                    // Check genesis file size before loading (prevent OOM from malicious files)
                    const MAX_GENESIS_FILE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB
                    let metadata = match std::fs::metadata(&validated_path) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("Cannot read genesis file metadata: {}", e);
                            std::process::exit(1);
                        }
                    };
                    if metadata.len() > MAX_GENESIS_FILE_BYTES {
                        eprintln!("Genesis file exceeds maximum size of 10MB");
                        std::process::exit(1);
                    }

                    let content = match std::fs::read_to_string(&validated_path) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Failed to read genesis file: {}", e);
                            std::process::exit(1);
                        }
                    };

                    // Parse allocations from the JSON
                    let genesis: serde_json::Value = match serde_json::from_str(&content) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("Invalid genesis JSON: {}", e);
                            std::process::exit(1);
                        }
                    };

                    let mut allocations: Vec<GenesisAllocation> = Vec::new();

                    if let Some(allocs) = genesis.get("allocations").and_then(|a| a.as_array()) {
                        for (idx, alloc) in allocs.iter().enumerate() {
                            let address =
                                alloc.get("address").and_then(|a| a.as_str()).unwrap_or("");

                            // Parse balance - support both string and number formats
                            let balance: u128 = if let Some(bal_str) =
                                alloc.get("balance").and_then(|b| b.as_str())
                            {
                                bal_str.parse().unwrap_or(0)
                            } else if let Some(bal_num) =
                                alloc.get("balance").and_then(|b| b.as_u64())
                            {
                                bal_num as u128
                            } else {
                                0
                            };

                            let allocation = GenesisAllocation {
                                address: address.to_string(),
                                balance,
                            };

                            // Validate each allocation entry
                            if let Err(e) = allocation.validate() {
                                eprintln!("Genesis allocation #{} invalid: {}", idx + 1, e);
                                std::process::exit(1);
                            }

                            allocations.push(allocation);
                        }
                    }

                    // Check for duplicate addresses
                    let mut seen_addresses: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for alloc in &allocations {
                        let normalized = alloc.normalized_address();
                        if !seen_addresses.insert(normalized.clone()) {
                            eprintln!("Duplicate address in genesis allocations: 0x{}", normalized);
                            std::process::exit(1);
                        }
                    }

                    // Sort allocations by address for deterministic ordering
                    allocations.sort();

                    let count = allocations.len();
                    config.genesis_allocations = allocations;
                    config.genesis_file = Some(genesis_path.clone());
                    info!(
                        "Loading {} genesis allocations from {}",
                        count, genesis_path
                    );
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--data-dir" && idx + 1 < args.len() {
            let data_dir_path = args[idx + 1].clone();
            match validate_path(&data_dir_path, "--data-dir") {
                Ok(validated_path) => {
                    config.data_dir = validated_path.to_string_lossy().to_string();
                    info!("Using data directory: {}", config.data_dir);
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--disable-mining" || arg == "--no-mining" {
            config.enable_mining = false;
            info!("Mining disabled (RPC-only mode)");
        } else if arg == "--single-stream" {
            config.single_stream = true;
            info!("Single-stream mining enabled (Stream A only, ~66% less CPU)");
        } else if arg == "--quic-idle-timeout" && idx + 1 < args.len() {
            if let Ok(secs) = args[idx + 1].parse::<u64>() {
                if (5..=300).contains(&secs) {
                    quic_idle_timeout_secs = secs;
                    info!("QUIC idle timeout: {} seconds", quic_idle_timeout_secs);
                } else {
                    warn!(
                        "--quic-idle-timeout must be between 5 and 300 seconds, using default: 30"
                    );
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--enable-stream-c" {
            config.enable_stream_c = true;
            info!("Stream C enabled (higher CPU usage)");
        } else if arg == "--mining-backend" && idx + 1 < args.len() {
            let backend = args[idx + 1].to_lowercase();
            match backend.as_str() {
                "cpu" | "gpu" | "auto" => {
                    config.mining_backend = backend;
                    info!("Mining backend: {}", config.mining_backend);
                }
                _ => {
                    eprintln!("--mining-backend must be one of: cpu, gpu, auto");
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--no-test-txs" {
            #[cfg(debug_assertions)]
            {
                generate_test_txs = false;
                info!("Test transaction generation disabled");
            }
            #[cfg(not(debug_assertions))]
            {
                // Silently ignored in release builds
            }
        } else if arg == "--test-txs" {
            #[cfg(debug_assertions)]
            {
                generate_test_txs = true;
                info!("Test transaction generation enabled (for development only)");
            }
            #[cfg(not(debug_assertions))]
            {
                eprintln!("--test-txs is only available in debug builds. Test transaction generation is not supported in release builds.");
                std::process::exit(1);
            }
        } else if arg == "--allow-unsigned-eth-send" {
            #[cfg(debug_assertions)]
            {
                config.allow_unsigned_eth_send = true;
                info!("Allow unsigned eth_sendTransaction (dev only)");
            }
            #[cfg(not(debug_assertions))]
            {
                warn!("--allow-unsigned-eth-send is ignored in release builds. Use debug builds for development.");
            }
        } else if arg == "--try-stun" {
            config.try_stun_discovery = true;
            info!("STUN discovery enabled (public IP for handshake when --advertise unset)");
        } else if arg == "--privacy-proving-key" && idx + 1 < args.len() {
            #[cfg(feature = "privacy")]
            {
                let key_path = args[idx + 1].clone();
                match validate_path(&key_path, "--privacy-proving-key") {
                    Ok(validated_path) => {
                        if !validated_path.exists() {
                            eprintln!("--privacy-proving-key file not found: {}", key_path);
                            std::process::exit(1);
                        }
                        config.privacy_proving_key_path =
                            Some(validated_path.to_string_lossy().to_string());
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
            }
            #[cfg(not(feature = "privacy"))]
            {
                eprintln!("--privacy-proving-key requires the 'privacy' feature to be enabled");
                std::process::exit(1);
            }
            #[cfg(feature = "privacy")]
            {
                // Consume the argument value
                skip_indices.insert(idx + 1);
                skip_next = true;
            }
        } else if arg == "--privacy-verifying-key" && idx + 1 < args.len() {
            #[cfg(feature = "privacy")]
            {
                let key_path = args[idx + 1].clone();
                match validate_path(&key_path, "--privacy-verifying-key") {
                    Ok(validated_path) => {
                        if !validated_path.exists() {
                            eprintln!("--privacy-verifying-key file not found: {}", key_path);
                            std::process::exit(1);
                        }
                        config.privacy_verifying_key_path =
                            Some(validated_path.to_string_lossy().to_string());
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
                // Consume the argument value
                skip_indices.insert(idx + 1);
                skip_next = true;
            }
            #[cfg(not(feature = "privacy"))]
            {
                eprintln!("--privacy-verifying-key requires the 'privacy' feature to be enabled");
                std::process::exit(1);
            }
        } else if arg == "--tls-cert" && idx + 1 < args.len() {
            let cert_path = args[idx + 1].clone();
            match validate_path(&cert_path, "--tls-cert") {
                Ok(validated_path) => {
                    if !validated_path.exists() {
                        eprintln!("--tls-cert file not found: {}", cert_path);
                        std::process::exit(1);
                    }
                    config.tls_cert_path = Some(validated_path.to_string_lossy().to_string());
                    info!("TLS certificate: {}", cert_path);
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--tls-key" && idx + 1 < args.len() {
            let key_path = args[idx + 1].clone();
            match validate_path(&key_path, "--tls-key") {
                Ok(validated_path) => {
                    if !validated_path.exists() {
                        eprintln!("--tls-key file not found: {}", key_path);
                        std::process::exit(1);
                    }
                    config.tls_key_path = Some(validated_path.to_string_lossy().to_string());
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
            skip_next = true;
        } else if arg == "--cors-origin" && idx + 1 < args.len() {
            let origin = args[idx + 1].clone();
            config.cors_allowed_origins.push(origin.clone());
            info!("CORS origin: {}", origin);
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--rpc-no-auth" {
            config.rpc_no_auth = true;
            warn!("RPC authentication disabled via --rpc-no-auth. Do not use in production.");
        } else if arg == "--rpc-api-key" && idx + 1 < args.len() {
            config.rpc_api_key = Some(args[idx + 1].clone());
            info!("Static RPC API key set via --rpc-api-key");
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--disable-tls-warning" {
            config.disable_tls_warning = true;
            info!("TLS warning disabled via --disable-tls-warning");
        } else if arg == "--sled-cache-mb" && idx + 1 < args.len() {
            if let Ok(cache_mb) = args[idx + 1].parse::<u64>() {
                if cache_mb == 0 {
                    eprintln!("--sled-cache-mb must be > 0");
                    std::process::exit(1);
                }
                config.sled_cache_mb = cache_mb;
                info!("Sled cache: {}MB", cache_mb);
            } else {
                eprintln!("Invalid --sled-cache-mb value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--sled-flush-ms" && idx + 1 < args.len() {
            if let Ok(flush_ms) = args[idx + 1].parse::<u64>() {
                const MAX_SLED_FLUSH_MS: u64 = 3_600_000; // 1 hour
                if flush_ms > MAX_SLED_FLUSH_MS {
                    eprintln!(
                        "--sled-flush-ms exceeds maximum allowed value of {} (1 hour), got {}",
                        MAX_SLED_FLUSH_MS, flush_ms
                    );
                    std::process::exit(1);
                }
                config.sled_flush_ms = flush_ms;
                info!("Sled flush interval: {}ms", flush_ms);
            } else {
                eprintln!("Invalid --sled-flush-ms value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--miner-address" && idx + 1 < args.len() {
            let addr_hex = args[idx + 1].clone();
            let addr_hex = addr_hex.strip_prefix("0x").unwrap_or(&addr_hex);
            if addr_hex.len() != 40 || !addr_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                eprintln!("--miner-address must be 40 hex characters (with optional 0x prefix)");
                std::process::exit(1);
            }
            match hex::decode(addr_hex) {
                Ok(bytes) => {
                    let mut addr = [0u8; 20];
                    addr.copy_from_slice(&bytes);
                    config.miner_address = Address(addr);
                    info!("Miner address: 0x{}", hex::encode(addr));
                }
                Err(e) => {
                    eprintln!("Invalid miner address hex: {}", e);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--zk-keys-dir" && idx + 1 < args.len() {
            #[cfg(feature = "privacy")]
            {
                let zk_dir = args[idx + 1].clone();
                match validate_path(&zk_dir, "--zk-keys-dir") {
                    Ok(validated_path) => {
                        // Check that directory exists (or warn if it doesn't - keys may be loaded later)
                        if !validated_path.exists() {
                            warn!("ZK keys directory does not exist: {}", zk_dir);
                        }
                        config.zk_keys_dir = Some(validated_path.to_string_lossy().to_string());
                        info!("ZK keys directory: {}", zk_dir);
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
                skip_indices.insert(idx + 1);
                skip_next = true;
            }
            #[cfg(not(feature = "privacy"))]
            {
                eprintln!("--zk-keys-dir requires the 'privacy' feature to be enabled");
                std::process::exit(1);
            }
        } else if arg == "--ghostdag-k" && idx + 1 < args.len() {
            if let Ok(k) = args[idx + 1].parse::<usize>() {
                if !(1..=64).contains(&k) {
                    eprintln!("--ghostdag-k must be between 1 and 64, got {}", k);
                    std::process::exit(1);
                }
                config.ghostdag_k = k;
                info!("GhostDAG K parameter: {}", k);
            } else {
                eprintln!("Invalid --ghostdag-k value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--rpc-rate-limit" && idx + 1 < args.len() {
            if let Ok(limit) = args[idx + 1].parse::<u32>() {
                if limit == 0 {
                    eprintln!("--rpc-rate-limit must be > 0");
                    std::process::exit(1);
                }
                const MAX_RPC_RATE_LIMIT: u32 = 100_000;
                if limit > MAX_RPC_RATE_LIMIT {
                    eprintln!(
                        "--rpc-rate-limit exceeds maximum allowed value of {}, got {}",
                        MAX_RPC_RATE_LIMIT, limit
                    );
                    std::process::exit(1);
                }
                config.rpc_rate_limit = limit;
                info!("RPC rate limit: {} requests/minute per IP", limit);
            } else {
                eprintln!("Invalid --rpc-rate-limit value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--rpc-burst-size" && idx + 1 < args.len() {
            if let Ok(burst) = args[idx + 1].parse::<u32>() {
                if burst == 0 {
                    eprintln!("--rpc-burst-size must be > 0");
                    std::process::exit(1);
                }
                const MAX_RPC_BURST_SIZE: u32 = 10_000;
                if burst > MAX_RPC_BURST_SIZE {
                    eprintln!(
                        "--rpc-burst-size exceeds maximum allowed value of {}, got {}",
                        MAX_RPC_BURST_SIZE, burst
                    );
                    std::process::exit(1);
                }
                config.rpc_burst_size = burst;
                info!("RPC burst size: {}", burst);
            } else {
                eprintln!("Invalid --rpc-burst-size value: {}", args[idx + 1]);
                std::process::exit(1);
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        } else if arg == "--enable-grpc-v2" {
            config.enable_grpc_v2 = true;
            warn!("gRPC v2 server enabled (experimental - some methods are unimplemented)");
        } else if arg == "--public-ip" && idx + 1 < args.len() {
            let ip_str = args[idx + 1].clone();
            match ip_str.parse::<std::net::IpAddr>() {
                Ok(ip) => {
                    config.public_ip = Some(ip);
                    info!("Public IP: {}", ip);
                }
                Err(_) => {
                    eprintln!("Invalid --public-ip value: {}", ip_str);
                    std::process::exit(1);
                }
            }
            skip_indices.insert(idx + 1);
            skip_next = true;
        }
    }

    // After the flag loop, check for unrecognized positional args
    for (idx, arg) in args.iter().enumerate().skip(1) {
        if !arg.starts_with("--") && !skip_indices.contains(&idx) {
            warn!(
                "Unrecognized positional argument '{}' — use --flag syntax instead",
                arg
            );
        }
    }

    // Validate TLS config: both cert and key must be provided together
    let has_cert = config.tls_cert_path.is_some();
    let has_key = config.tls_key_path.is_some();
    if has_cert != has_key {
        eprintln!("TLS requires both --tls-cert and --tls-key to be provided together");
        std::process::exit(1);
    }
    if has_cert && has_key {
        info!("TLS enabled for RPC server (HTTPS)");
    }

    // Auto-derive HTTP API port from P2P port if not explicitly set
    if !http_api_port_set && config.port != 8080 {
        config.http_api_port = config.port + 10; // E.g., P2P 9090 -> HTTP 9100
        info!("HTTP API port auto-derived: {}", config.http_api_port);
    }

    // Apply QUIC idle timeout from CLI to config
    config.quic_idle_timeout_secs = quic_idle_timeout_secs;

    // === NODE STARTUP ===
    let node = Arc::new(Node::new(config.clone()).await);

    // Start the node (this also starts mining if enabled)
    node.start().await?;

    // Connect to initial peers from --peer flags
    for peer_addr in &initial_peers {
        info!("Connecting to peer: {}", peer_addr);
        if let Err(e) = node.connect_peer(*peer_addr).await {
            warn!("Failed to connect to {}: {}", peer_addr, e);
        }
    }

    // Connect to peers if provided as raw addresses (skip flags and their values)
    // NOTE: This second pass processes positional arguments as peer addresses.
    // skip_indices (HashSet<usize>) is used in the first pass to track consumed flag values.
    // skip_flag_value (bool) is used here because this pass iterates differently (by value, not index).
    // Two mechanisms exist because the first pass uses index-based tracking while this pass
    // uses a state machine approach for the same purpose.
    #[cfg(feature = "privacy")]
    let flags_with_values = [
        "--config",
        "--port",
        "--p2p-port",
        "--rpc-port",
        "--http-api-port",
        "--data-dir",
        "--peer",
        "--bootstrap-peer",
        "--advertise",
        "--max-peers",
        "--chain-id",
        "--genesis-file",
        "--miner-address",
        "--privacy-proving-key",
        "--privacy-verifying-key",
        "--tls-cert",
        "--tls-key",
        "--cors-origin",
        "--mining-backend",
        "--sled-cache-mb",
        "--sled-flush-ms",
        "--zk-keys-dir",
        "--ghostdag-k",
        "--rpc-rate-limit",
        "--rpc-burst-size",
    ];
    #[cfg(not(feature = "privacy"))]
    let flags_with_values = [
        "--config",
        "--port",
        "--p2p-port",
        "--rpc-port",
        "--http-api-port",
        "--data-dir",
        "--peer",
        "--bootstrap-peer",
        "--advertise",
        "--max-peers",
        "--chain-id",
        "--genesis-file",
        "--miner-address",
        "--tls-cert",
        "--tls-key",
        "--cors-origin",
        "--mining-backend",
        "--sled-cache-mb",
        "--sled-flush-ms",
        "--ghostdag-k",
        "--rpc-rate-limit",
        "--rpc-burst-size",
    ];
    let mut skip_flag_value = false;

    for arg in args.iter().skip(1) {
        // Skip if this is a flag's value
        if skip_flag_value {
            skip_flag_value = false;
            continue;
        }

        // Skip flags
        if arg.starts_with("--") {
            if flags_with_values.iter().any(|f| *f == arg) {
                skip_flag_value = true;
            }
            continue;
        }

        // Skip numeric values that look like port numbers (for legacy parsing)
        if arg.parse::<u16>().is_ok() && !arg.contains(':') {
            continue;
        }

        // Try to parse as peer address
        if let Ok(peer_addr) = arg.parse::<std::net::SocketAddr>() {
            info!("Connecting to peer: {}", peer_addr);
            if let Err(e) = node.connect_peer(peer_addr).await {
                warn!("Failed to connect to {}: {}", peer_addr, e);
            }
        }
    }

    // Generate some test transactions (optional) - delayed to let RPC start
    // Only available in debug builds - physically impossible in release builds
    #[cfg(debug_assertions)]
    if generate_test_txs {
        let node_clone = node.clone();
        tokio::spawn(async move {
            // Wait 3 seconds to let RPC server fully initialize
            sleep(Duration::from_secs(3)).await;

            info!("Generating signed test transactions...");
            let mining_manager = node_clone.mining_manager();

            // Generate a random test keypair for Alice (Ed25519)
            use ed25519_dalek::SigningKey;
            use irondag::blockchain::Transaction;
            use rand::rngs::OsRng;
            use sha3::{Digest, Keccak256};

            let signing_key = SigningKey::generate(&mut OsRng);
            let alice_secret_key = signing_key.to_bytes();
            let public_key = signing_key.verifying_key();
            let mut hasher = Keccak256::new();
            hasher.update(public_key.as_bytes());
            let hash = hasher.finalize();
            let mut alice = Address::zero();
            alice.0.copy_from_slice(&hash[12..32]);

            let bob = Address([3u8; 20]);

            info!("Alice address: 0x{}", hex::encode(alice));

            // WARNING: Direct balance injection bypasses consensus — test/dev only
            // This code is gated behind #[cfg(debug_assertions)] so it cannot run in release builds.
            // Direct balance manipulation will cause peer disagreements and should NEVER be used in production.
            warn!("Test mode: directly injecting balance for test account — this bypasses consensus and will cause peer disagreements");

            // Give Alice some initial balance
            {
                let blockchain = node_clone.blockchain();
                let mut bc = blockchain.write().await;
                bc.set_balance(alice, 1_000_000_000_000_000_000_000)
                    .unwrap_or_else(|e| {
                        warn!("Failed to set balance: {}", e);
                    }); // 1000 tokens
                info!("Alice balance: 1000 tokens");
            }

            // Add signed transactions to the pool
            for i in 0..50 {
                let tx = Transaction::new(
                    alice,
                    bob,
                    10_000_000_000_000_000, // 0.01 tokens
                    1_000_000_000_000_000,  // 0.001 token fee
                    i,
                )
                .sign(&alice_secret_key); // Sign the transaction!

                if mining_manager.add_transaction(tx).await.is_ok() && i % 10 == 0 {
                    info!("Added signed transaction {}", i + 1);
                }
            }
            info!("All 50 test transactions signed and added");
        });
    }

    info!("Node is running!");
    if !config.enable_mining {
        info!("Mining: OFF (this node only syncs and serves RPC)");
    }
    info!("Press Ctrl+C to stop");

    // === SIGNAL HANDLING ===
    // Create shutdown notifier for clean shutdown
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());

    // BPR-009: Setup graceful shutdown with SIGTERM/SIGINT support
    let node_shutdown = node.clone();
    let shutdown_notify_clone = shutdown_notify.clone();
    tokio::spawn(async move {
        // Handle SIGINT (Ctrl+C) on all platforms
        #[cfg(unix)]
        {
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to register SIGTERM handler: {}", e);
                        // Fall back to just Ctrl+C
                        let _ = signal::ctrl_c().await;
                        info!("Received SIGINT (Ctrl+C)");
                        if let Err(e) = (*node_shutdown).shutdown().await {
                            warn!("Error during shutdown: {}", e);
                        }
                        shutdown_notify_clone.notify_one();
                        return;
                    }
                };

            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Received SIGINT (Ctrl+C)");
                }
                _ = sigterm.recv() => {
                    info!("Received SIGTERM");
                }
            }
        }

        #[cfg(not(unix))]
        {
            let _ = signal::ctrl_c().await;
            info!("Received SIGINT (Ctrl+C)");
        }

        info!("Starting graceful shutdown...");
        if let Err(e) = (*node_shutdown).shutdown().await {
            warn!("Error during shutdown: {}", e);
        }
        info!("Shutdown complete.");
        shutdown_notify_clone.notify_one();
    });

    info!("RPC API available on http://127.0.0.1:{}", config.rpc_port);
    info!("Web Dashboard: Open irondag-explorer-frontend/index.html in browser");
    sleep(Duration::from_secs(1)).await;

    // Real-time console dashboard - DISABLED to avoid lock contention with RPC
    // Keeping node running without dashboard spam
    info!("Node running silently (dashboard disabled to prevent RPC lock contention)");
    info!(
        "Use JSON-RPC API on port {} to query node status",
        config.rpc_port
    );
    info!("Example: curl -X POST -H 'Content-Type: application/json' --data '{{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}}' http://127.0.0.1:{}", config.rpc_port);

    // Wait for shutdown signal
    shutdown_notify.notified().await;
    info!("Main loop exiting cleanly");
    Ok(())
}

/// Show real-time dashboard
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
fn show_dashboard(
    blocks: u64,
    txs: usize,
    miner_balance: u128,
    stream_a: u64,
    stream_b: u64,
    stream_c: u64,
    dag_stats: irondag::consensus::DAGStats,
    tps: f64,
) {
    // Screen clearing disabled to avoid wiping console output

    let blue_ratio = if dag_stats.total_blocks > 0 {
        (dag_stats.blue_blocks as f64 / dag_stats.total_blocks as f64) * 100.0
    } else {
        0.0
    };

    info!("IronDAG Blockchain - Mining Dashboard");
    info!("Network Stats");
    info!("Total Blocks: {}", blocks + 1);
    info!("Total Transactions: {}", txs);
    info!(
        "Miner Balance: {} IDAG",
        miner_balance / 1_000_000_000_000_000_000
    );
    info!("Mining Streams");
    info!(
        "Stream A (ASIC):     {:<4} blocks | 50 IDAG/block | 10s blocks",
        stream_a
    );
    info!(
        "Stream B (CPU): {:<4} blocks | 25 IDAG/block | 1s blocks (GPU planned)",
        stream_b
    );
    info!(
        "Stream C (ZK):       {:<4} blocks | Fees only      | 100ms blocks",
        stream_c
    );
    info!("GhostDAG Consensus");
    info!(
        "Blue Blocks: {:<4} | Red Blocks: {:<4} | Blue Ratio: {:.1}%",
        dag_stats.blue_blocks, dag_stats.red_blocks, blue_ratio
    );
    info!("TPS (60s): {:.2}", tps);
    info!(
        "Avg Block Size: {:<4} bytes | Avg Txs/Block: {:.1}",
        dag_stats.avg_block_size, dag_stats.avg_txs_per_block
    );
    info!("Press Ctrl+C to stop mining");
    info!("Web Dashboard: Open irondag-explorer-frontend/index.html in browser");
}

// =============================================================================
// UNIT TESTS
// =============================================================================
//
// TEST-01: Argument/config validation tests
// TEST-02: Path traversal edge cases
// TEST-03: Debug assertions gate verification (TomlConfig deserialization)
// TEST-04: Port conflict detection (TomlConfig port fields)
//
// Note: The test transaction generation code is gated with #[cfg(debug_assertions)].
// This is verified at compile time - release builds will not include that code path.
// We cannot runtime-test #[cfg] attributes, but the build configuration ensures
// the code is physically absent in release builds.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // =========================================================================
    // TEST-01: Argument/config validation tests
    // =========================================================================

    /// Test validate_path with a normal relative path -> Ok
    #[test]
    fn test_validate_path_normal_relative() {
        let result = validate_path("config/node.toml", "test_path");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path, PathBuf::from("config/node.toml"));
    }

    /// Test validate_path with ".." in path -> Err
    #[test]
    fn test_validate_path_traversal() {
        let result = validate_path("../etc/passwd", "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("path traversal"));
    }

    /// Test validate_path with empty string -> Err
    #[test]
    fn test_validate_path_empty() {
        let result = validate_path("", "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("empty"));
    }

    /// Test validate_path with absolute path -> Err (MED-01 added this check)
    /// Note: On Windows, Unix-style paths like "/etc/passwd" are NOT absolute.
    /// Windows absolute paths require drive letter prefixes (e.g., "C:\\").
    #[test]
    fn test_validate_path_absolute() {
        // Use a platform-appropriate absolute path
        #[cfg(windows)]
        let abs_path = "C:\\Windows\\System32";
        #[cfg(not(windows))]
        let abs_path = "/etc/passwd";

        let result = validate_path(abs_path, "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("absolute"));
    }

    /// Test validate_path_within with a path inside the base -> Ok
    #[test]
    fn test_validate_path_within_inside_base() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base = temp_dir.path();

        // Create a subdirectory inside the temp dir
        let subdir = base.join("subdir");
        std::fs::create_dir(&subdir).expect("Failed to create subdir");

        // Create a file inside the subdir
        let file_path = subdir.join("config.toml");
        std::fs::write(&file_path, "test").expect("Failed to write file");

        let result = validate_path_within("subdir/config.toml", base, "test_path");
        assert!(result.is_ok());

        // Verify the resolved path is correct
        let resolved = result.unwrap();
        assert_eq!(resolved, file_path.canonicalize().unwrap());
    }

    /// Test validate_path_within with a path escaping the base -> Err
    #[test]
    fn test_validate_path_within_escapes_base() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let base = temp_dir.path();

        // Note: validate_path already rejects ".." at the basic level,
        // so validate_path_within will also reject it
        let result = validate_path_within("../etc/passwd", base, "test_path");
        assert!(result.is_err());
    }

    /// Test print_help() doesn't panic
    #[test]
    fn test_print_help_no_panic() {
        // Just verify it runs without panicking
        print_help();
    }

    // =========================================================================
    // TEST-02: Path traversal edge cases
    // =========================================================================

    /// Path with ".." embedded: "foo/../bar" -> Err
    #[test]
    fn test_path_traversal_embedded() {
        let result = validate_path("foo/../bar", "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("path traversal"));
    }

    /// Path with multiple "..": "../../etc/passwd" -> Err
    #[test]
    fn test_path_traversal_multiple() {
        let result = validate_path("../../etc/passwd", "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("path traversal"));
    }

    /// Normal nested path: "config/node.toml" -> Ok
    #[test]
    fn test_path_normal_nested() {
        let result = validate_path("config/node.toml", "test_path");
        assert!(result.is_ok());
    }

    /// Path with dots in filename (not traversal): "node.config.toml" -> Ok
    #[test]
    fn test_path_dots_in_filename() {
        let result = validate_path("node.config.toml", "test_path");
        assert!(result.is_ok());
    }

    /// Path with single dot (current dir): "./config.toml" -> Ok (not traversal)
    #[test]
    fn test_path_single_dot() {
        // "./" is current directory, not traversal
        let result = validate_path("./config.toml", "test_path");
        // Note: "./" does NOT contain "..", so it should be allowed
        assert!(result.is_ok());
    }

    /// Windows absolute path -> Err (Windows-specific test)
    #[test]
    #[cfg(windows)]
    fn test_path_windows_absolute() {
        let result = validate_path("C:\\Windows\\System32", "test_path");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("absolute"));
    }

    // =========================================================================
    // TEST-03: Debug assertions gate verification
    // =========================================================================
    //
    // Note: We cannot runtime-test #[cfg(debug_assertions)] because the code
    // is physically absent in release builds. The following comment documents
    // that the test tx generation code is verified by the build configuration.
    //
    // The test transaction generator in main() (lines ~1116-1178) is gated with
    // #[cfg(debug_assertions)]. In release builds, the Rust compiler completely
    // removes this code, making it impossible to accidentally enable in production.
    //
    // Verification: Compile with `cargo build --release` and inspect the binary
    // to confirm the test transaction code is absent.

    /// Test TomlConfig can be deserialized from minimal TOML
    #[test]
    fn test_toml_config_minimal() {
        let toml_str = r#"
            port = 8080
            rpc_port = 8546
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert_eq!(cfg.port, Some(8080));
        assert_eq!(cfg.rpc_port, Some(8546));
        assert!(cfg.quic_idle_timeout_secs.is_none());
    }

    /// Test TomlConfig quic_idle_timeout_secs field works
    #[test]
    fn test_toml_config_quic_timeout() {
        let toml_str = r#"
            quic_idle_timeout_secs = 60
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert_eq!(cfg.quic_idle_timeout_secs, Some(60));
    }

    /// Test TomlConfig with all optional fields set
    #[test]
    fn test_toml_config_all_fields() {
        let toml_str = r#"
            port = 8080
            rpc_port = 8546
            p2p_port = 9090
            http_api_port = 8090
            max_peers = 100
            miner_address = "0x1234567890123456789012345678901234567890"
            disable_mining = true
            single_stream = true
            enable_stream_c = false
            mining_backend = "cpu"
            chain_id = 11567
            genesis_file = "genesis.json"
            data_dir = "data"
            bootstrap_peers = ["127.0.0.1:8081"]
            peers = ["127.0.0.1:8082"]
            advertise = "0.0.0.0:8080"
            tls_cert = "cert.pem"
            tls_key = "key.pem"
            cors_origins = ["*"]
            disable_tls_warning = true
            test_txs = true
            rpc_no_auth = true
            prune_interval_secs = 3600
            keep_red_blocks = false
            prune_batch_size = 1000
            sled_cache_mb = 512
            sled_flush_ms = 500
            sled_high_throughput = true
            zk_keys_dir = "keys"
            ghostdag_k = 8
            rpc_rate_limit = 500
            rpc_burst_size = 25
            public_ip = "192.168.1.1"
            quic_idle_timeout_secs = 45
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert_eq!(cfg.port, Some(8080));
        assert_eq!(cfg.rpc_port, Some(8546));
        assert_eq!(cfg.p2p_port, Some(9090));
        assert_eq!(cfg.http_api_port, Some(8090));
        assert_eq!(cfg.max_peers, Some(100));
        assert_eq!(
            cfg.miner_address,
            Some("0x1234567890123456789012345678901234567890".to_string())
        );
        assert_eq!(cfg.disable_mining, Some(true));
        assert_eq!(cfg.single_stream, Some(true));
        assert_eq!(cfg.enable_stream_c, Some(false));
        assert_eq!(cfg.mining_backend, Some("cpu".to_string()));
        assert_eq!(cfg.chain_id, Some(11567));
        assert_eq!(cfg.genesis_file, Some("genesis.json".to_string()));
        assert_eq!(cfg.data_dir, Some("data".to_string()));
        assert_eq!(cfg.quic_idle_timeout_secs, Some(45));
    }

    // =========================================================================
    // TEST-04: Port conflict detection
    // =========================================================================

    /// Test TomlConfig with both port and p2p_port set (conflict detectable)
    #[test]
    fn test_toml_config_port_conflict_detectable() {
        let toml_str = r#"
            port = 8080
            p2p_port = 9090
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        // Both are set — runtime code should detect and reject this
        assert!(cfg.port.is_some() && cfg.p2p_port.is_some());
    }

    /// Test TomlConfig with only port set (no conflict)
    #[test]
    fn test_toml_config_port_only() {
        let toml_str = r#"
            port = 8080
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert_eq!(cfg.port, Some(8080));
        assert!(cfg.p2p_port.is_none());
    }

    /// Test TomlConfig with only p2p_port set (no conflict)
    #[test]
    fn test_toml_config_p2p_port_only() {
        let toml_str = r#"
            p2p_port = 9090
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert!(cfg.port.is_none());
        assert_eq!(cfg.p2p_port, Some(9090));
    }

    /// Test TomlConfig with neither port nor p2p_port set (defaults will be used)
    #[test]
    fn test_toml_config_no_ports() {
        let toml_str = r#"
            rpc_port = 8546
        "#;
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert!(cfg.port.is_none());
        assert!(cfg.p2p_port.is_none());
        assert_eq!(cfg.rpc_port, Some(8546));
    }

    /// Test empty TomlConfig (all defaults)
    #[test]
    fn test_toml_config_empty() {
        let toml_str = "";
        let cfg: TomlConfig = toml::from_str(toml_str).expect("Failed to parse TOML");
        assert!(cfg.port.is_none());
        assert!(cfg.rpc_port.is_none());
        assert!(cfg.p2p_port.is_none());
        assert!(cfg.quic_idle_timeout_secs.is_none());
    }
}
