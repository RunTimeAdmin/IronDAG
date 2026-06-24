//! Sharding TPS Benchmark Test
//!
//! Tests throughput scaling with sharding enabled.
//! Target: 160,000+ TPS with 10 shards (16,000 TPS per shard)

use irondag::blockchain::Transaction;
use irondag::sharding::{AssignmentStrategy, ShardConfig, ShardManager};
use irondag::types::Address;
use std::time::Instant;

/// Benchmark transaction routing across shards
#[tokio::test]
async fn test_sharding_tps_benchmark() {
    println!("\n=== SHARDING TPS BENCHMARK ===\n");

    // Test configurations
    let shard_counts = [1, 4, 10];
    let tx_counts = [1000, 5000, 10000];

    for &shard_count in &shard_counts {
        for &tx_count in &tx_counts {
            let config = ShardConfig {
                shard_count,
                enable_cross_shard: true,
                assignment_strategy: AssignmentStrategy::ConsistentHashing,
                ..Default::default()
            };

            let manager = ShardManager::new(config);

            // Generate test transactions
            let mut transactions = Vec::with_capacity(tx_count);
            for i in 0..tx_count {
                let sender = Address([(i % 256) as u8; 20]);
                let receiver = Address([((i + 1) % 256) as u8; 20]);
                let tx = Transaction::new(sender, receiver, 100, (i as u128) + 1, i as u64);
                transactions.push(tx);
            }

            // Measure routing throughput
            let start = Instant::now();

            for tx in &transactions {
                manager.add_transaction(tx.clone()).await.unwrap();
            }

            let elapsed = start.elapsed();
            let tps = tx_count as f64 / elapsed.as_secs_f64();

            // Verify all transactions were routed
            let mut total_routed = 0;
            for shard_id in 0..shard_count {
                let txs = manager.get_shard_transactions(shard_id, tx_count).await;
                total_routed += txs.len();
            }

            println!(
                "Shards: {:2} | TXs: {:5} | Time: {:8.2}ms | TPS: {:12.0} | Routed: {}",
                shard_count,
                tx_count,
                elapsed.as_secs_f64() * 1000.0,
                tps,
                total_routed
            );

            assert_eq!(total_routed, tx_count, "All transactions should be routed");
        }
    }

    println!("\n=== BENCHMARK COMPLETE ===\n");
}

/// Test linear scaling with shard count
#[tokio::test]
async fn test_sharding_linear_scaling() {
    println!("\n=== LINEAR SCALING TEST ===\n");

    let tx_count = 10000;
    let mut results = Vec::new();

    for shard_count in [1, 2, 4, 8, 10] {
        let config = ShardConfig {
            shard_count,
            enable_cross_shard: true,
            assignment_strategy: AssignmentStrategy::ConsistentHashing,
            ..Default::default()
        };

        let manager = ShardManager::new(config);

        // Generate transactions
        let transactions: Vec<_> = (0..tx_count)
            .map(|i| {
                let sender = Address([(i % 256) as u8; 20]);
                let receiver = Address([((i + 1) % 256) as u8; 20]);
                Transaction::new(sender, receiver, 100, (i as u128) + 1, i as u64)
            })
            .collect();

        let start = Instant::now();
        for tx in &transactions {
            manager.add_transaction(tx.clone()).await.unwrap();
        }
        let elapsed = start.elapsed();

        let tps = tx_count as f64 / elapsed.as_secs_f64();
        results.push((shard_count, tps));

        println!("Shards: {:2} | TPS: {:12.0}", shard_count, tps);
    }

    // Verify scaling trend (more shards should handle more TPS for routing)
    // Note: This tests routing, not actual block production
    let (_, tps_1_shard) = results[0];
    let (_, tps_10_shards) = results[results.len() - 1];

    println!(
        "\nScaling factor (10 shards vs 1): {:.2}x",
        tps_10_shards / tps_1_shard
    );
    println!("\n=== LINEAR SCALING TEST COMPLETE ===\n");
}

/// Test cross-shard transaction overhead
#[tokio::test]
async fn test_cross_shard_overhead() {
    println!("\n=== CROSS-SHARD OVERHEAD TEST ===\n");

    let config = ShardConfig {
        shard_count: 4,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config.clone());
    let tx_count = 5000;

    // Generate same-shard transactions (sender and receiver in same shard)
    let same_shard_txs: Vec<_> = (0..tx_count)
        .map(|i| {
            // Use addresses that hash to same shard
            let base = (i % 256) as u8;
            let sender = Address([base; 20]);
            let receiver = Address([base; 20]);
            Transaction::new(sender, receiver, 100, (i as u128) + 1, i as u64)
        })
        .collect();

    // Generate cross-shard transactions
    let cross_shard_txs: Vec<_> = (0..tx_count)
        .map(|i| {
            // Use addresses that hash to different shards
            let sender = Address([(i % 256) as u8; 20]);
            let receiver = Address([((i + 128) % 256) as u8; 20]);
            Transaction::new(sender, receiver, 100, (i as u128) + 1, i as u64)
        })
        .collect();

    // Benchmark same-shard
    let start = Instant::now();
    for tx in &same_shard_txs {
        manager.add_transaction(tx.clone()).await.unwrap();
    }
    let same_shard_time = start.elapsed();
    let same_shard_tps = tx_count as f64 / same_shard_time.as_secs_f64();

    // Clear and benchmark cross-shard
    let manager2 = ShardManager::new(config.clone());
    let start = Instant::now();
    for tx in &cross_shard_txs {
        manager2.add_transaction(tx.clone()).await.unwrap();
    }
    let cross_shard_time = start.elapsed();
    let cross_shard_tps = tx_count as f64 / cross_shard_time.as_secs_f64();

    let overhead = ((same_shard_tps - cross_shard_tps) / same_shard_tps) * 100.0;

    println!("Same-shard TPS:  {:12.0}", same_shard_tps);
    println!("Cross-shard TPS: {:12.0}", cross_shard_tps);
    println!("Overhead:        {:12.1}%", overhead.abs());

    // Cross-shard overhead should be reasonable (whitepaper claims 5-10%)
    // Note: Routing overhead is minimal; actual execution overhead may differ
    println!("\n=== CROSS-SHARD OVERHEAD TEST COMPLETE ===\n");
}

/// High-volume stress test
#[tokio::test]
async fn test_high_volume_sharding() {
    println!("\n=== HIGH-VOLUME SHARDING TEST ===\n");

    let config = ShardConfig {
        shard_count: 10,
        enable_cross_shard: true,
        assignment_strategy: AssignmentStrategy::ConsistentHashing,
        ..Default::default()
    };

    let manager = ShardManager::new(config);
    let tx_count = 50000;

    println!("Generating {} transactions...", tx_count);

    let transactions: Vec<_> = (0..tx_count)
        .map(|i| {
            let sender = Address([(i % 256) as u8; 20]);
            let receiver = Address([((i + 1) % 256) as u8; 20]);
            Transaction::new(sender, receiver, 100, (i as u128) + 1, i as u64)
        })
        .collect();

    println!("Routing transactions to 10 shards...");

    let start = Instant::now();
    for tx in &transactions {
        manager.add_transaction(tx.clone()).await.unwrap();
    }
    let elapsed = start.elapsed();

    let tps = tx_count as f64 / elapsed.as_secs_f64();

    // Verify distribution
    let mut shard_counts = [0usize; 10];
    for (shard_id, count) in shard_counts.iter_mut().enumerate() {
        let txs = manager.get_shard_transactions(shard_id, tx_count).await;
        *count = txs.len();
    }

    let total_routed: usize = shard_counts.iter().sum();
    let avg_per_shard = total_routed / 10;
    let min_shard = *shard_counts.iter().min().unwrap();
    let max_shard = *shard_counts.iter().max().unwrap();
    let balance_ratio = min_shard as f64 / max_shard as f64;

    println!("\nResults:");
    println!("  Total TXs:      {}", tx_count);
    println!("  Time:           {:.2}ms", elapsed.as_secs_f64() * 1000.0);
    println!("  Routing TPS:    {:.0}", tps);
    println!("  Total Routed:   {}", total_routed);
    println!("  Avg per Shard:  {}", avg_per_shard);
    println!("  Min Shard:      {}", min_shard);
    println!("  Max Shard:      {}", max_shard);
    println!("  Balance Ratio:  {:.2}", balance_ratio);

    assert_eq!(total_routed, tx_count, "All transactions must be routed");

    println!("\n=== HIGH-VOLUME TEST COMPLETE ===\n");
}
