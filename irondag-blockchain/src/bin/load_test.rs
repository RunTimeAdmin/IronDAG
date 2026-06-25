//! Load Testing Binary for IronDAG Blockchain
//!
//! Generates signed EIP-155 transactions and benchmarks TPS
//!
//! Usage:
//!   load_test [options]
//!
//! Options:
//!   --rpc-url <url>       RPC endpoint (default: http://127.0.0.1:8546)
//!   --target-tps <n>      Target transactions per second (default: 100)
//!   --duration <secs>     Test duration in seconds (default: 60)
//!   --accounts <n>        Number of sender accounts to generate (default: 10)
//!   --value <wei>         Value per transaction in wei (default: 1000000000000000)
//!   --chain_id 11567)
//!   --ramp-up             Enable ramp-up phases (100->500->1000 TPS)
//!   --json                Output results in JSON format

use alloy_rlp::{BytesMut, Encodable};
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use tracing::{error, info};

/// Account with keypair and nonce tracking
#[derive(Clone)]
struct Account {
    signing_key: SigningKey,
    address: [u8; 20],
    nonce: u64,
}

impl Account {
    fn new(rng: &mut rand::rngs::ThreadRng) -> Self {
        let signing_key = SigningKey::random(rng);
        let verifying_key = signing_key.verifying_key();

        // Derive address (Ethereum-style: last 20 bytes of Keccak256(pubkey))
        let pubkey_bytes = verifying_key.to_sec1_bytes();
        let mut hasher = Keccak256::new();
        hasher.update(&pubkey_bytes[1..]); // Skip 0x04 prefix
        let hash = hasher.finalize();

        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..]);

        Account {
            signing_key,
            address,
            nonce: 0,
        }
    }
}

/// Transaction for signing (EIP-155 format)
#[derive(Debug, Clone)]
struct UnsignedTransaction {
    nonce: u64,
    gas_price: u128,
    gas_limit: u64,
    to: [u8; 20],
    value: u128,
    data: Vec<u8>,
    chain_id: u64,
}

impl UnsignedTransaction {
    /// Create signing hash using EIP-155 format
    fn signing_hash(&self) -> [u8; 32] {
        // EIP-155: RLP([nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0])
        let mut buf = BytesMut::new();
        self.nonce.encode(&mut buf);
        self.gas_price.encode(&mut buf);
        self.gas_limit.encode(&mut buf);
        if self.to == [0u8; 20] {
            let empty: &[u8] = &[];
            empty.encode(&mut buf);
        } else {
            self.to.as_ref().encode(&mut buf);
        }
        self.value.encode(&mut buf);
        self.data.encode(&mut buf);
        self.chain_id.encode(&mut buf);
        0u8.encode(&mut buf);
        0u8.encode(&mut buf);

        let mut hasher = Keccak256::new();
        hasher.update(&buf[..]);
        let hash = hasher.finalize();

        let mut result = [0u8; 32];
        result.copy_from_slice(&hash);
        result
    }

    /// Sign transaction and return RLP-encoded signed transaction
    fn sign_and_encode(&self, signing_key: &SigningKey, chain_id: u64) -> (Vec<u8>, u8) {
        let hash = self.signing_hash();

        // Sign with recovery
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&hash)
            .expect("Signing should never fail with valid key");

        let sig_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig_bytes[..32]);
        s.copy_from_slice(&sig_bytes[32..]);

        // EIP-155: v = chain_id * 2 + 35 + recovery_id
        let v = chain_id * 2 + 35 + recovery_id.to_byte() as u64;

        // RLP encode: [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
        let mut buf = BytesMut::new();
        self.nonce.encode(&mut buf);
        self.gas_price.encode(&mut buf);
        self.gas_limit.encode(&mut buf);
        if self.to == [0u8; 20] {
            let empty: &[u8] = &[];
            empty.encode(&mut buf);
        } else {
            self.to.as_ref().encode(&mut buf);
        }
        self.value.encode(&mut buf);
        self.data.encode(&mut buf);
        v.encode(&mut buf);
        r.as_ref().encode(&mut buf);
        s.as_ref().encode(&mut buf);

        (buf.to_vec(), recovery_id.to_byte())
    }
}

/// Metrics for load testing
#[derive(Debug, Default)]
#[allow(dead_code)]
struct Metrics {
    submitted_count: AtomicU64,
    success_count: AtomicU64,
    error_count: AtomicU64,
    latencies: RwLock<Vec<Duration>>,
    start_block: AtomicU64,
    end_block: AtomicU64,
}

impl Metrics {
    #[allow(dead_code)]
    fn new() -> Self {
        Self {
            submitted_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            latencies: RwLock::new(Vec::new()),
            start_block: AtomicU64::new(0),
            end_block: AtomicU64::new(0),
        }
    }

    #[allow(dead_code)]
    fn reset(&self) {
        self.submitted_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.error_count.store(0, Ordering::SeqCst);
        self.start_block.store(0, Ordering::SeqCst);
        self.end_block.store(0, Ordering::SeqCst);
    }
}

/// CLI arguments
#[derive(Clone)]
struct Config {
    rpc_url: String,
    target_tps: u64,
    duration: u64,
    accounts: usize,
    value: u128,
    chain_id: u64,
    ramp_up: bool,
    json_output: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            rpc_url: "http://127.0.0.1:8546".to_string(),
            target_tps: 100,
            duration: 60,
            accounts: 10,
            value: 1_000_000_000_000_000, // 0.001 ETH
            chain_id 11567,
            ramp_up: false,
            json_output: false,
        }
    }
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let mut config = Config::default();
    let mut skip_next = false;

    for (idx, arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg == "--rpc-url" && idx + 1 < args.len() {
            config.rpc_url = args[idx + 1].clone();
            skip_next = true;
        } else if arg == "--target-tps" && idx + 1 < args.len() {
            config.target_tps = args[idx + 1].parse().unwrap_or(100);
            skip_next = true;
        } else if arg == "--duration" && idx + 1 < args.len() {
            config.duration = args[idx + 1].parse().unwrap_or(60);
            skip_next = true;
        } else if arg == "--accounts" && idx + 1 < args.len() {
            config.accounts = args[idx + 1].parse().unwrap_or(10);
            skip_next = true;
        } else if arg == "--value" && idx + 1 < args.len() {
            config.value = args[idx + 1].parse().unwrap_or(1_000_000_000_000_000);
            skip_next = true;
        } else if arg == "--chain-id" && idx + 1 < args.len() {
            config.chain_id 11567);
            skip_next = true;
        } else if arg == "--ramp-up" {
            config.ramp_up = true;
        } else if arg == "--json" {
            config.json_output = true;
        }
    }

    config
}

/// Send JSON-RPC request
async fn send_rpc_request(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(rpc_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {}", e))?;

    Ok(json)
}

/// Get current block number
async fn get_block_number(client: &reqwest::Client, rpc_url: &str) -> Result<u64, String> {
    let result =
        send_rpc_request(client, rpc_url, "eth_blockNumber", serde_json::json!([])).await?;

    let hex = result["result"]
        .as_str()
        .ok_or("Missing block number result")?;

    u64::from_str_radix(hex.trim_start_matches("0x"), 16).map_err(|e| format!("Parse error: {}", e))
}

/// Send raw transaction
async fn send_raw_transaction(
    client: &reqwest::Client,
    rpc_url: &str,
    raw_tx: &[u8],
) -> Result<String, String> {
    let hex_tx = format!("0x{}", hex::encode(raw_tx));
    let result = send_rpc_request(
        client,
        rpc_url,
        "eth_sendRawTransaction",
        serde_json::json!([hex_tx]),
    )
    .await?;

    if let Some(error) = result.get("error") {
        return Err(format!("RPC error: {}", error));
    }

    result["result"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or("Missing transaction hash".to_string())
}

/// Calculate percentile from sorted durations
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::from_millis(0);
    }
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Print results table
fn print_results_table(
    phases: &[(String, u64, f64, u64)],
    latencies: &[Duration],
    total_txs: u64,
    blocks_produced: u64,
) {
    info!("");
    info!("LOAD TEST RESULTS");
    info!("Phase      | Target TPS | Actual TPS | Errors");

    for (phase, target_tps, actual_tps, errors) in phases {
        info!(
            "{:<10} | {:<10} | {:<10.1} | {:<18}",
            phase, target_tps, actual_tps, errors
        );
    }

    // Calculate latency percentiles
    let mut sorted_latencies: Vec<Duration> = latencies.to_vec();
    sorted_latencies.sort();

    let p50 = percentile(&sorted_latencies, 50.0);
    let p95 = percentile(&sorted_latencies, 95.0);
    let p99 = percentile(&sorted_latencies, 99.0);

    let success_rate = if total_txs > 0 {
        let success_count = phases.iter().map(|(_, _, _, e)| *e as f64).sum::<f64>();
        let total_attempts = phases.iter().map(|(_, _, _, e)| *e).sum::<u64>();
        // success_rate = (total - errors) / total * 100
        100.0 - (success_count / total_attempts as f64 * 100.0)
    } else {
        100.0
    };

    let avg_block_size = total_txs.checked_div(blocks_produced).unwrap_or(0);

    info!(
        "Latency p50: {}ms  p95: {}ms  p99: {}ms",
        p50.as_millis(),
        p95.as_millis(),
        p99.as_millis()
    );
    info!(
        "Total Txs: {:<6}  Success Rate: {:.1}%",
        total_txs, success_rate
    );
    info!(
        "Blocks Produced: {:<4}  Avg Block Size: {:<6} txs",
        blocks_produced, avg_block_size
    );
}

/// Print JSON results
fn print_json_results(
    phases: &[(String, u64, f64, u64)],
    latencies: &[Duration],
    total_txs: u64,
    blocks_produced: u64,
) {
    let mut sorted_latencies: Vec<Duration> = latencies.to_vec();
    sorted_latencies.sort();

    let p50 = percentile(&sorted_latencies, 50.0);
    let p95 = percentile(&sorted_latencies, 95.0);
    let p99 = percentile(&sorted_latencies, 99.0);

    let phase_results: Vec<serde_json::Value> = phases
        .iter()
        .map(|(phase, target_tps, actual_tps, errors)| {
            serde_json::json!({
                "phase": phase,
                "target_tps": target_tps,
                "actual_tps": actual_tps,
                "errors": errors
            })
        })
        .collect();

    let total_errors: u64 = phases.iter().map(|(_, _, _, e)| *e).sum();
    let success_rate = if total_txs > 0 {
        100.0 * (total_txs - total_errors) as f64 / total_txs as f64
    } else {
        100.0
    };

    let result = serde_json::json!({
        "phases": phase_results,
        "latency": {
            "p50_ms": p50.as_millis(),
            "p95_ms": p95.as_millis(),
            "p99_ms": p99.as_millis()
        },
        "total_transactions": total_txs,
        "success_rate": success_rate,
        "blocks_produced": blocks_produced,
        "avg_block_size": total_txs.checked_div(blocks_produced).unwrap_or(0)
    });

    // For JSON output, we still use println since it's the structured output format
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("irondag=info".parse().unwrap()),
        )
        .init();

    let config = parse_args();

    info!("IronDAG Load Test - TPS Benchmark");
    info!("Configuration:");
    info!("RPC URL:    {}", config.rpc_url);
    info!("Target TPS: {}", config.target_tps);
    info!("Duration:   {}s", config.duration);
    info!("Accounts:   {}", config.accounts);
    info!("Value:      {} wei", config.value);
    info!("Chain ID:   {}", config.chain_id);
    info!("Ramp-up:    {}", config.ramp_up);

    // Create HTTP client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to create HTTP client");

    // Check RPC connectivity
    info!("Testing RPC connection...");
    match get_block_number(&client, &config.rpc_url).await {
        Ok(block) => info!("Connected. Current block: {}", block),
        Err(e) => {
            error!("Failed to connect to RPC: {}", e);
            std::process::exit(1);
        }
    }

    // Generate accounts
    info!("Generating {} accounts...", config.accounts);
    let mut rng = rand::thread_rng();
    let mut accounts: Vec<Account> = (0..config.accounts)
        .map(|_| Account::new(&mut rng))
        .collect();

    for (i, account) in accounts.iter().enumerate().take(5) {
        info!("Account {}: 0x{}", i, hex::encode(account.address));
    }
    if accounts.len() > 5 {
        info!("... and {} more", accounts.len() - 5);
    }

    // Fund accounts (send initial balance from faucet/coinbase)
    info!("Funding accounts (via node's faucet)...");
    let _coinbase: [u8; 20] = [1u8; 20]; // Default miner address

    // First, check if we can use the first account's address for funding
    // In a real scenario, the node should have funded these accounts
    // For now, we assume the node is running with test mode enabled
    // or accounts have been pre-funded

    info!("Note: Ensure accounts have sufficient balance before running load test");
    info!("You may need to manually fund test accounts or use --test-txs on node");

    // Initialize metrics
    let metrics = Arc::new(Metrics::new());

    // Define phases
    let phases: Vec<(&str, u64, u64)> = if config.ramp_up {
        vec![
            ("Ramp 100", 100, 30),
            ("Ramp 500", 500, 30),
            ("Sustained", config.target_tps, config.duration),
        ]
    } else {
        vec![("Sustained", config.target_tps, config.duration)]
    };

    // Track results per phase
    let mut phase_results: Vec<(String, u64, f64, u64)> = Vec::new();
    let mut all_latencies: Vec<Duration> = Vec::new();
    let mut total_txs = 0u64;

    // Get starting block
    let start_block = get_block_number(&client, &config.rpc_url)
        .await
        .unwrap_or(0);
    metrics.start_block.store(start_block, Ordering::SeqCst);

    info!("Starting load test...");

    // Run each phase
    for (phase_name, target_tps, duration_secs) in &phases {
        info!(
            "Phase: {} (Target: {} TPS, Duration: {}s)",
            phase_name, target_tps, duration_secs
        );

        let phase_metrics = Arc::new(Metrics::new());
        let phase_start = Instant::now();
        let phase_duration = Duration::from_secs(*duration_secs);

        // Spawn block monitor
        let monitor_client = client.clone();
        let monitor_rpc_url = config.rpc_url.clone();
        let monitor_metrics = phase_metrics.clone();
        let monitor_handle = tokio::spawn(async move {
            let mut last_block = get_block_number(&monitor_client, &monitor_rpc_url)
                .await
                .unwrap_or(0);
            monitor_metrics
                .start_block
                .store(last_block, Ordering::SeqCst);

            loop {
                sleep(Duration::from_secs(2)).await;
                if let Ok(block) = get_block_number(&monitor_client, &monitor_rpc_url).await {
                    if block > last_block {
                        info!(
                            "Block {} produced ({} txs submitted)",
                            block,
                            monitor_metrics.submitted_count.load(Ordering::SeqCst)
                        );
                        last_block = block;
                    }
                }
            }
        });

        // Spawn transaction sender
        let sender_client = client.clone();
        let sender_rpc_url = config.rpc_url.clone();
        let sender_metrics = phase_metrics.clone();
        let sender_accounts = Arc::new(RwLock::new(accounts.clone()));
        let sender_accounts_clone = sender_accounts.clone();
        let sender_config = config.clone();
        let sender_target_tps = *target_tps;
        let sender_duration = phase_duration;

        let sender_handle = tokio::spawn(async move {
            let interval_duration = Duration::from_micros(1_000_000 / sender_target_tps);
            let mut interval_timer = interval(interval_duration);
            let start = Instant::now();

            let mut account_idx = 0usize;
            let gas_price = 1_000_000_000u128; // 1 Gwei
            let gas_limit = 21000u64;

            while start.elapsed() < sender_duration {
                interval_timer.tick().await;

                // Get accounts length and recipient info first (before mutable borrow)
                let accounts_len = sender_accounts.read().await.len();
                let sender_idx = account_idx % accounts_len;
                let recipient_idx = (account_idx + 1) % accounts_len;
                account_idx += 1;

                // Get recipient address
                let recipient = sender_accounts.read().await[recipient_idx].address;

                // Get sender account info and increment nonce
                let (signing_key, nonce) = {
                    let mut accounts_guard = sender_accounts.write().await;
                    let account = &mut accounts_guard[sender_idx];
                    let key = account.signing_key.clone();
                    let n = account.nonce;
                    account.nonce += 1;
                    (key, n)
                };

                // Create unsigned transaction
                let tx = UnsignedTransaction {
                    nonce,
                    gas_price,
                    gas_limit,
                    to: recipient,
                    value: sender_config.value,
                    data: vec![],
                    chain_id: sender_config.chain_id,
                };

                // Sign and encode
                let (raw_tx, _) = tx.sign_and_encode(&signing_key, sender_config.chain_id);

                // Send transaction
                sender_metrics
                    .submitted_count
                    .fetch_add(1, Ordering::SeqCst);
                let submit_start = Instant::now();

                match send_raw_transaction(&sender_client, &sender_rpc_url, &raw_tx).await {
                    Ok(_hash) => {
                        sender_metrics.success_count.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => {
                        sender_metrics.error_count.fetch_add(1, Ordering::SeqCst);
                        if sender_metrics.error_count.load(Ordering::SeqCst) < 10 {
                            error!("Error: {}", e);
                        }
                    }
                }

                let latency = submit_start.elapsed();
                sender_metrics.latencies.write().await.push(latency);
            }
        });

        // Wait for phase to complete
        sender_handle.await.expect("Sender task failed");
        monitor_handle.abort(); // Stop block monitor

        // Collect phase metrics
        let phase_elapsed = phase_start.elapsed();
        let phase_submitted = phase_metrics.submitted_count.load(Ordering::SeqCst);
        let phase_success = phase_metrics.success_count.load(Ordering::SeqCst);
        let phase_errors = phase_metrics.error_count.load(Ordering::SeqCst);
        let actual_tps = phase_success as f64 / phase_elapsed.as_secs_f64();

        let mut phase_latencies = phase_metrics.latencies.write().await.clone();
        all_latencies.append(&mut phase_latencies);

        phase_results.push((
            phase_name.to_string(),
            *target_tps,
            actual_tps,
            phase_errors,
        ));
        total_txs += phase_success;

        info!(
            "Completed: {} txs submitted, {} errors, {:.1} actual TPS",
            phase_submitted, phase_errors, actual_tps
        );

        // Update accounts for next phase
        accounts = sender_accounts_clone.read().await.clone();
    }

    // Get final block
    let end_block = get_block_number(&client, &config.rpc_url)
        .await
        .unwrap_or(0);
    let blocks_produced = end_block.saturating_sub(start_block);

    // Print results
    if config.json_output {
        print_json_results(&phase_results, &all_latencies, total_txs, blocks_produced);
    } else {
        print_results_table(&phase_results, &all_latencies, total_txs, blocks_produced);
    }
}
