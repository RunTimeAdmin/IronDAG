//! Minimal RPC-Only Node for Testing
//!
//! Stripped-down node with configurable features:
//! - RPC server (always enabled)
//! - Mining (optional via --enable-mining)
//! - Stats dashboard (optional via --enable-stats)
//! - Minimal lock contention with fine-grained locking

use irondag_blockchain::node::Node;
use irondag_blockchain::node::NodeConfig;
use irondag_blockchain::types::Address;
use tokio::signal;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("irondag=info".parse().unwrap()),
        )
        .init();

    info!("IronDAG Minimal Node - Progressive Feature Testing");
    info!("Fine-grained locking architecture enabled");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut config = NodeConfig::default();

    // Parse flags
    let mut enable_mining = false;
    let mut enable_stats = false;

    for (idx, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "--enable-mining" => {
                enable_mining = true;
                info!("Mining will be enabled");
            }
            "--enable-stats" => {
                enable_stats = true;
                info!("Stats dashboard will be enabled");
            }
            "--data-dir" if idx + 1 < args.len() => {
                config.data_dir = args[idx + 1].clone();
            }
            _ => {
                // Try parsing as RPC port if it's a number
                if let Ok(rpc_port) = arg.parse::<u16>() {
                    if rpc_port > 1024 && rpc_port < 65535 {
                        config.rpc_port = rpc_port;
                    }
                }
            }
        }
    }

    // Apply configuration
    config.enable_mining = enable_mining;

    info!("Config:");
    info!("RPC Port: {}", config.rpc_port);
    info!("Data Dir: {}", config.data_dir);
    info!(
        "Mining: {}",
        if enable_mining { "ENABLED" } else { "DISABLED" }
    );
    info!(
        "Stats: {}",
        if enable_stats { "ENABLED" } else { "DISABLED" }
    );

    // Create and start node
    let node = Node::new(config.clone()).await;
    node.start().await?;

    info!("Minimal node started!");
    info!("RPC: http://127.0.0.1:{}", config.rpc_port);
    info!("Test: curl -X POST http://127.0.0.1:{} -H 'Content-Type: application/json' -d '{{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}}'", config.rpc_port);

    // Optionally start stats loop if requested
    if enable_stats {
        let blockchain_stats = node.blockchain();
        let network_stats = node.network_manager();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let blockchain = blockchain_stats.read().await;
                let latest_num = blockchain.latest_block_number();
                let tx_cnt = blockchain.transaction_count();
                let miner_balance = blockchain.get_balance(Address([1u8; 20]));
                let peer_count = network_stats.peer_count();
                info!("Stats:");
                info!("Blocks: {}", latest_num + 1);
                info!("Transactions: {}", tx_cnt);
                info!(
                    "Miner Balance: {} IDAG",
                    miner_balance / 1_000_000_000_000_000_000
                );
                info!("Connected Peers: {}", peer_count);
            }
        });
    }

    // Status message
    if enable_mining {
        info!("Mining active - RPC should remain responsive");
    } else {
        info!("RPC-only mode - no background mining");
    }
    info!("Press Ctrl+C to stop");

    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    info!("Shutting down...");
    node.shutdown().await?;

    Ok(())
}
