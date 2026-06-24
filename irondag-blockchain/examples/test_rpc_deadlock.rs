//! Deadlock diagnostic test for RPC server
//!
//! This test isolates each component to find where the lock is held
#![allow(unexpected_cfgs)]

#[cfg(feature = "reqwest")]
use irondag_blockchain::node::{Node, NodeConfig};
#[cfg(feature = "reqwest")]
use std::sync::Arc;
#[cfg(feature = "reqwest")]
use tokio::time::{timeout, Duration};

#[cfg(feature = "reqwest")]
#[tokio::main]
async fn main() {
    println!("🔍 RPC Deadlock Diagnostic Test\n");

    let mut config = NodeConfig::default();
    config.rpc_port = 8545;
    config.data_dir = "data-deadlock-test".to_string();
    config.enable_mining = false;

    println!("✅ Config created");

    // Test 1: Can we create a node?
    println!("\n📝 Test 1: Creating Node instance...");
    let node = Arc::new(Node::new(config.clone()));
    println!("✅ Node instance created");

    // Test 2: Can we start the node with timeout?
    println!("\n📝 Test 2: Starting node (with 10s timeout)...");
    match timeout(Duration::from_secs(10), node.start()).await {
        Ok(Ok(())) => println!("✅ Node started successfully"),
        Ok(Err(e)) => {
            println!("❌ Node start failed: {}", e);
            return;
        }
        Err(_) => {
            println!("❌ Node start TIMED OUT - deadlock detected during startup!");
            println!("   This means a lock is being held indefinitely during Node::start()");
            return;
        }
    }

    // Test 3: Can we access blockchain with timeout?
    println!("\n📝 Test 3: Accessing blockchain (with 5s timeout)...");
    let blockchain = node.blockchain();
    match timeout(Duration::from_secs(5), async {
        let bc = blockchain.read().await;
        let height = bc.latest_block_number();
        println!("   Block height: {}", height);
    })
    .await
    {
        Ok(()) => println!("✅ Blockchain read succeeded"),
        Err(_) => {
            println!("❌ Blockchain read TIMED OUT - RwLock deadlock!");
            println!("   The blockchain RwLock is held and never released");
            return;
        }
    }

    // Test 4: Can we make a simple RPC call?
    println!("\n📝 Test 4: Testing RPC call (with 5s timeout)...");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let rpc_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_blockNumber",
        "params": [],
        "id": 1
    });

    match timeout(Duration::from_secs(5), async {
        client
            .post("http://127.0.0.1:8545")
            .json(&rpc_request)
            .send()
            .await
    })
    .await
    {
        Ok(Ok(response)) => {
            println!("✅ RPC call succeeded");
            if let Ok(text) = response.text().await {
                println!("   Response: {}", text);
            }
        }
        Ok(Err(e)) => println!("❌ RPC call failed: {}", e),
        Err(_) => {
            println!("❌ RPC call TIMED OUT");
            println!("   The RPC handler is not responding");
        }
    }

    // Test 5: Can we make a second RPC call?
    println!("\n📝 Test 5: Testing second RPC call (with 5s timeout)...");
    tokio::time::sleep(Duration::from_secs(1)).await;

    match timeout(Duration::from_secs(5), async {
        client
            .post("http://127.0.0.1:8545")
            .json(&rpc_request)
            .send()
            .await
    })
    .await
    {
        Ok(Ok(response)) => {
            println!("✅ Second RPC call succeeded");
            if let Ok(text) = response.text().await {
                println!("   Response: {}", text);
            }
        }
        Ok(Err(e)) => println!("❌ Second RPC call failed: {}", e),
        Err(_) => {
            println!("❌ Second RPC call TIMED OUT");
            println!("   Lock is not being released between calls!");
        }
    }

    println!("\n✅ All tests completed");
    println!("\n💡 Analysis:");
    println!("   - If Test 2 times out: Deadlock during node initialization");
    println!("   - If Test 3 times out: Blockchain RwLock is permanently held");
    println!("   - If Test 4 times out: RPC handler can't acquire lock");
    println!("   - If Test 5 times out: Lock not released after first RPC call");
}

#[cfg(not(feature = "reqwest"))]
fn main() {
    eprintln!("reqwest feature not enabled; skipping RPC deadlock test.");
}
