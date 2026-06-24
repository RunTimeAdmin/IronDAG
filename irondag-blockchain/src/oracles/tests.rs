#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::oracles::{FeedType, OracleRegistry, PriceFeedManager, PriceUpdate, VrfManager};
    use crate::types::{Address, Hash};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn test_oracle_registration_and_price_feed() {
        let registry = Arc::new(RwLock::new(OracleRegistry::default()));
        let price_feeds = Arc::new(RwLock::new(PriceFeedManager::new(registry.clone())));

        let oracle1 = Address::new([1u8; 20]);
        let oracle2 = Address::new([2u8; 20]);
        let oracle3 = Address::new([3u8; 20]);

        // Create feed
        {
            let price_feeds = price_feeds.clone();
            tokio::task::spawn_blocking(move || {
                price_feeds.blocking_write().create_feed(
                    "BTC/USD".to_string(),
                    ("BTC".to_string(), "USD".to_string()),
                    60,
                );
            })
            .await
            .unwrap();
        }

        // Register oracles
        registry
            .write()
            .await
            .register_oracle(
                oracle1,
                vec![FeedType::Price],
                2_000_000_000_000_000_000,
                1000,
            )
            .unwrap();

        registry
            .write()
            .await
            .register_oracle(
                oracle2,
                vec![FeedType::Price],
                2_000_000_000_000_000_000,
                1000,
            )
            .unwrap();

        registry
            .write()
            .await
            .register_oracle(
                oracle3,
                vec![FeedType::Price],
                2_000_000_000_000_000_000,
                1000,
            )
            .unwrap();

        // Assign oracles to feed
        registry
            .write()
            .await
            .assign_oracles_to_feed("BTC/USD".to_string(), vec![oracle1, oracle2, oracle3])
            .unwrap();

        {
            let price_feeds = price_feeds.clone();
            tokio::task::spawn_blocking(move || {
                let mut feeds = price_feeds.blocking_write();
                // Submit price updates
                feeds
                    .submit_price_update(PriceUpdate {
                        oracle_address: oracle1,
                        feed_id: "BTC/USD".to_string(),
                        price: 50_000_000_000_000_000_000,
                        timestamp: 2000,
                        signature: None,
                    })
                    .unwrap();

                feeds
                    .submit_price_update(PriceUpdate {
                        oracle_address: oracle2,
                        feed_id: "BTC/USD".to_string(),
                        price: 51_000_000_000_000_000_000,
                        timestamp: 2000,
                        signature: None,
                    })
                    .unwrap();

                feeds
                    .submit_price_update(PriceUpdate {
                        oracle_address: oracle3,
                        feed_id: "BTC/USD".to_string(),
                        price: 52_000_000_000_000_000_000,
                        timestamp: 2000,
                        signature: None,
                    })
                    .unwrap();

                // Aggregate
                feeds.aggregate_feed("BTC/USD", 2000).unwrap();
            })
            .await
            .unwrap();
        }

        // Median should be 51,000
        let median = {
            let price_feeds = price_feeds.clone();
            tokio::task::spawn_blocking(move || price_feeds.blocking_read().get_price("BTC/USD"))
                .await
                .unwrap()
        };
        assert_eq!(median, Some(51_000_000_000_000_000_000));
    }

    #[tokio::test]
    async fn test_vrf_randomness() {
        let vrf = Arc::new(RwLock::new(VrfManager::new()));
        let requester = Address::new([1u8; 20]);
        let seed = Hash([42u8; 32]);
        let timestamp = 1000;

        let request_id = vrf
            .write()
            .await
            .request_randomness(requester, seed, timestamp);

        // Fulfill request
        vrf.write()
            .await
            .fulfill_randomness(&request_id, requester)
            .unwrap();

        let vrf_read = vrf.read().await;
        let request = vrf_read.get_request(&request_id).unwrap();
        assert!(request.fulfilled);
        assert!(request.randomness.is_some());
    }
}
