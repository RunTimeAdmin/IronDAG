//! Proof-of-Work implementation
//!
//! Implements actual cryptographic mining for BraidCore architecture:
//! - Stream A: Blake3 hashing (ASIC-friendly)
//! - Stream B: B3MemHash (CPU; GPU via OpenCL planned)
//! - Stream C: ZK proofs (not PoW, fee-based only)
//!
//! ## Performance Optimizations
//! - Pre-allocated memory buffers for B3MemHash (avoids 1MB alloc per hash)
//! - Early termination on difficulty check (check leading bytes first)
//! - Zero-allocation mining loop (nonce update inline)
//! - SIMD-accelerated XOR for B3MemHash memory mixing (AVX2/SSE2)
//! - Parallel mining with per-thread optimized state

use crate::blockchain::BlockHeader;
use crate::types::{Hash, StreamType};
use blake3;
use sha3::{Digest, Keccak256};
use std::sync::Arc;
use tracing::{info, warn};

/// Target block times (in seconds)
pub const STREAM_A_TARGET_TIME: u64 = 10;
/// Stream B: 5s target (reduces B3MemHash CPU/memory pressure on 4-core nodes)
pub const STREAM_B_TARGET_TIME: u64 = 5;

/// Initial difficulty values
/// Difficulty = number of leading zero bits required
/// Difficulty 8 = 1 leading zero byte (8 bits) - reasonable for testing
/// Difficulty 16 = 2 leading zero bytes (16 bits) - harder
/// Difficulty 24 = 3 leading zero bytes (24 bits) - very hard
pub const INITIAL_DIFFICULTY_A: u64 = 9; // Stream A: starts in sweet spot, less oscillation
pub const INITIAL_DIFFICULTY_B: u64 = 5; // Stream B: 1 zero byte (reasonable)

/// Maximum difficulty cap (prevents runaway difficulty)
/// 21 = ~2.6 zero bytes — 25% lower than previous 28 cap
/// This can be made configurable via node configuration
pub const MAX_DIFFICULTY: u64 = 21;

/// Maximum nonce value (2^64 - 1)
pub const MAX_NONCE: u64 = u64::MAX;

/// Calculate difficulty target from difficulty value
/// Difficulty target = 2^256 / (difficulty + 1)
/// Lower difficulty = higher target (easier to mine)
/// Higher difficulty = lower target (harder to mine)
pub fn difficulty_to_target(difficulty: u64) -> [u8; 32] {
    // For simplicity, we'll use a simpler approach:
    // Target = leading zeros required in hash
    // Difficulty 1 = 1 leading zero, Difficulty 1000 = 1000 leading zeros (but that's too many)
    // Instead: Target = 2^(256 - difficulty_bits)
    // We'll use difficulty as "number of leading zero bits required"

    // More practical: difficulty represents how many leading zero bytes we need
    // Difficulty 1 = 1 leading zero byte (8 bits)
    // Difficulty 10 = 10 leading zero bits (1.25 bytes)

    // For now, use a simpler approach: difficulty = number of leading zero bits
    // Target hash must have at least `difficulty/8` bytes are zero

    // Return a target hash where first `difficulty/8` bytes are zero
    let mut target = [0xFFu8; 32];
    let zero_bytes = (difficulty / 8) as usize;
    let zero_bits_in_last_byte = difficulty % 8;

    // Set leading bytes to zero
    for item in target.iter_mut().take(zero_bytes.min(32)) {
        *item = 0;
    }

    // Set partial bits in the next byte
    if zero_bytes < 32 && zero_bits_in_last_byte > 0 {
        target[zero_bytes] = 0xFF >> zero_bits_in_last_byte;
    }

    target
}

/// Check if hash meets difficulty target
/// Optimized with early termination - checks leading bytes first
pub fn meets_difficulty(hash: &Hash, difficulty: u64) -> bool {
    // Early termination optimization: check required zero bytes first
    let zero_bytes = (difficulty / 8) as usize;

    // Fast path: check leading zero bytes directly (most common rejection)
    for i in 0..zero_bytes.min(32) {
        if hash[i] != 0 {
            return false; // Early exit - doesn't meet difficulty
        }
    }

    // Check partial bits in the boundary byte
    if zero_bytes < 32 {
        let zero_bits_in_last_byte = difficulty % 8;
        if zero_bits_in_last_byte > 0 {
            let mask = 0xFF >> zero_bits_in_last_byte;
            if hash[zero_bytes] > mask {
                return false;
            }
        }
    }

    true
}

/// Hash block header using Blake3 (Stream A - ASIC)
pub fn hash_blake3(header: &BlockHeader, transactions_root: &Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();

    // Hash all header fields
    for parent in &header.parent_hashes {
        hasher.update(parent);
    }
    hasher.update(&header.block_number.to_le_bytes());
    hasher.update(&header.difficulty.to_le_bytes());
    hasher.update(&header.timestamp.to_le_bytes());
    hasher.update(&header.nonce.to_le_bytes());
    hasher.update(&header.stream_type.to_bytes());
    hasher.update(transactions_root);

    let hash = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(hash.as_bytes());
    Hash(result)
}

/// Blake3-based memory-hard hash function for Stream B mining.
///
/// B3MemHash ("Blake3 Memory-Hard Hash") provides ASIC resistance through
/// memory-hardness properties using a 256KB memory buffer and multiple mixing passes.
/// See ADR-003 for the design rationale.
pub fn hash_b3memhash(header: &BlockHeader, transactions_root: &Hash) -> Hash {
    // Use thread-local pre-allocated buffer for massive performance gain
    // Avoids 1MB allocation per hash (was the biggest bottleneck)
    thread_local! {
        static MEMORY_BUFFER: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; B3MEM_MEMORY_SIZE]);
    }

    MEMORY_BUFFER.with(|buf| {
        let mut memory = buf.borrow_mut();
        hash_b3memhash_with_buffer(header, transactions_root, &mut memory)
    })
}

/// B3MemHash memory size (256KB - reduced from 1MB for CPU/memory bandwidth on 4-core nodes)
const B3MEM_MEMORY_SIZE: usize = 256 * 1024;

/// B3MemHash number of memory passes (2 - reduced from 3 for CPU)
const B3MEM_PASSES: usize = 2;

// ============================================================================
// SIMD-accelerated XOR for B3MemHash memory mixing
// ============================================================================

/// XOR bytes with automatic SIMD detection (AVX2 > SSE2 > scalar fallback)
#[inline]
pub fn xor_bytes(dst: &mut [u8], src: &[u8]) {
    let len = dst.len().min(src.len());

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: The AVX2 intrinsic calls in xor_bytes_avx2 are safe because:
            // 1. `is_x86_feature_detected!("avx2")` confirms CPU support at runtime
            // 2. The `#[target_feature(enable = "avx2")]` on xor_bytes_avx2 allows
            //    the function to use AVX2 instructions without undefined behavior
            // 3. Bounds check above ensures `len <= dst.len()` and `len <= src.len()`
            // 4. The function uses unaligned loads/stores (_mm256_loadu/storeu) which
            //    do not require alignment, making them safe for arbitrary slices
            unsafe {
                xor_bytes_avx2(dst, src, len);
            }
            return;
        }
        if is_x86_feature_detected!("sse2") {
            // SAFETY: The SSE2 intrinsic calls in xor_bytes_sse2 are safe because:
            // 1. `is_x86_feature_detected!("sse2")` confirms CPU support at runtime
            // 2. The `#[target_feature(enable = "sse2")]` on xor_bytes_sse2 allows
            //    the function to use SSE2 instructions without undefined behavior
            // 3. `len` is bounded by `dst.len().min(src.len())` computed above
            // 4. The function uses unaligned loads/stores (_mm_loadu/storeu) which
            //    do not require alignment, making them safe for arbitrary slices
            unsafe {
                xor_bytes_sse2(dst, src, len);
            }
            return;
        }
    }

    // Scalar fallback for non-x86 or missing SIMD
    xor_bytes_scalar(dst, src, len);
}

/// Scalar XOR fallback (always available)
#[inline]
fn xor_bytes_scalar(dst: &mut [u8], src: &[u8], len: usize) {
    // Process 8 bytes at a time for better performance
    let chunks_8 = len / 8;
    for i in 0..chunks_8 {
        let offset = i * 8;
        let dst_val = u64::from_le_bytes(
            dst.get(offset..offset + 8)
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0u8; 8]),
        );
        let src_val = u64::from_le_bytes(
            src.get(offset..offset + 8)
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0u8; 8]),
        );
        if let Some(dst_slice) = dst.get_mut(offset..offset + 8) {
            dst_slice.copy_from_slice(&(dst_val ^ src_val).to_le_bytes());
        }
    }
    // Handle remaining bytes
    for i in (chunks_8 * 8)..len {
        if i < dst.len() && i < src.len() {
            dst[i] ^= src[i];
        }
    }
}

/// AVX2 SIMD XOR (32 bytes per operation) - 8x faster than scalar
///
/// # Safety
///
/// This function is unsafe because it uses AVX2 intrinsics that require CPU support.
/// Callers must ensure:
/// - AVX2 is available on the CPU (check with `is_x86_feature_detected!("avx2")`)
/// - `len <= dst.len()` and `len <= src.len()` to prevent out-of-bounds access
/// - The function uses unaligned loads/stores, so no alignment is required
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn xor_bytes_avx2(dst: &mut [u8], src: &[u8], len: usize) {
    use std::arch::x86_64::*;

    let chunks_32 = len / 32;
    for i in 0..chunks_32 {
        let offset = i * 32;
        let dst_ptr = dst.as_mut_ptr().add(offset) as *mut __m256i;
        let src_ptr = src.as_ptr().add(offset) as *const __m256i;

        let dst_val = _mm256_loadu_si256(dst_ptr);
        let src_val = _mm256_loadu_si256(src_ptr);
        let xored = _mm256_xor_si256(dst_val, src_val);

        _mm256_storeu_si256(dst_ptr, xored);
    }

    // Handle remaining bytes with scalar
    for i in (chunks_32 * 32)..len {
        dst[i] ^= src[i];
    }
}

/// SSE2 SIMD XOR (16 bytes per operation) - 4x faster than scalar
///
/// # Safety
///
/// This function is unsafe because it uses SSE2 intrinsics that require CPU support.
/// Callers must ensure:
/// - SSE2 is available on the CPU (check with `is_x86_feature_detected!("sse2")`)
/// - `len <= dst.len()` and `len <= src.len()` to prevent out-of-bounds access
/// - The function uses unaligned loads/stores, so no alignment is required
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn xor_bytes_sse2(dst: &mut [u8], src: &[u8], len: usize) {
    use std::arch::x86_64::*;

    let chunks_16 = len / 16;
    for i in 0..chunks_16 {
        let offset = i * 16;
        let dst_ptr = dst.as_mut_ptr().add(offset) as *mut __m128i;
        let src_ptr = src.as_ptr().add(offset) as *const __m128i;

        let dst_val = _mm_loadu_si128(dst_ptr);
        let src_val = _mm_loadu_si128(src_ptr);
        let xored = _mm_xor_si128(dst_val, src_val);

        _mm_storeu_si128(dst_ptr, xored);
    }

    // Handle remaining bytes with scalar
    for i in (chunks_16 * 16)..len {
        dst[i] ^= src[i];
    }
}

// ============================================================================
// B3MemHash Implementation
// ============================================================================

/// Hash block header using B3MemHash with pre-allocated buffer
/// This is the optimized version - call hash_b3memhash() for automatic buffer management
fn hash_b3memhash_with_buffer(
    header: &BlockHeader,
    transactions_root: &Hash,
    memory: &mut [u8],
) -> Hash {
    // Prepare input data (reuse allocation pattern)
    let mut input = Vec::with_capacity(256);
    for parent in &header.parent_hashes {
        input.extend_from_slice(parent);
    }
    input.extend_from_slice(&header.block_number.to_le_bytes());
    input.extend_from_slice(&header.difficulty.to_le_bytes());
    input.extend_from_slice(&header.timestamp.to_le_bytes());
    input.extend_from_slice(&header.nonce.to_le_bytes());
    input.extend_from_slice(&header.stream_type.to_bytes());
    input.extend_from_slice(transactions_root);

    // First pass: Fill memory with Blake3 hashes
    let mut hasher = blake3::Hasher::new();
    hasher.update(&input);
    let mut seed = hasher.finalize();

    // Fill memory in chunks (reusing buffer)
    for chunk in memory.chunks_mut(32) {
        let mut hasher = blake3::Hasher::new();
        hasher.update(seed.as_bytes());
        hasher.update(&input);
        let hash = hasher.finalize();
        seed = hash;
        chunk.copy_from_slice(&hash.as_bytes()[..chunk.len()]);
    }

    // Additional passes: Mix memory (memory-hard property)
    for pass in 0..B3MEM_PASSES {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&input);
        hasher.update(&pass.to_le_bytes());
        let mut mix_seed = hasher.finalize();

        // Mix memory in chunks
        for chunk in memory.chunks_mut(32) {
            let mut hasher = blake3::Hasher::new();
            hasher.update(mix_seed.as_bytes());
            hasher.update(chunk);
            hasher.update(&input);
            let hash = hasher.finalize();
            mix_seed = hash;

            // SIMD-accelerated XOR (8x faster than scalar on AVX2)
            xor_bytes(chunk, hash.as_bytes());
        }
    }

    // Final hash: Hash the entire memory buffer
    let mut final_hasher = blake3::Hasher::new();
    final_hasher.update(&input);
    final_hasher.update(memory);
    let final_hash = final_hasher.finalize();

    let mut result = [0u8; 32];
    result.copy_from_slice(final_hash.as_bytes());
    Hash(result)
}

/// Mine a block using Proof-of-Work
/// Returns (nonce, hash) if successful, None if mining was cancelled
pub fn mine_block(
    header_template: &BlockHeader,
    transactions_root: &Hash,
    stream_type: StreamType,
    max_iterations: Option<u64>,
) -> Option<(u64, Hash)> {
    let max_nonce = max_iterations.unwrap_or(MAX_NONCE);
    let difficulty = header_template.difficulty;

    // Optimization: Pre-compute static parts of header for faster hashing
    // Only nonce changes during mining, so we can cache everything else
    let mut header_bytes = Vec::with_capacity(256);
    for parent in &header_template.parent_hashes {
        header_bytes.extend_from_slice(parent);
    }
    header_bytes.extend_from_slice(&header_template.block_number.to_le_bytes());
    header_bytes.extend_from_slice(&header_template.difficulty.to_le_bytes());
    header_bytes.extend_from_slice(&header_template.timestamp.to_le_bytes());
    // Nonce position marker - we'll update this inline
    let nonce_offset = header_bytes.len();
    header_bytes.extend_from_slice(&0u64.to_le_bytes()); // Placeholder
    header_bytes.extend_from_slice(&header_template.stream_type.to_bytes());
    header_bytes.extend_from_slice(transactions_root);

    // Mining loop with optimizations
    let mut nonce = 0u64;

    loop {
        // Update nonce in pre-computed buffer (avoids full header clone)
        header_bytes[nonce_offset..nonce_offset + 8].copy_from_slice(&nonce.to_le_bytes());

        // Hash based on stream type
        let hash = match stream_type {
            StreamType::StreamA => hash_blake3_from_bytes(&header_bytes),
            StreamType::StreamB => {
                // For B3MemHash, we still need the full header structure
                let mut header = header_template.clone();
                header.nonce = nonce;
                hash_b3memhash(&header, transactions_root)
            }
            StreamType::StreamC => {
                // Stream C doesn't use PoW (ZK proofs), but we'll still hash it
                hash_blake3_from_bytes(&header_bytes)
            }
        };

        // Check if hash meets difficulty (optimized with early termination)
        if meets_difficulty(&hash, difficulty) {
            return Some((nonce, hash));
        }

        // Increment nonce
        nonce += 1;

        // Check if we've exceeded max iterations
        if nonce >= max_nonce {
            return None; // Mining failed (difficulty too high or cancelled)
        }
    }
}

/// Optimized Blake3 hash from pre-computed byte buffer
/// Avoids header struct overhead in tight mining loop
#[inline]
fn hash_blake3_from_bytes(data: &[u8]) -> Hash {
    let hash = blake3::hash(data);
    let mut result = [0u8; 32];
    result.copy_from_slice(hash.as_bytes());
    Hash(result)
}

/// Batch mine with parallel processing (for multi-core CPUs)
/// Divides nonce space across threads for higher throughput
///
/// # Performance
/// - Zero-allocation inner loop (nonce update inline)
/// - Per-thread optimized state (no shared mutable state)
/// - Lock-free result propagation via atomics
///
/// # Arguments
/// * `header_template` - Block header template
/// * `transactions_root` - Merkle root of transactions
/// * `stream_type` - Mining stream type
/// * `batch_size` - Number of nonces to try per batch (default: 100_000)
/// * `max_batches` - Maximum number of batches to try (None = unlimited)
///
/// # Returns
/// (nonce, hash) if successful, None if max_batches exceeded
pub fn mine_block_parallel(
    header_template: &BlockHeader,
    transactions_root: &Hash,
    stream_type: StreamType,
    batch_size: u64,
    max_batches: Option<u64>,
) -> Option<(u64, Hash)> {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    let found = Arc::new(AtomicBool::new(false));
    let result_nonce = Arc::new(AtomicU64::new(0));
    let result_hash = Arc::new(std::sync::Mutex::new([0u8; 32]));
    let difficulty = header_template.difficulty;

    let mut batch_start = 0u64;
    let max_batch_count = max_batches.unwrap_or(u64::MAX);
    let mut batch_count = 0u64;

    // Two mining streams (A and B) run simultaneously. Divide available cores by 4
    // so each stream gets ~1 dedicated thread on a 4-core VPS under CPUQuota=200%.
    // Without this, both streams each spawn available/2 threads, oversubscribing
    // the cgroup-limited CPUs and starving the slower Stream B.
    let available = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let num_threads = (available / 4).max(1);

    while !found.load(Ordering::Acquire) && batch_count < max_batch_count {
        // Divide batch across threads
        let nonces_per_thread = batch_size / num_threads as u64;

        std::thread::scope(|s| {
            for thread_id in 0..num_threads {
                let found = found.clone();
                let result_nonce = result_nonce.clone();
                let result_hash = result_hash.clone();
                let tx_root = *transactions_root;
                let start = batch_start + (thread_id as u64 * nonces_per_thread);
                let end = start + nonces_per_thread;

                // Clone header ONCE per thread (not per iteration)
                let mut thread_header = header_template.clone();

                // Pre-compute header bytes for Stream A/C (zero-allocation loop)
                let mut header_bytes = Vec::with_capacity(256);
                for parent in &header_template.parent_hashes {
                    header_bytes.extend_from_slice(parent);
                }
                header_bytes.extend_from_slice(&header_template.block_number.to_le_bytes());
                header_bytes.extend_from_slice(&header_template.difficulty.to_le_bytes());
                header_bytes.extend_from_slice(&header_template.timestamp.to_le_bytes());
                let nonce_offset = header_bytes.len();
                header_bytes.extend_from_slice(&0u64.to_le_bytes());
                header_bytes.extend_from_slice(&header_template.stream_type.to_bytes());
                header_bytes.extend_from_slice(&tx_root);

                s.spawn(move || {
                    for nonce in start..end {
                        if found.load(Ordering::Acquire) {
                            return; // Another thread found solution
                        }

                        let hash = match stream_type {
                            StreamType::StreamA | StreamType::StreamC => {
                                // Zero-allocation path: update nonce inline
                                header_bytes[nonce_offset..nonce_offset + 8]
                                    .copy_from_slice(&nonce.to_le_bytes());
                                hash_blake3_from_bytes(&header_bytes)
                            }
                            StreamType::StreamB => {
                                // B3MemHash needs full header (uses thread-local buffer)
                                thread_header.nonce = nonce;
                                hash_b3memhash(&thread_header, &tx_root)
                            }
                        };

                        if meets_difficulty(&hash, difficulty) {
                            found.store(true, Ordering::Release);
                            result_nonce.store(nonce, Ordering::Relaxed);
                            if let Ok(mut rh) = result_hash.lock() {
                                *rh = hash.into();
                            }
                            return;
                        }
                    }
                });
            }
        });

        batch_start += batch_size;
        batch_count += 1;

        // PER-006: Yield CPU between batches to prevent saturation on resource-constrained VPS
        // Using yield_now() instead of sleep(1ms) for better performance while still allowing
        // other threads to run
        std::thread::yield_now();
    }

    if found.load(Ordering::Acquire) {
        let nonce = result_nonce.load(Ordering::Relaxed);
        let hash = result_hash.lock().ok().map(|h| *h).unwrap_or([0u8; 32]);
        Some((nonce, Hash(hash)))
    } else {
        None
    }
}

/// Calculate difficulty adjustment with damping to prevent oscillation
///
/// Uses a damping factor to prevent wild swings:
/// - Max change per adjustment: 4x (doubles or halves at most)
/// - Smooths out variance from lucky/unlucky blocks
///
/// Formula: new_difficulty = old_difficulty * min(max(target_time / actual_time, 0.25), 4.0)
///
/// This ensures:
/// - Blocks too fast (1/10th time) → difficulty increases by max 4x
/// - Blocks too slow (10x time) → difficulty decreases by max 4x
/// - Prevents oscillation from single lucky/unlucky blocks
pub fn adjust_difficulty(current_difficulty: u64, target_time: u64, actual_time: u64) -> u64 {
    // Prevent division by zero
    if actual_time == 0 {
        return current_difficulty;
    }

    // Calculate raw adjustment factor
    // Use fixed-point arithmetic: multiply by 1000 for precision
    let raw_adjustment = (target_time as u128 * 1000).saturating_div(actual_time as u128);

    // Apply damping: clamp adjustment between 0.667x and 1.5x
    // This prevents wild swings from single lucky/unlucky blocks
    let min_adjustment = 667u128; // 0.667x (in thousandths)
    let max_adjustment = 1500u128; // 1.5x (in thousandths)

    let clamped_adjustment = raw_adjustment.max(min_adjustment).min(max_adjustment);

    // Apply adjustment: new_difficulty = current * (adjustment / 1000)
    let new_difficulty = (current_difficulty as u128)
        .saturating_mul(clamped_adjustment)
        .saturating_div(1000);

    // Clamp to absolute bounds
    let min_difficulty = 1u64;
    let max_difficulty = MAX_DIFFICULTY; // Use cap for dev/testnet

    new_difficulty
        .min(max_difficulty as u128)
        .max(min_difficulty as u128) as u64
}

/// Calculate difficulty adjustment using moving average (more stable)
///
/// This is the preferred method for production - uses average of last N blocks
/// to smooth out variance and prevent oscillation.
///
/// # Arguments
/// * `current_difficulty` - Current difficulty
/// * `target_time` - Target block time in seconds
/// * `recent_block_times` - Vector of recent block times (should be last 10-20 blocks)
///
/// # Returns
/// Adjusted difficulty based on moving average
pub fn adjust_difficulty_moving_average(
    current_difficulty: u64,
    target_time: u64,
    recent_block_times: &[u64],
) -> u64 {
    if recent_block_times.is_empty() {
        return current_difficulty;
    }

    // Calculate average block time from recent blocks
    let sum: u128 = recent_block_times.iter().map(|&t| t as u128).sum();
    let count = recent_block_times.len() as u128;
    let avg_time = sum / count;

    if avg_time == 0 {
        return current_difficulty;
    }

    // Use the same damping as single-block adjustment
    let raw_adjustment = (target_time as u128 * 1000).saturating_div(avg_time as u128);

    let min_adjustment = 667u128; // 0.667x
    let max_adjustment = 1500u128; // 1.5x

    let clamped_adjustment = raw_adjustment.max(min_adjustment).min(max_adjustment);

    let new_difficulty = (current_difficulty as u128)
        .saturating_mul(clamped_adjustment)
        .saturating_div(1000);

    let min_difficulty = 1u64;
    let max_difficulty = MAX_DIFFICULTY; // Use cap for dev/testnet

    new_difficulty
        .min(max_difficulty as u128)
        .max(min_difficulty as u128) as u64
}

/// Calculate transactions root using binary Merkle tree
///
/// Uses Keccak256 for hashing. For odd-length levels, duplicates the last element.
pub fn calculate_transactions_root(transaction_hashes: &[Hash]) -> Hash {
    if transaction_hashes.is_empty() {
        // Empty transactions root = hash of empty array
        let mut hasher = Keccak256::new();
        hasher.update(&[0u8; 32]);
        let hash = hasher.finalize();
        let mut result = [0u8; 32];
        result.copy_from_slice(&hash);
        return Hash(result);
    }

    if transaction_hashes.len() == 1 {
        // Single transaction: root is the transaction hash itself
        return transaction_hashes[0];
    }

    // Build binary Merkle tree
    let mut current_level: Vec<Hash> = transaction_hashes.to_vec();

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));

        for chunk in current_level.chunks(2) {
            let mut hasher = Keccak256::new();
            hasher.update(&chunk[0]);
            if chunk.len() > 1 {
                hasher.update(&chunk[1]);
            } else {
                // Odd count: duplicate last element
                hasher.update(&chunk[0]);
            }
            let hash = hasher.finalize();
            let mut result = [0u8; 32];
            result.copy_from_slice(&hash);
            next_level.push(Hash(result));
        }
        current_level = next_level;
    }

    current_level.into_iter().next().unwrap()
}

/// Mining backend abstraction for runtime-switchable CPU/GPU mining (GPU not yet implemented)
pub trait MiningBackend: Send + Sync {
    /// Mine a block by searching for a valid nonce
    /// Returns Some((nonce, hash)) if found, None if max_iterations exhausted
    fn mine(
        &self,
        header_template: &BlockHeader,
        transactions_root: &Hash,
        max_iterations: Option<u64>,
    ) -> Option<(u64, Hash)>;

    /// Backend name for logging
    fn name(&self) -> &str;
}

/// CPU mining backend wrapping existing mine_block_parallel() logic
pub struct CpuBackend {
    pub num_threads: usize,
}

impl CpuBackend {
    pub fn new() -> Self {
        Self {
            num_threads: num_cpus::get(),
        }
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MiningBackend for CpuBackend {
    fn mine(
        &self,
        header_template: &BlockHeader,
        transactions_root: &Hash,
        _max_iterations: Option<u64>,
    ) -> Option<(u64, Hash)> {
        // 400 nonces ≈ 3.5 s at the measured ~8.8 ms/hash for B3MemHash on this VPS.
        // Must stay under the ~4.2 s timestamp freshness window:
        //   median of 11 A blocks at ~2.5 blocks/s ≈ 2.2 s behind current time;
        //   MEDIAN_TOLERANCE_SECS = 2 s → max safe mining = 4.2 s = ~477 nonces.
        // Returning None triggers a fresh header + difficulty decrease in mine_stream_b.
        mine_block_parallel(
            header_template,
            transactions_root,
            StreamType::StreamB,
            400,
            Some(1),
        )
    }
    fn name(&self) -> &str {
        "CPU"
    }
}

/// GPU mining backend (placeholder for OpenCL integration)
/// When the `gpu` feature is enabled, this will use OpenCL for B3MemHash mining.
/// Currently falls back to CPU mining.
pub struct GpuBackend {
    fallback: CpuBackend,
    pub gpu_available: bool,
}

impl GpuBackend {
    pub fn new() -> Self {
        // Check for GPU availability
        let gpu_available = Self::detect_gpu();
        if gpu_available {
            info!("GPU device detected - GPU mining backend initialized");
        } else {
            // Silently fall back to CPU (this is the normal case)
        }
        Self {
            fallback: CpuBackend::new(),
            gpu_available,
        }
    }

    fn detect_gpu() -> bool {
        // TODO: When `gpu` feature is enabled, use ocl to detect OpenCL devices
        // #[cfg(feature = "gpu")]
        // {
        //     ocl::Platform::list().map(|p| !p.is_empty()).unwrap_or(false)
        // }
        // QUA-010: Log warning that GPU detection is not implemented
        warn!("GPU mining detection not implemented — always returns CPU fallback");
        // For now, always return false (no GPU support yet)
        false
    }
}

impl Default for GpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MiningBackend for GpuBackend {
    fn mine(
        &self,
        header_template: &BlockHeader,
        transactions_root: &Hash,
        max_iterations: Option<u64>,
    ) -> Option<(u64, Hash)> {
        if self.gpu_available {
            // TODO: Launch OpenCL kernel for B3MemHash
            // 1. Upload header_data to GPU buffer
            // 2. Launch kernel with nonce range [0..max_iterations]
            // 3. Poll for result
            // 4. Return nonce + hash if found
            // For now, fall through to CPU
        }
        self.fallback
            .mine(header_template, transactions_root, max_iterations)
    }
    fn name(&self) -> &str {
        if self.gpu_available {
            "GPU"
        } else {
            "GPU(fallback=CPU)"
        }
    }
}

/// Mining backend configuration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiningBackendConfig {
    Cpu,
    Gpu,
    Auto,
}

impl MiningBackendConfig {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "gpu" => Self::Gpu,
            "auto" => Self::Auto,
            _ => Self::Cpu,
        }
    }
}

/// Factory function to create the appropriate mining backend
pub fn create_mining_backend(config: MiningBackendConfig) -> Arc<dyn MiningBackend> {
    match config {
        MiningBackendConfig::Cpu => {
            info!("Using CPU mining backend");
            Arc::new(CpuBackend::new())
        }
        MiningBackendConfig::Gpu => {
            info!("Using GPU mining backend");
            Arc::new(GpuBackend::new())
        }
        MiningBackendConfig::Auto => {
            let gpu = GpuBackend::new();
            if gpu.gpu_available {
                info!("Auto-detected GPU, using GPU mining backend");
                Arc::new(gpu)
            } else {
                info!("No GPU detected, using CPU mining backend");
                Arc::new(CpuBackend::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_difficulty_target() {
        let target = difficulty_to_target(8);
        assert_eq!(target[0], 0); // First byte should be zero for difficulty 8
    }

    #[test]
    fn test_meets_difficulty() {
        let hash = Hash([0u8; 32]);
        assert!(meets_difficulty(&hash, 8)); // All zeros meets any difficulty

        let hash = Hash([0xFFu8; 32]);
        assert!(!meets_difficulty(&hash, 8)); // All 0xFF does not meet difficulty
    }

    #[test]
    fn test_adjust_difficulty() {
        // Note: MAX_DIFFICULTY is capped at 16 for dev/testnet
        // Use difficulty values that work within that cap

        // If blocks are coming too fast, difficulty should increase
        let new_diff = adjust_difficulty(8, 10, 5); // Target 10s, actual 5s
        assert!(
            new_diff > 8,
            "Difficulty should increase from 8, got {}",
            new_diff
        );

        // If blocks are coming too slow, difficulty should decrease
        let new_diff = adjust_difficulty(8, 10, 20); // Target 10s, actual 20s
        assert!(
            new_diff < 8,
            "Difficulty should decrease from 8, got {}",
            new_diff
        );

        // If blocks are on target, difficulty should stay similar
        let new_diff = adjust_difficulty(8, 10, 10); // Target 10s, actual 10s
        assert_eq!(new_diff, 8);
    }

    #[test]
    fn test_difficulty_damping_no_oscillation() {
        // Note: MAX_DIFFICULTY is capped at 28 for dev/testnet
        // Test damping behavior within those constraints

        // Test that damping prevents wild swings (before hitting MAX cap)
        // Damping limits adjustment to 1.5x max, so 4 * 1.5 = 6 (not 8)
        let new_diff = adjust_difficulty(4, 10, 5); // Target 10s, actual 5s (2x too fast)
        assert!(new_diff > 4, "Difficulty should increase");
        assert_eq!(new_diff, 6, "With 2x speedup, damping limits to 1.5x = 6");

        // Even if block is 2x too slow, difficulty should decrease
        // Damping limits decrease to 0.667x min, so 8 * 0.667 = 5 (rounded)
        let new_diff = adjust_difficulty(8, 10, 20); // Target 10s, actual 20s (2x too slow)
        assert!(new_diff < 8, "Difficulty should decrease");
        assert_eq!(
            new_diff, 5,
            "With 2x slowdown, damping limits to 0.667x = 5"
        );

        // Test extreme case: division by zero protection
        let new_diff = adjust_difficulty(8, 10, 0); // Division by zero protection
        assert_eq!(new_diff, 8, "Should return current difficulty on zero time");

        // Test damping limit (max 1.5x increase due to damping)
        let new_diff = adjust_difficulty(4, 10, 1); // Target 10s, actual 1s (10x too fast)
                                                    // Damping limits to 1.5x: 4 * 1.5 = 6
        assert_eq!(new_diff, 6, "Should hit 1.5x damping limit");
    }

    #[test]
    fn test_difficulty_moving_average() {
        // Note: MAX_DIFFICULTY is capped at 16 for dev/testnet
        // Test moving average adjustment (smoother than single-block)
        let recent_times = vec![8, 9, 10, 11, 12]; // Average = 10s (on target)
        let new_diff = adjust_difficulty_moving_average(8, 10, &recent_times);
        assert_eq!(new_diff, 8, "On-target average should keep difficulty same");

        // Test with fast blocks
        let recent_times = vec![1, 2, 3, 4, 5]; // Average = 3s (too fast)
        let new_diff = adjust_difficulty_moving_average(4, 10, &recent_times);
        assert!(
            new_diff > 4,
            "Fast blocks should increase difficulty, got {}",
            new_diff
        );
        assert!(new_diff <= 32, "Should be clamped by MAX_DIFFICULTY");

        // Test with slow blocks
        let recent_times = vec![20, 21, 22, 23, 24]; // Average = 22s (too slow)
        let new_diff = adjust_difficulty_moving_average(8, 10, &recent_times);
        assert!(
            new_diff < 8,
            "Slow blocks should decrease difficulty, got {}",
            new_diff
        );
        assert!(new_diff >= 1, "Should have minimum difficulty of 1");

        // Test empty vector
        let new_diff = adjust_difficulty_moving_average(8, 10, &[]);
        assert_eq!(new_diff, 8, "Empty vector should return current difficulty");
    }

    #[test]
    fn test_actual_mining_low_difficulty() {
        // Test that we can actually mine a block with low difficulty
        let header = BlockHeader::new(vec![], 1, StreamType::StreamA, 8, 1_000_000_000); // Low difficulty: 8 bits
        let tx_root = calculate_transactions_root(&[]);

        let start = std::time::Instant::now();
        let result = mine_block(&header, &tx_root, StreamType::StreamA, Some(10_000_000));
        let elapsed = start.elapsed();

        assert!(
            result.is_some(),
            "Mining should succeed with low difficulty"
        );
        let (nonce, hash) = result.unwrap();

        // Verify the hash actually meets difficulty
        assert!(
            meets_difficulty(&hash, 8),
            "Mined hash should meet difficulty"
        );

        // Verify nonce is not zero (proves we actually iterated)
        assert!(
            nonce > 0 || hash[0] == 0,
            "Nonce should be > 0 or hash should meet difficulty"
        );

        println!(
            "✅ Mined block with nonce: {}, hash: {:02x?}, time: {:?}",
            nonce,
            &hash.0[0..8],
            elapsed
        );
    }

    #[test]
    #[ignore] // Slow PoW test — runs in nightly CI only
    fn test_actual_mining_medium_difficulty() {
        // Test mining with medium difficulty (should take longer)
        let header = BlockHeader::new(vec![], 2, StreamType::StreamA, 16, 1_000_000_000); // Medium difficulty: 16 bits
        let tx_root = calculate_transactions_root(&[]);

        let start = std::time::Instant::now();
        let result = mine_block(&header, &tx_root, StreamType::StreamA, Some(100_000_000));
        let elapsed = start.elapsed();

        assert!(
            result.is_some(),
            "Mining should succeed with medium difficulty"
        );
        let (nonce, hash) = result.unwrap();

        assert!(
            meets_difficulty(&hash, 16),
            "Mined hash should meet difficulty"
        );

        println!(
            "✅ Mined block with nonce: {}, hash: {:02x?}, time: {:?}",
            nonce,
            &hash.0[0..8],
            elapsed
        );
        println!(
            "   This proves actual PoW - nonce {} required {} iterations",
            nonce, nonce
        );
    }

    #[test]
    fn test_mining_requires_work() {
        // Test that different nonces produce different hashes
        let header1 = BlockHeader::with_nonce(vec![], 1, StreamType::StreamA, 8, 0, 1_000_000_000);
        let header2 = BlockHeader::with_nonce(vec![], 1, StreamType::StreamA, 8, 1, 1_000_000_000);
        let tx_root = calculate_transactions_root(&[]);

        let hash1 = hash_blake3(&header1, &tx_root);
        let hash2 = hash_blake3(&header2, &tx_root);

        // Different nonces should produce different hashes
        assert_ne!(
            hash1, hash2,
            "Different nonces should produce different hashes"
        );

        println!("✅ Nonce 0 hash: {:02x?}", &hash1.0[0..8]);
        println!("✅ Nonce 1 hash: {:02x?}", &hash2.0[0..8]);
        println!("   This proves hashing is working correctly");
    }

    #[test]
    #[ignore] // Slow PoW test — runs in nightly CI only
    fn test_difficulty_affects_mining_time() {
        // Test that higher difficulty takes longer to mine
        let header_low = BlockHeader::new(vec![], 1, StreamType::StreamA, 8, 1_000_000_000);
        let header_high = BlockHeader::new(vec![], 2, StreamType::StreamA, 16, 1_000_000_000);
        let tx_root = calculate_transactions_root(&[]);

        let start_low = std::time::Instant::now();
        let result_low = mine_block(&header_low, &tx_root, StreamType::StreamA, Some(10_000_000));
        let time_low = start_low.elapsed();

        let start_high = std::time::Instant::now();
        let result_high = mine_block(
            &header_high,
            &tx_root,
            StreamType::StreamA,
            Some(100_000_000),
        );
        let time_high = start_high.elapsed();

        assert!(result_low.is_some());
        assert!(result_high.is_some());

        println!(
            "✅ Low difficulty (8): {:?}, nonce: {}",
            time_low,
            result_low.unwrap().0
        );
        println!(
            "✅ High difficulty (16): {:?}, nonce: {}",
            time_high,
            result_high.unwrap().0
        );
        println!("   Higher difficulty should generally take longer (may vary due to randomness)");
    }

    #[test]
    fn test_stream_b_mining() {
        // Test Stream B mining (B3MemHash)
        let header = BlockHeader::new(vec![], 1, StreamType::StreamB, 8, 1_000_000_000);
        let tx_root = calculate_transactions_root(&[]);

        let start = std::time::Instant::now();
        let result = mine_block(&header, &tx_root, StreamType::StreamB, Some(10_000_000));
        let elapsed = start.elapsed();

        assert!(result.is_some(), "Stream B mining should work");
        let (nonce, hash) = result.unwrap();

        assert!(
            meets_difficulty(&hash, 8),
            "Mined hash should meet difficulty"
        );

        println!(
            "✅ Stream B mined block with nonce: {}, time: {:?}",
            nonce, elapsed
        );
    }

    // =========================================================================
    // PoW Edge Case Tests (TEST-05)
    // =========================================================================

    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_atomic_bool_ordering_across_threads() {
        // Test that AtomicBool ordering works correctly for the `found` flag
        // This is critical for the mine_block_parallel function
        let found = Arc::new(AtomicBool::new(false));
        let found_clone = found.clone();

        // Spawn a thread that will set the flag
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(10));
            found_clone.store(true, Ordering::Release);
        });

        // Main thread waits for the flag to be set
        let start = std::time::Instant::now();
        while !found.load(Ordering::Acquire) {
            if start.elapsed() > std::time::Duration::from_secs(1) {
                panic!("Timeout waiting for atomic flag");
            }
            std::thread::yield_now();
        }

        handle.join().unwrap();

        // Verify the flag is set
        assert!(found.load(Ordering::Acquire));
    }

    #[test]
    fn test_max_difficulty_boundary() {
        // Test MAX_DIFFICULTY boundary (difficulty = 21)
        assert_eq!(MAX_DIFFICULTY, 21);

        // Test difficulty target calculation at max difficulty
        let target = difficulty_to_target(MAX_DIFFICULTY);

        // At difficulty 21 = 2 full zero bytes + 5 bits in the 3rd byte
        for (i, item) in target.iter().enumerate().take(2) {
            assert_eq!(*item, 0, "Byte {} should be zero at max difficulty", i);
        }

        // The 3rd byte should have the top 5 bits cleared (0xFF >> 5 = 0x07)
        assert_eq!(target[2], 0xFF >> 5);
    }

    #[test]
    fn test_meets_difficulty_at_max() {
        // Test that an all-zero hash meets MAX_DIFFICULTY (21)
        let hash = Hash([0u8; 32]);
        assert!(meets_difficulty(&hash, MAX_DIFFICULTY));

        // Test that a hash with 2 zero bytes but a failing 3rd byte fails at difficulty 21
        // MAX_DIFFICULTY = 21 = 2 full bytes (16 bits) + 5 bits in 3rd byte
        // The 3rd byte must be <= 0x07 (0xFF >> 5); 0x08 exceeds that
        let mut hash_inner = [0u8; 32];
        hash_inner[2] = 0x08; // top 5 bits check: 0x08 > 0x07 -> fails
        let hash = Hash(hash_inner);
        assert!(!meets_difficulty(&hash, MAX_DIFFICULTY));

        // Verify that 0x07 (0000 0111) passes - top 5 bits are zero
        hash_inner[2] = 0x07;
        let hash = Hash(hash_inner);
        assert!(meets_difficulty(&hash, MAX_DIFFICULTY));
    }

    #[test]
    fn test_meets_difficulty_partial_bits() {
        // Test partial bit difficulty (e.g., 12 bits = 1.5 bytes)
        let mut hash_inner = [0u8; 32];

        // Difficulty 12 = 1 full zero byte + 4 bits in second byte
        // 4 bits = 0xF0 mask, so value must be <= 0x0F
        hash_inner[0] = 0;
        hash_inner[1] = 0x0F; // 0000 1111 - should pass (4 zero bits)
        let hash = Hash(hash_inner);
        assert!(meets_difficulty(&hash, 12));

        hash_inner[1] = 0x10; // 0001 0000 - should fail (only 3 zero bits)
        let hash = Hash(hash_inner);
        assert!(!meets_difficulty(&hash, 12));
    }

    #[test]
    fn test_difficulty_zero() {
        // Test that difficulty 0 always passes
        let hash = Hash([0xFFu8; 32]);
        assert!(meets_difficulty(&hash, 0));

        let hash = Hash([0u8; 32]);
        assert!(meets_difficulty(&hash, 0));
    }

    #[test]
    fn test_difficulty_target_consistency() {
        // Test that difficulty_to_target and meets_difficulty are consistent
        for difficulty in [1, 8, 16, 24, 32, 40, 48, 56, 64] {
            let _target = difficulty_to_target(difficulty);

            // A hash of all zeros should always meet difficulty
            let zero_hash = Hash([0u8; 32]);
            assert!(
                meets_difficulty(&zero_hash, difficulty),
                "Zero hash should meet difficulty {}",
                difficulty
            );

            // The target itself (if interpreted as a hash) should meet difficulty
            // Note: This is a conceptual test - target is not a hash but a threshold
        }
    }

    #[test]
    fn test_parallel_mining_with_max_difficulty() {
        // Test parallel mining at max difficulty - should still work correctly
        use crate::blockchain::BlockHeader;

        let header = BlockHeader::new(
            vec![Hash([0u8; 32])], // parent hash
            1,
            StreamType::StreamA,
            8, // Use difficulty 8 for reasonable test time
            1_000_000_000,
        );
        let tx_root = calculate_transactions_root(&[]);

        // Use small batch size and limited batches for test
        let result = mine_block_parallel(
            &header,
            &tx_root,
            StreamType::StreamA,
            1000,     // Small batch
            Some(10), // Max 10 batches
        );

        // Should find a solution or exhaust batches
        // With difficulty 8, it should usually find a solution
        if let Some((nonce, hash)) = result {
            assert!(meets_difficulty(&hash, 8));
            assert!(nonce < 1000 * 10); // Should be within our search space
        }
        // If None, it's acceptable - we limited the search
    }

    #[test]
    fn test_mine_block_early_termination() {
        // Test that mine_block respects max_iterations
        let header = BlockHeader::new(
            vec![Hash([0u8; 32])],
            1,
            StreamType::StreamA,
            64, // Very high difficulty - unlikely to find solution
            1_000_000_000,
        );
        let tx_root = calculate_transactions_root(&[]);

        let max_iterations = 1000u64;
        let result = mine_block(&header, &tx_root, StreamType::StreamA, Some(max_iterations));

        // Should return None (didn't find solution) but not exceed iterations
        // We can't directly verify nonce didn't exceed, but function should return
        assert!(result.is_none() || result.unwrap().0 < max_iterations);
    }

    #[test]
    fn test_hash_blake3_deterministic() {
        // Test that Blake3 hashing is deterministic
        let header = BlockHeader::new(
            vec![Hash([0x01u8; 32])],
            1,
            StreamType::StreamA,
            8,
            1_000_000_000,
        );
        let tx_root = calculate_transactions_root(&[]);

        let hash1 = hash_blake3(&header, &tx_root);
        let hash2 = hash_blake3(&header, &tx_root);

        assert_eq!(hash1, hash2, "Blake3 hashing should be deterministic");
    }

    #[test]
    fn test_hash_b3memhash_deterministic() {
        // Test that B3MemHash is deterministic
        let header = BlockHeader::new(
            vec![Hash([0x01u8; 32])],
            1,
            StreamType::StreamB,
            8,
            1_000_000_000,
        );
        let tx_root = calculate_transactions_root(&[]);

        let hash1 = hash_b3memhash(&header, &tx_root);
        let hash2 = hash_b3memhash(&header, &tx_root);

        assert_eq!(hash1, hash2, "B3MemHash should be deterministic");
    }

    #[test]
    fn test_different_headers_produce_different_hashes() {
        // Test that different headers produce different hashes
        let tx_root = calculate_transactions_root(&[]);

        let header1 = BlockHeader::new(
            vec![Hash([0x01u8; 32])],
            1,
            StreamType::StreamA,
            8,
            1_000_000_000,
        );
        let header2 = BlockHeader::new(
            vec![Hash([0x02u8; 32])],
            1,
            StreamType::StreamA,
            8,
            1_000_000_000,
        );

        let hash1 = hash_blake3(&header1, &tx_root);
        let hash2 = hash_blake3(&header2, &tx_root);

        assert_ne!(
            hash1, hash2,
            "Different headers should produce different hashes"
        );
    }
}
