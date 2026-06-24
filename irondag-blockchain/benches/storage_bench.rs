//! Storage-level benchmarks using Criterion
//!
//! Benchmarks:
//! 1. Block write throughput (3 concurrent streams)
//! 2. Hot cache hit rate
//! 3. Checkpoint save/load
//! 4. Concurrent read/write contention

use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use irondag_blockchain::blockchain::Block;
use irondag_blockchain::blockchain::BlockHeader;
use irondag_blockchain::blockchain::Transaction;
use irondag_blockchain::consensus::storage::DagStorageConfig;
use irondag_blockchain::consensus::storage::HybridDagStorage;
use irondag_blockchain::storage::BlockStore;
use irondag_blockchain::storage::Database;
use irondag_blockchain::types::Address;
use irondag_blockchain::types::Hash;
use irondag_blockchain::types::StreamType;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

// ============================================================================
// MOCK DATA GENERATORS
// ============================================================================

/// Generate a deterministic hash from a seed
fn generate_hash(seed: u64) -> Hash {
    let mut hash = [0u8; 32];
    let bytes = seed.to_le_bytes();
    for i in 0..32 {
        hash[i] = bytes[i % 8].wrapping_add(i as u8);
    }
    Hash(hash)
}

/// Generate a deterministic address from a seed
fn generate_address(seed: u64) -> Address {
    let mut addr = [0u8; 20];
    let bytes = seed.to_le_bytes();
    for i in 0..20 {
        addr[i] = bytes[i % 8].wrapping_add(i as u8);
    }
    Address(addr)
}

/// Create a mock transaction
fn create_mock_transaction(seed: u64) -> Transaction {
    Transaction::new(
        generate_address(seed),
        generate_address(seed + 1),
        1000,
        100,
        seed,
    )
}

/// Create a mock block with specified size characteristics
fn create_mock_block(
    block_number: u64,
    stream_type: StreamType,
    parent_hashes: Vec<Hash>,
    tx_count: usize,
) -> Block {
    let header = BlockHeader::new(
        parent_hashes,
        block_number,
        stream_type,
        1000,
        1_000_000_000,
    );

    let transactions: Vec<Transaction> = (0..tx_count)
        .map(|i| create_mock_transaction(block_number * 1000 + i as u64))
        .collect();

    Block::new(header, transactions)
}

/// Create a large block (simulating Stream A - low frequency, large blocks)
fn create_large_block(block_number: u64, parent_hash: Hash) -> Block {
    create_mock_block(block_number, StreamType::StreamA, vec![parent_hash], 100)
}

/// Create a medium block (simulating Stream B - medium frequency)
fn create_medium_block(block_number: u64, parent_hash: Hash) -> Block {
    create_mock_block(block_number, StreamType::StreamB, vec![parent_hash], 10)
}

/// Create a small block (simulating Stream C - high frequency, small blocks)
fn create_small_block(block_number: u64, parent_hash: Hash) -> Block {
    create_mock_block(block_number, StreamType::StreamC, vec![parent_hash], 1)
}

// ============================================================================
// BENCHMARK 1: Block Write Throughput
// ============================================================================

fn benchmark_block_write_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_write_throughput");

    // Configure for measurement
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(10);

    group.bench_function("3_concurrent_streams", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());
            let block_store = BlockStore::new(&db);

            let start = Instant::now();
            let mut total_writes = 0u64;

            // Simulate 3 concurrent streams writing at their target rates
            // For benchmark purposes, we write a fixed number of blocks per iteration
            for _ in 0..iters {
                // Stream A: 1 block every 10s (low frequency, large blocks)
                // In benchmark: write 1 large block
                let block_a = create_large_block(total_writes, generate_hash(total_writes));
                block_store.put(&block_a).unwrap();
                total_writes += 1;

                // Stream B: 1 block every 1s (medium frequency)
                // In benchmark: write 10 medium blocks
                for i in 0..10 {
                    let block_b =
                        create_medium_block(total_writes + i, generate_hash(total_writes + i));
                    block_store.put(&block_b).unwrap();
                }
                total_writes += 10;

                // Stream C: 1 block every 100ms (high frequency, small blocks)
                // In benchmark: write 100 small blocks
                for i in 0..100 {
                    let block_c =
                        create_small_block(total_writes + i, generate_hash(total_writes + i));
                    block_store.put(&block_c).unwrap();
                }
                total_writes += 100;
            }

            let elapsed = start.elapsed();

            // Report throughput
            black_box(total_writes);
            elapsed
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 2: Hot Cache Hit Rate
// ============================================================================

fn benchmark_hot_cache_hit_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_cache_hit_rate");

    group.measurement_time(Duration::from_secs(5));

    // Benchmark hot cache hits (recent 500 blocks)
    group.bench_function("hot_cache_hits", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());

            let config = DagStorageConfig {
                hot_cache_size: 1000,
                finalized_depth: 500,
                confirmations_for_checkpoint: 100,
            };

            let mut storage = HybridDagStorage::with_database(db, config);

            // Insert 2000 blocks
            let mut hashes = Vec::with_capacity(2000);
            for i in 0..2000 {
                let block = create_medium_block(i as u64, generate_hash(i as u64));
                hashes.push(block.hash);
                storage.add_block(&block).unwrap();
            }

            // Measure reading the most recent 500 (should be hot cache hits)
            let recent_hashes: Vec<Hash> = hashes[1500..2000].to_vec();

            let start = Instant::now();
            for _ in 0..iters {
                for hash in &recent_hashes {
                    black_box(storage.get_block(hash).unwrap());
                }
            }
            start.elapsed()
        });
    });

    // Benchmark disk reads (oldest 500 blocks)
    group.bench_function("disk_reads", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());

            let config = DagStorageConfig {
                hot_cache_size: 1000,
                finalized_depth: 500,
                confirmations_for_checkpoint: 100,
            };

            let mut storage = HybridDagStorage::with_database(db, config);

            // Insert 2000 blocks
            let mut hashes = Vec::with_capacity(2000);
            for i in 0..2000 {
                let block = create_medium_block(i as u64, generate_hash(i as u64));
                hashes.push(block.hash);
                storage.add_block(&block).unwrap();
            }

            // Measure reading blocks 1-500 (should be disk reads)
            let old_hashes: Vec<Hash> = hashes[0..500].to_vec();

            let start = Instant::now();
            for _ in 0..iters {
                for hash in &old_hashes {
                    black_box(storage.get_block(hash).unwrap());
                }
            }
            start.elapsed()
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 3: Checkpoint Save/Load
// ============================================================================

fn benchmark_checkpoint_save_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint");

    group.measurement_time(Duration::from_secs(5));

    // Benchmark checkpoint save
    group.bench_function("save_checkpoint", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());

            let config = DagStorageConfig {
                hot_cache_size: 1000,
                finalized_depth: 500,
                confirmations_for_checkpoint: 100,
            };

            let storage = HybridDagStorage::with_database(db, config);

            // Prepare checkpoint data (1000 blue scores, blue set, ordering)
            let genesis_hash = generate_hash(0);
            let mut blue_set = HashSet::new();
            let mut blue_scores = HashMap::new();
            let mut ordering = Vec::new();

            for i in 0..1000 {
                let hash = generate_hash(i);
                blue_set.insert(hash);
                blue_scores.insert(hash, i);
                ordering.push(hash);
            }

            let start = Instant::now();
            for _ in 0..iters {
                storage
                    .save_checkpoint(Some(genesis_hash), &blue_set, &blue_scores, &ordering)
                    .unwrap();
            }
            start.elapsed()
        });
    });

    // Benchmark checkpoint load
    group.bench_function("load_checkpoint", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());

            let config = DagStorageConfig {
                hot_cache_size: 1000,
                finalized_depth: 500,
                confirmations_for_checkpoint: 100,
            };

            let storage = HybridDagStorage::with_database(db, config);

            // Prepare and save checkpoint data first
            let genesis_hash = generate_hash(0);
            let mut blue_set = HashSet::new();
            let mut blue_scores = HashMap::new();
            let mut ordering = Vec::new();

            for i in 0..1000 {
                let hash = generate_hash(i);
                blue_set.insert(hash);
                blue_scores.insert(hash, i);
                ordering.push(hash);
            }

            storage
                .save_checkpoint(Some(genesis_hash), &blue_set, &blue_scores, &ordering)
                .unwrap();

            let start = Instant::now();
            for _ in 0..iters {
                black_box(storage.load_checkpoint().unwrap());
            }
            start.elapsed()
        });
    });

    group.finish();
}

// ============================================================================
// BENCHMARK 4: Concurrent Read/Write Contention
// ============================================================================

fn benchmark_concurrent_read_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_read_write");

    group.measurement_time(Duration::from_secs(10));
    group.sample_size(10);

    group.bench_function("contention", |b| {
        b.iter_custom(|iters| {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("bench_db");
            let db = Arc::new(Database::open(&db_path).unwrap());
            #[allow(clippy::arc_with_non_send_sync)]
            let block_store = Arc::new(BlockStore::new(&db));

            // Pre-populate with some blocks for reading
            let mut hashes = Vec::with_capacity(100);
            for i in 0..100 {
                let block = create_medium_block(i, generate_hash(i));
                hashes.push(block.hash);
                block_store.put(&block).unwrap();
            }

            let start = Instant::now();

            for _ in 0..iters {
                let writer_db = Arc::clone(&db);
                let reader_db = Arc::clone(&db);
                let read_hashes = hashes.clone();

                // Spawn writer thread
                let writer_handle = thread::spawn(move || {
                    let store = BlockStore::new(&writer_db);
                    let mut writes = 0;
                    for i in 100..200 {
                        let block = create_medium_block(i as u64, generate_hash(i as u64));
                        if store.put(&block).is_ok() {
                            writes += 1;
                        }
                    }
                    writes
                });

                // Spawn reader thread
                let reader_handle = thread::spawn(move || {
                    let store = BlockStore::new(&reader_db);
                    let mut reads = 0;
                    for hash in &read_hashes {
                        if store.get(hash).is_ok() {
                            reads += 1;
                        }
                    }
                    reads
                });

                // Wait for both threads to complete
                let writes = writer_handle.join().unwrap();
                let reads = reader_handle.join().unwrap();

                black_box((writes, reads));
            }

            start.elapsed()
        });
    });

    group.finish();
}

// ============================================================================
// CRITERION GROUPS
// ============================================================================

criterion_group!(
    benches,
    benchmark_block_write_throughput,
    benchmark_hot_cache_hit_rate,
    benchmark_checkpoint_save_load,
    benchmark_concurrent_read_write
);

criterion_main!(benches);
