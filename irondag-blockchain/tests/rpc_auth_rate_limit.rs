//! RPC authentication and rate limiting tests

use irondag_blockchain::blockchain::Blockchain;
use irondag_blockchain::rpc::rate_limit::PerIpRateLimiter;
use irondag_blockchain::rpc::{JsonRpcRequest, RpcServer};
use serde_json::json;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

fn make_request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(json!(1)),
    }
}

// --- Authentication tests ---

#[tokio::test]
async fn test_rpc_auth_no_key_rejected() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let rpc_server = RpcServer::with_auth(blockchain, "secret-key".to_string());

    // net_peerCount is not in public_methods, so it requires auth
    let req = make_request("net_peerCount", Some(json!([])));
    let response = rpc_server.handle_request(req, None, None).await;

    assert!(
        response.error.is_some(),
        "Expected auth rejection when no key provided"
    );
    let err = response.error.unwrap();
    assert!(err.message.contains("Unauthorized") || err.message.contains("API key"));
}

#[tokio::test]
async fn test_rpc_auth_wrong_key_rejected() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let rpc_server = RpcServer::with_auth(blockchain, "correct-key".to_string());

    // net_peerCount requires auth
    let req = make_request("net_peerCount", Some(json!([])));
    let response = rpc_server
        .handle_request(req, Some("wrong-key"), None)
        .await;

    assert!(
        response.error.is_some(),
        "Expected auth rejection for wrong key"
    );
}

#[tokio::test]
async fn test_rpc_auth_correct_key_accepted() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let api_key = "my-secret-key-123";
    let rpc_server = RpcServer::with_auth(blockchain, api_key.to_string());

    // net_peerCount requires auth
    let req = make_request("net_peerCount", Some(json!([])));
    let response = rpc_server.handle_request(req, Some(api_key), None).await;

    assert!(
        response.error.is_none(),
        "Expected success with correct key: {:?}",
        response.error
    );
}

#[tokio::test]
async fn test_rpc_no_auth_when_disabled() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let rpc_server = RpcServer::without_auth(blockchain);

    let req = make_request("net_peerCount", Some(json!([])));
    let response = rpc_server.handle_request(req, None, None).await;

    assert!(response.error.is_none(), "No auth required when disabled");
}

// --- Rate limiting tests ---

#[tokio::test]
async fn test_rpc_per_ip_rate_limit() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let mut rpc_server = RpcServer::without_auth_with_chain_id(blockchain, 1338);

    // 5 tokens, 1 refill per second - burst of 5 then limit
    let limiter = Arc::new(PerIpRateLimiter::new(5, 1.0));
    rpc_server.set_per_ip_rate_limiter(limiter);

    let ip: IpAddr = "127.0.0.1".parse().unwrap();

    // First 5 should succeed
    for i in 0..5 {
        let r = make_request("net_peerCount", Some(json!([])));
        let response = rpc_server.handle_request(r, None, Some(ip)).await;
        assert!(response.error.is_none(), "Request {} should succeed", i + 1);
    }

    // 6th should be rate limited
    let r = make_request("net_peerCount", Some(json!([])));
    let response = rpc_server.handle_request(r, None, Some(ip)).await;
    assert!(
        response.error.is_some(),
        "6th request should be rate limited"
    );
    let err = response.error.unwrap();
    assert!(
        err.message.contains("Rate limit") || err.code == -32005,
        "Expected rate limit error: {:?}",
        err
    );
}

#[tokio::test]
async fn test_rpc_different_ips_separate_buckets() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let mut rpc_server = RpcServer::without_auth_with_chain_id(blockchain, 1338);
    rpc_server.set_per_ip_rate_limiter(Arc::new(PerIpRateLimiter::new(2, 1.0)));

    let ip1: IpAddr = "192.168.1.1".parse().unwrap();
    let ip2: IpAddr = "192.168.1.2".parse().unwrap();

    // Exhaust ip1's bucket (2 requests)
    for _ in 0..2 {
        let rq = make_request("net_peerCount", Some(json!([])));
        let r = rpc_server.handle_request(rq, None, Some(ip1)).await;
        assert!(r.error.is_none());
    }
    let rq = make_request("net_peerCount", Some(json!([])));
    let r = rpc_server.handle_request(rq, None, Some(ip1)).await;
    assert!(r.error.is_some(), "ip1 should be rate limited");

    // ip2 should still get through (separate bucket)
    let rq = make_request("net_peerCount", Some(json!([])));
    let r = rpc_server.handle_request(rq, None, Some(ip2)).await;
    assert!(r.error.is_none(), "ip2 should have own bucket");
}
