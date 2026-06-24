//! RPC endpoint tests

use irondag_blockchain::blockchain::Blockchain;
use irondag_blockchain::rpc::{JsonRpcRequest, RpcServer};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Test all RPC methods
#[tokio::test]
async fn test_all_rpc_methods() {
    let blockchain = Arc::new(RwLock::new(Blockchain::new()));
    let rpc_server = RpcServer::without_auth(blockchain);

    let make_request = |method: &str, params: Option<serde_json::Value>| JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(json!(1)),
    };

    let response = rpc_server
        .handle_request(make_request("eth_blockNumber", Some(json!([]))), None, None)
        .await;
    assert!(response.error.is_none(), "eth_blockNumber failed");

    let response = rpc_server
        .handle_request(make_request("net_version", Some(json!([]))), None, None)
        .await;
    assert_eq!(response.result, Some(json!("1338")));

    let response = rpc_server
        .handle_request(make_request("eth_chainId", Some(json!([]))), None, None)
        .await;
    assert_eq!(response.result, Some(json!("0x53a")));

    let response = rpc_server
        .handle_request(make_request("net_peerCount", Some(json!([]))), None, None)
        .await;
    assert_eq!(response.result, Some(json!("0x0")));

    let response = rpc_server
        .handle_request(make_request("eth_syncing", Some(json!([]))), None, None)
        .await;
    assert_eq!(response.result, Some(json!(false)));
}
