//! Rate limiting for RPC API
//!
//! Implements token bucket rate limiting (global and per-IP) to prevent abuse.

use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Rate limiter using token bucket algorithm
pub struct RateLimiter {
    /// Maximum number of tokens (requests)
    max_tokens: u32,
    /// Current number of tokens
    tokens: Arc<RwLock<u32>>,
    /// Token refill rate (tokens per second)
    refill_rate: f64,
    /// Last refill time
    last_refill: Arc<RwLock<Instant>>,
}

impl RateLimiter {
    /// Create a new rate limiter
    ///
    /// # Arguments
    /// * `max_tokens` - Maximum number of tokens (burst capacity)
    /// * `tokens_per_second` - Token refill rate
    pub fn new(max_tokens: u32, tokens_per_second: f64) -> Self {
        Self {
            max_tokens,
            tokens: Arc::new(RwLock::new(max_tokens)),
            refill_rate: tokens_per_second,
            last_refill: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Try to acquire a token (allow a request)
    ///
    /// Returns `true` if request is allowed, `false` if rate limited
    pub async fn try_acquire(&self) -> bool {
        let mut tokens = self.tokens.write().await;
        let mut last_refill = self.last_refill.write().await;

        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill);
        let tokens_to_add = (elapsed.as_secs_f64() * self.refill_rate) as u32;

        if tokens_to_add > 0 {
            *tokens = (*tokens + tokens_to_add).min(self.max_tokens);
            *last_refill = now;
        }

        // Try to consume a token
        if *tokens > 0 {
            *tokens -= 1;
            true
        } else {
            false
        }
    }

    /// Get current token count (for monitoring)
    pub async fn current_tokens(&self) -> u32 {
        *self.tokens.read().await
    }
}

/// Per-IP rate limiter: each client IP has its own token bucket.
/// Use when client IP is available (e.g. from HTTP/gRPC) to limit abuse per source.
pub struct PerIpRateLimiter {
    max_tokens: u32,
    refill_rate: f64,
    buckets: DashMap<IpAddr, (u32, Instant)>,
}

impl PerIpRateLimiter {
    pub fn new(max_tokens: u32, tokens_per_second: f64) -> Self {
        Self {
            max_tokens,
            refill_rate: tokens_per_second,
            buckets: DashMap::new(),
        }
    }

    /// Try to acquire a token for this IP. Returns true if allowed, false if rate limited.
    pub async fn try_acquire(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        if let Some(mut entry) = self.buckets.get_mut(&ip) {
            let (tokens, last_refill) = &mut *entry;
            let elapsed = now.duration_since(*last_refill);
            let tokens_to_add = (elapsed.as_secs_f64() * self.refill_rate) as u32;
            if tokens_to_add > 0 {
                *tokens = (*tokens + tokens_to_add).min(self.max_tokens);
                *last_refill = now;
            }
            if *tokens > 0 {
                *tokens -= 1;
                return true;
            }
            return false;
        }
        self.buckets.insert(ip, (self.max_tokens - 1, now));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;
    use tokio::time::sleep;

    // =========================================================================
    // Rate Limiter Tests (TEST-04)
    // =========================================================================

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(10, 1.0); // 10 tokens, 1 per second

        // Should allow initial requests
        for _ in 0..10 {
            assert!(limiter.try_acquire().await);
        }

        // Should rate limit after tokens exhausted
        assert!(!limiter.try_acquire().await);

        // Should refill after time passes
        sleep(Duration::from_secs(2)).await;
        assert!(limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_rate_limiter_within_limit() {
        // Test that requests within the limit pass
        let limiter = RateLimiter::new(5, 10.0); // 5 tokens, 10 per second refill

        // All 5 requests should pass
        for i in 0..5 {
            assert!(limiter.try_acquire().await, "Request {} should pass", i);
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_exceeds_limit() {
        // Test that requests exceeding the limit are rejected
        let limiter = RateLimiter::new(3, 10.0); // 3 tokens, 10 per second refill

        // First 3 requests should pass
        for _ in 0..3 {
            assert!(limiter.try_acquire().await);
        }

        // 4th request should be rejected
        assert!(
            !limiter.try_acquire().await,
            "4th request should be rate limited"
        );

        // 5th request should also be rejected
        assert!(
            !limiter.try_acquire().await,
            "5th request should also be rate limited"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_refill_after_window() {
        // Test that rate limit resets after window
        let limiter = RateLimiter::new(2, 5.0); // 2 tokens, 5 per second refill

        // Exhaust tokens
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);
        assert!(!limiter.try_acquire().await);

        // Wait for refill (200ms for 1 token at 5 tokens/sec rate)
        sleep(Duration::from_millis(250)).await;

        // Should have at least 1 token now
        assert!(
            limiter.try_acquire().await,
            "Should have refilled at least 1 token"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter_burst_then_refill() {
        // Test burst capacity followed by refill
        let limiter = RateLimiter::new(5, 2.0); // 5 tokens, 2 per second refill

        // Burst: use all 5 tokens
        for _ in 0..5 {
            assert!(limiter.try_acquire().await);
        }

        // Should be rate limited
        assert!(!limiter.try_acquire().await);

        // Wait 1 second - should get 2 tokens back
        sleep(Duration::from_secs(1)).await;

        // Should be able to make 2 more requests
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);

        // Third should be rejected
        assert!(!limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_rate_limiter_current_tokens() {
        let limiter = RateLimiter::new(10, 1.0);

        // Initially should have max tokens
        assert_eq!(limiter.current_tokens().await, 10);

        // Use some tokens
        limiter.try_acquire().await;
        limiter.try_acquire().await;
        limiter.try_acquire().await;

        assert_eq!(limiter.current_tokens().await, 7);

        // Exhaust all
        for _ in 0..7 {
            limiter.try_acquire().await;
        }

        assert_eq!(limiter.current_tokens().await, 0);
    }

    #[tokio::test]
    async fn test_rate_limiter_zero_refill_rate() {
        // Test with zero refill rate (pure burst bucket)
        let limiter = RateLimiter::new(3, 0.0); // 3 tokens, no refill

        // Use all tokens
        for _ in 0..3 {
            assert!(limiter.try_acquire().await);
        }

        // Should be rejected
        assert!(!limiter.try_acquire().await);

        // Wait a bit - still no refill
        sleep(Duration::from_millis(100)).await;
        assert!(!limiter.try_acquire().await);
    }

    // =========================================================================
    // Per-IP Rate Limiter Tests (TEST-04)
    // =========================================================================

    #[tokio::test]
    async fn test_per_ip_rate_limiter_basic() {
        let limiter = PerIpRateLimiter::new(5, 1.0); // 5 tokens per IP
        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

        // Both IPs should have independent buckets
        for _ in 0..5 {
            assert!(limiter.try_acquire(ip1).await);
            assert!(limiter.try_acquire(ip2).await);
        }

        // Both should be rate limited now
        assert!(!limiter.try_acquire(ip1).await);
        assert!(!limiter.try_acquire(ip2).await);
    }

    #[tokio::test]
    async fn test_per_ip_rate_limiter_isolation() {
        let limiter = PerIpRateLimiter::new(3, 10.0);
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        // Exhaust ip1's tokens
        for _ in 0..3 {
            assert!(limiter.try_acquire(ip1).await);
        }
        assert!(!limiter.try_acquire(ip1).await);

        // ip2 should still have all tokens
        for _ in 0..3 {
            assert!(limiter.try_acquire(ip2).await);
        }
    }

    #[tokio::test]
    async fn test_per_ip_rate_limiter_refill() {
        let limiter = PerIpRateLimiter::new(2, 5.0); // 2 tokens, 5 per second
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Exhaust tokens
        assert!(limiter.try_acquire(ip).await);
        assert!(limiter.try_acquire(ip).await);
        assert!(!limiter.try_acquire(ip).await);

        // Wait for refill
        sleep(Duration::from_millis(250)).await;

        // Should have at least 1 token
        assert!(limiter.try_acquire(ip).await);
    }

    #[tokio::test]
    async fn test_per_ip_rate_limiter_new_ip() {
        let limiter = PerIpRateLimiter::new(5, 1.0);
        let new_ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));

        // New IP should get a fresh bucket
        assert!(limiter.try_acquire(new_ip).await);
        assert!(limiter.try_acquire(new_ip).await);
        assert!(limiter.try_acquire(new_ip).await);
    }
}
