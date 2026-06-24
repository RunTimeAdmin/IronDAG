//! Metrics collection for production monitoring
//!
//! Provides Prometheus metrics for monitoring blockchain operations,
//! including blocks, transactions, network, mining, sharding, and RPC metrics.

use prometheus::{
    Counter, CounterVec, Encoder, Gauge, GaugeVec, Histogram, HistogramOpts, HistogramVec, Opts,
    Registry, TextEncoder,
};
use std::sync::Arc;
use std::sync::Mutex;

/// Metrics collector for the blockchain
pub struct Metrics {
    // Block metrics
    pub blocks_mined: Counter,
    pub blocks_received: Counter,
    pub block_size: Histogram,

    // Transaction metrics
    pub transactions_processed: Counter,
    pub transaction_pool_size: Gauge,
    pub transactions_per_second: Gauge,

    // Network metrics
    pub peers_connected: Gauge,
    pub peers_banned: Gauge,
    pub peer_latency_ms: GaugeVec,
    pub messages_sent: Counter,
    pub messages_received: Counter,

    // Mining metrics
    pub blocks_mined_stream_a: Counter,
    pub blocks_mined_stream_b: Counter,
    pub blocks_mined_stream_c: Counter,
    pub mining_rewards: Counter,
    pub mining_duration_ms: HistogramVec,

    // Mempool metrics (per-stream)
    pub mempool_size_total: Gauge,
    pub mempool_size_stream_a: Gauge,
    pub mempool_size_stream_b: Gauge,
    pub mempool_size_stream_c: Gauge,

    // Block validation metrics
    pub block_height: Gauge,
    pub block_validation_duration_ms: Histogram,

    // Fee metrics
    pub total_fees_burned: Counter,
    pub total_fees_collected: Counter,

    // Sharding metrics
    pub shard_transaction_count: Vec<Gauge>,
    pub cross_shard_transactions: Counter,

    // RPC metrics
    pub rpc_requests_total: CounterVec,
    pub rpc_request_duration: HistogramVec,
    pub rpc_errors_total: CounterVec,
    pub rpc_active_requests: Gauge,

    // Registry
    registry: Registry,
}

impl Metrics {
    /// Create a new metrics collector
    pub fn new(shard_count: usize) -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        // Block metrics
        let blocks_mined = Counter::with_opts(
            Opts::new("irondag_blocks_mined_total", "Total number of blocks mined")
                .namespace("irondag"),
        )?;

        let blocks_received = Counter::with_opts(
            Opts::new(
                "irondag_blocks_received_total",
                "Total number of blocks received from network",
            )
            .namespace("irondag"),
        )?;

        let block_size = Histogram::with_opts(
            HistogramOpts::new("irondag_block_size_bytes", "Block size in bytes")
                .namespace("irondag")
                .buckets(vec![1024.0, 10240.0, 102400.0, 1024000.0, 10240000.0]),
        )?;

        // Transaction metrics
        let transactions_processed = Counter::with_opts(
            Opts::new(
                "irondag_transactions_processed_total",
                "Total number of transactions processed",
            )
            .namespace("irondag"),
        )?;

        let transaction_pool_size = Gauge::with_opts(
            Opts::new(
                "irondag_transaction_pool_size",
                "Current transaction pool size",
            )
            .namespace("irondag"),
        )?;

        let transactions_per_second = Gauge::with_opts(
            Opts::new(
                "irondag_transactions_per_second",
                "Current transactions per second",
            )
            .namespace("irondag"),
        )?;

        // Network metrics
        let peers_connected = Gauge::with_opts(
            Opts::new("irondag_peers_connected", "Number of connected peers").namespace("irondag"),
        )?;

        let peers_banned = Gauge::with_opts(
            Opts::new("irondag_peers_banned", "Number of currently banned peers")
                .namespace("irondag"),
        )?;

        let peer_latency_ms = GaugeVec::new(
            Opts::new("irondag_peer_latency_ms", "Latency to peer in milliseconds")
                .namespace("irondag"),
            &["peer"],
        )?;

        let messages_sent = Counter::with_opts(
            Opts::new(
                "irondag_messages_sent_total",
                "Total number of messages sent",
            )
            .namespace("irondag"),
        )?;

        let messages_received = Counter::with_opts(
            Opts::new(
                "irondag_messages_received_total",
                "Total number of messages received",
            )
            .namespace("irondag"),
        )?;

        // Mining metrics
        let blocks_mined_stream_a = Counter::with_opts(
            Opts::new(
                "irondag_blocks_mined_stream_a_total",
                "Total blocks mined in Stream A",
            )
            .namespace("irondag"),
        )?;

        let blocks_mined_stream_b = Counter::with_opts(
            Opts::new(
                "irondag_blocks_mined_stream_b_total",
                "Total blocks mined in Stream B",
            )
            .namespace("irondag"),
        )?;

        let blocks_mined_stream_c = Counter::with_opts(
            Opts::new(
                "irondag_blocks_mined_stream_c_total",
                "Total blocks mined in Stream C",
            )
            .namespace("irondag"),
        )?;

        let mining_rewards = Counter::with_opts(
            Opts::new(
                "irondag_mining_rewards_total",
                "Total mining rewards earned (in smallest unit)",
            )
            .namespace("irondag"),
        )?;

        let mining_duration_ms = HistogramVec::new(
            HistogramOpts::new(
                "irondag_mining_duration_ms",
                "Block mining duration in milliseconds",
            )
            .namespace("irondag")
            .buckets(vec![
                10.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
            ]),
            &["stream"],
        )?;

        // Mempool metrics (per-stream)
        let mempool_size_total = Gauge::with_opts(
            Opts::new(
                "irondag_mempool_size_total",
                "Total transactions in mempool across all streams",
            )
            .namespace("irondag"),
        )?;

        let mempool_size_stream_a = Gauge::with_opts(
            Opts::new(
                "irondag_mempool_size_stream_a",
                "Transactions in Stream A mempool",
            )
            .namespace("irondag"),
        )?;

        let mempool_size_stream_b = Gauge::with_opts(
            Opts::new(
                "irondag_mempool_size_stream_b",
                "Transactions in Stream B mempool",
            )
            .namespace("irondag"),
        )?;

        let mempool_size_stream_c = Gauge::with_opts(
            Opts::new(
                "irondag_mempool_size_stream_c",
                "Transactions in Stream C mempool",
            )
            .namespace("irondag"),
        )?;

        // Block validation metrics
        let block_height = Gauge::with_opts(
            Opts::new("irondag_block_height", "Current block height").namespace("irondag"),
        )?;

        let block_validation_duration_ms = Histogram::with_opts(
            HistogramOpts::new(
                "irondag_block_validation_duration_ms",
                "Block validation duration in milliseconds",
            )
            .namespace("irondag")
            .buckets(vec![
                0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0,
            ]),
        )?;

        // Fee metrics
        let total_fees_burned = Counter::with_opts(
            Opts::new(
                "irondag_total_fees_burned",
                "Total transaction fees burned (in smallest unit)",
            )
            .namespace("irondag"),
        )?;

        let total_fees_collected = Counter::with_opts(
            Opts::new(
                "irondag_total_fees_collected",
                "Total transaction fees collected by miners (in smallest unit)",
            )
            .namespace("irondag"),
        )?;

        // Sharding metrics
        let mut shard_transaction_count = Vec::new();
        for i in 0..shard_count {
            let gauge = Gauge::with_opts(
                Opts::new(
                    "irondag_shard_transaction_count",
                    "Transaction count in shard",
                )
                .namespace("irondag")
                .const_label("shard_id", &i.to_string()),
            )?;
            shard_transaction_count.push(gauge);
        }

        let cross_shard_transactions = Counter::with_opts(
            Opts::new(
                "irondag_cross_shard_transactions_total",
                "Total cross-shard transactions",
            )
            .namespace("irondag"),
        )?;

        // RPC metrics
        let rpc_requests_total = CounterVec::new(
            Opts::new("irondag_rpc_requests_total", "Total RPC requests by method")
                .namespace("irondag"),
            &["method"],
        )?;

        let rpc_request_duration = HistogramVec::new(
            HistogramOpts::new(
                "irondag_rpc_request_duration_seconds",
                "RPC request duration",
            )
            .namespace("irondag")
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
            ]),
            &["method"],
        )?;

        let rpc_errors_total = CounterVec::new(
            Opts::new(
                "irondag_rpc_errors_total",
                "Total RPC errors by method and code",
            )
            .namespace("irondag"),
            &["method", "code"],
        )?;

        let rpc_active_requests = Gauge::with_opts(
            Opts::new(
                "irondag_rpc_active_requests",
                "Number of active RPC requests",
            )
            .namespace("irondag"),
        )?;

        // Register all metrics
        registry.register(Box::new(blocks_mined.clone()))?;
        registry.register(Box::new(blocks_received.clone()))?;
        registry.register(Box::new(block_size.clone()))?;
        registry.register(Box::new(transactions_processed.clone()))?;
        registry.register(Box::new(transaction_pool_size.clone()))?;
        registry.register(Box::new(transactions_per_second.clone()))?;
        registry.register(Box::new(peers_connected.clone()))?;
        registry.register(Box::new(peers_banned.clone()))?;
        registry.register(Box::new(peer_latency_ms.clone()))?;
        registry.register(Box::new(messages_sent.clone()))?;
        registry.register(Box::new(messages_received.clone()))?;
        registry.register(Box::new(blocks_mined_stream_a.clone()))?;
        registry.register(Box::new(blocks_mined_stream_b.clone()))?;
        registry.register(Box::new(blocks_mined_stream_c.clone()))?;
        registry.register(Box::new(mining_rewards.clone()))?;
        registry.register(Box::new(mining_duration_ms.clone()))?;
        registry.register(Box::new(mempool_size_total.clone()))?;
        registry.register(Box::new(mempool_size_stream_a.clone()))?;
        registry.register(Box::new(mempool_size_stream_b.clone()))?;
        registry.register(Box::new(mempool_size_stream_c.clone()))?;
        registry.register(Box::new(block_height.clone()))?;
        registry.register(Box::new(block_validation_duration_ms.clone()))?;
        registry.register(Box::new(total_fees_burned.clone()))?;
        registry.register(Box::new(total_fees_collected.clone()))?;

        for gauge in &shard_transaction_count {
            registry.register(Box::new(gauge.clone()))?;
        }

        registry.register(Box::new(cross_shard_transactions.clone()))?;
        registry.register(Box::new(rpc_requests_total.clone()))?;
        registry.register(Box::new(rpc_request_duration.clone()))?;
        registry.register(Box::new(rpc_errors_total.clone()))?;
        registry.register(Box::new(rpc_active_requests.clone()))?;

        Ok(Self {
            blocks_mined,
            blocks_received,
            block_size,
            transactions_processed,
            transaction_pool_size,
            transactions_per_second,
            peers_connected,
            peers_banned,
            peer_latency_ms,
            messages_sent,
            messages_received,
            blocks_mined_stream_a,
            blocks_mined_stream_b,
            blocks_mined_stream_c,
            mining_rewards,
            mining_duration_ms,
            mempool_size_total,
            mempool_size_stream_a,
            mempool_size_stream_b,
            mempool_size_stream_c,
            block_height,
            block_validation_duration_ms,
            total_fees_burned,
            total_fees_collected,
            shard_transaction_count,
            cross_shard_transactions,
            rpc_requests_total,
            rpc_request_duration,
            rpc_errors_total,
            rpc_active_requests,
            registry,
        })
    }

    /// Get metrics in Prometheus format
    pub fn gather(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8_lossy(&buffer).to_string())
    }

    /// Record a block being mined
    pub fn record_block_mined(&self, stream: &str, size: usize, reward: u128) {
        self.blocks_mined.inc();
        self.block_size.observe(size as f64);
        self.mining_rewards.inc_by(reward as f64);

        match stream {
            "A" => self.blocks_mined_stream_a.inc(),
            "B" => self.blocks_mined_stream_b.inc(),
            "C" => self.blocks_mined_stream_c.inc(),
            _ => {}
        }
    }

    /// Record a block being received
    pub fn record_block_received(&self, size: usize) {
        self.blocks_received.inc();
        self.block_size.observe(size as f64);
    }

    /// Record transactions processed
    pub fn record_transactions_processed(&self, count: usize) {
        self.transactions_processed.inc_by(count as f64);
    }

    /// Update transaction pool size
    pub fn update_transaction_pool_size(&self, size: usize) {
        self.transaction_pool_size.set(size as f64);
    }

    /// Update transactions per second
    pub fn update_transactions_per_second(&self, tps: f64) {
        self.transactions_per_second.set(tps);
    }

    /// Update peers connected
    pub fn update_peers_connected(&self, count: usize) {
        self.peers_connected.set(count as f64);
    }

    /// Record message sent
    pub fn record_message_sent(&self) {
        self.messages_sent.inc();
    }

    /// Record message received
    pub fn record_message_received(&self) {
        self.messages_received.inc();
    }

    /// Update shard transaction count
    pub fn update_shard_transaction_count(&self, shard_id: usize, count: usize) {
        if let Some(gauge) = self.shard_transaction_count.get(shard_id) {
            gauge.set(count as f64);
        }
    }

    /// Record cross-shard transaction
    pub fn record_cross_shard_transaction(&self) {
        self.cross_shard_transactions.inc();
    }

    // ========================================================================
    // RPC Metrics
    // ========================================================================

    /// Record RPC request start (increment active count)
    pub fn rpc_request_start(&self, method: &str) {
        self.rpc_requests_total.with_label_values(&[method]).inc();
        self.rpc_active_requests.inc();
    }

    /// Record RPC request completion (duration, decrement active)
    pub fn rpc_request_complete(&self, method: &str, duration_secs: f64) {
        self.rpc_request_duration
            .with_label_values(&[method])
            .observe(duration_secs);
        self.rpc_active_requests.dec();
    }

    /// Record RPC error
    pub fn rpc_request_error(&self, method: &str, error_code: i32) {
        self.rpc_errors_total
            .with_label_values(&[method, &error_code.to_string()])
            .inc();
    }

    // ========================================================================
    // Network Metrics (Extended)
    // ========================================================================

    /// Update number of banned peers
    pub fn update_peers_banned(&self, count: usize) {
        self.peers_banned.set(count as f64);
    }

    /// Update latency for a specific peer
    pub fn update_peer_latency(&self, peer_addr: &str, latency_ms: f64) {
        self.peer_latency_ms
            .with_label_values(&[peer_addr])
            .set(latency_ms);
    }

    // ========================================================================
    // Mempool Metrics
    // ========================================================================

    /// Update all mempool sizes at once
    pub fn update_mempool_sizes(
        &self,
        total: usize,
        stream_a: usize,
        stream_b: usize,
        stream_c: usize,
    ) {
        self.mempool_size_total.set(total as f64);
        self.mempool_size_stream_a.set(stream_a as f64);
        self.mempool_size_stream_b.set(stream_b as f64);
        self.mempool_size_stream_c.set(stream_c as f64);
    }

    // ========================================================================
    // Block Metrics
    // ========================================================================

    /// Update current block height
    pub fn update_block_height(&self, height: u64) {
        self.block_height.set(height as f64);
    }

    /// Record block validation duration in milliseconds
    pub fn record_block_validation_duration(&self, duration_ms: f64) {
        self.block_validation_duration_ms.observe(duration_ms);
    }

    /// Record mining duration for a stream in milliseconds
    pub fn record_mining_duration(&self, stream: &str, duration_ms: f64) {
        self.mining_duration_ms
            .with_label_values(&[stream])
            .observe(duration_ms);
    }

    // ========================================================================
    // Fee Metrics
    // ========================================================================

    /// Record fees burned (EIP-1559 style)
    pub fn record_fees_burned(&self, amount: u128) {
        self.total_fees_burned.inc_by(amount as f64);
    }

    /// Record fees collected by miners
    pub fn record_fees_collected(&self, amount: u128) {
        self.total_fees_collected.inc_by(amount as f64);
    }
}

/// Thread-safe metrics wrapper
pub type MetricsHandle = Arc<Mutex<Metrics>>;

/// Create a metrics handle
pub fn create_metrics(shard_count: usize) -> Result<MetricsHandle, prometheus::Error> {
    Ok(Arc::new(Mutex::new(Metrics::new(shard_count)?)))
}
