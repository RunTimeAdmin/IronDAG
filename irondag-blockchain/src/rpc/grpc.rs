//! gRPC Server Implementation (Tonic)
//!
//! High-performance RPC server using gRPC protocol
//! Supports HTTP/2, binary protobuf, and streaming.
//! Auth and client IP from metadata are passed to RPC for same policy as JSON-RPC.

use crate::rpc::RpcServer;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::info;

/// Extract API key and client IP from gRPC request for auth and per-IP rate limiting.
fn auth_from_request<T>(req: &Request<T>) -> (Option<String>, Option<std::net::IpAddr>) {
    let api_key = req
        .metadata()
        .get("x-api-key")
        .or_else(|| req.metadata().get("api-key"))
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let ip = req.remote_addr().map(|a| a.ip());
    (api_key, ip)
}

// Include generated protobuf code
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/irondag.rpc.rs"));
}

use proto::blockchain_service_server::{BlockchainService, BlockchainServiceServer};
use proto::*;

/// gRPC service implementation
pub struct BlockchainServiceImpl {
    rpc_server: Arc<RpcServer>,
}

impl BlockchainServiceImpl {
    pub fn new(rpc_server: Arc<RpcServer>) -> Self {
        Self { rpc_server }
    }
}

#[tonic::async_trait]
impl BlockchainService for BlockchainServiceImpl {
    async fn get_block_number(
        &self,
        request: Request<GetBlockNumberRequest>,
    ) -> Result<Response<GetBlockNumberResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_blockNumber".to_string(),
            params: None,
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let block_number = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x0".to_string());

        Ok(Response::new(GetBlockNumberResponse { block_number }))
    }

    async fn get_balance(
        &self,
        request: Request<GetBalanceRequest>,
    ) -> Result<Response<GetBalanceResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let address = req.address;
        let block_number = if req.block_number.is_empty() {
            "latest".to_string()
        } else {
            req.block_number
        };

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getBalance".to_string(),
            params: Some(serde_json::json!([address, block_number])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let balance = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x0".to_string());

        Ok(Response::new(GetBalanceResponse { balance }))
    }

    async fn get_transaction_count(
        &self,
        request: Request<GetTransactionCountRequest>,
    ) -> Result<Response<GetTransactionCountResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let address = req.address;
        let block_number = if req.block_number.is_empty() {
            "latest".to_string()
        } else {
            req.block_number
        };

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionCount".to_string(),
            params: Some(serde_json::json!([address, block_number])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let count = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x0".to_string());

        Ok(Response::new(GetTransactionCountResponse { count }))
    }

    async fn send_raw_transaction(
        &self,
        request: Request<SendRawTransactionRequest>,
    ) -> Result<Response<SendRawTransactionResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let raw_tx = req.raw_transaction;

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_sendRawTransaction".to_string(),
            params: Some(serde_json::json!([raw_tx])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let tx_hash = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| {
                Status::internal(
                    json_response
                        .error
                        .map(|e| e.message)
                        .unwrap_or_else(|| "Unknown error".to_string()),
                )
            })?;

        Ok(Response::new(SendRawTransactionResponse {
            transaction_hash: tx_hash,
        }))
    }

    async fn get_dag_stats(
        &self,
        request: Request<GetDagStatsRequest>,
    ) -> Result<Response<GetDagStatsResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "irondag_getDagStats".to_string(),
            params: None,
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let stats_json = json_response
            .result
            .ok_or_else(|| Status::internal("Failed to get DAG stats".to_string()))?;

        let stats = serde_json::from_value::<serde_json::Value>(stats_json)
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(GetDagStatsResponse {
            total_blocks: stats["total_blocks"].as_u64().unwrap_or(0),
            blue_blocks: stats["blue_blocks"].as_u64().unwrap_or(0),
            red_blocks: stats["red_blocks"].as_u64().unwrap_or(0),
            total_transactions: stats["total_transactions"].as_u64().unwrap_or(0),
            total_size_bytes: stats["total_size_bytes"].as_u64().unwrap_or(0),
            avg_block_size: stats["avg_block_size"].as_u64().unwrap_or(0),
            avg_txs_per_block: stats["avg_txs_per_block"].as_u64().unwrap_or(0),
        }))
    }

    async fn get_peer_count(
        &self,
        request: Request<GetPeerCountRequest>,
    ) -> Result<Response<GetPeerCountResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "net_peerCount".to_string(),
            params: None,
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let peer_count_str = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x0".to_string());

        // Parse hex string to u32
        let peer_count =
            u32::from_str_radix(peer_count_str.trim_start_matches("0x"), 16).unwrap_or(0);

        Ok(Response::new(GetPeerCountResponse { peer_count }))
    }

    async fn get_block_by_number(
        &self,
        request: Request<GetBlockByNumberRequest>,
    ) -> Result<Response<GetBlockByNumberResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let block_number = if req.block_number.is_empty() {
            "latest".to_string()
        } else {
            req.block_number
        };
        let include_txs = req.include_transactions;

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getBlockByNumber".to_string(),
            params: Some(serde_json::json!([block_number, include_txs])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let block_json = json_response.result.ok_or_else(|| {
            Status::internal(
                json_response
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "Block not found".to_string()),
            )
        })?;

        // Handle null result (block not found)
        if block_json.is_null() {
            return Ok(Response::new(GetBlockByNumberResponse { block: None }));
        }

        let block = map_block_from_json(&block_json)?;

        Ok(Response::new(GetBlockByNumberResponse {
            block: Some(block),
        }))
    }

    async fn get_block_by_hash(
        &self,
        request: Request<GetBlockByHashRequest>,
    ) -> Result<Response<GetBlockByHashResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let hash = req.hash;
        let include_txs = req.include_transactions;

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getBlockByHash".to_string(),
            params: Some(serde_json::json!([hash, include_txs])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let block_json = json_response.result.ok_or_else(|| {
            Status::internal(
                json_response
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "Block not found".to_string()),
            )
        })?;

        // Handle null result (block not found)
        if block_json.is_null() {
            return Ok(Response::new(GetBlockByHashResponse { block: None }));
        }

        let block = map_block_from_json(&block_json)?;

        Ok(Response::new(GetBlockByHashResponse { block: Some(block) }))
    }

    async fn get_transaction_by_hash(
        &self,
        request: Request<GetTransactionByHashRequest>,
    ) -> Result<Response<GetTransactionByHashResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let hash = req.hash;

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionByHash".to_string(),
            params: Some(serde_json::json!([hash])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let tx_json = json_response.result.ok_or_else(|| {
            Status::internal(
                json_response
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "Transaction not found".to_string()),
            )
        })?;

        // Handle null result (transaction not found)
        if tx_json.is_null() {
            return Ok(Response::new(GetTransactionByHashResponse {
                transaction: None,
            }));
        }

        let transaction = map_transaction_from_json(&tx_json)?;

        Ok(Response::new(GetTransactionByHashResponse {
            transaction: Some(transaction),
        }))
    }

    async fn get_transaction_receipt(
        &self,
        request: Request<GetTransactionReceiptRequest>,
    ) -> Result<Response<GetTransactionReceiptResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let hash = req.hash;

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getTransactionReceipt".to_string(),
            params: Some(serde_json::json!([hash])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let receipt_json = json_response.result.ok_or_else(|| {
            Status::internal(
                json_response
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_else(|| "Transaction receipt not found".to_string()),
            )
        })?;

        // Handle null result (receipt not found)
        if receipt_json.is_null() {
            return Ok(Response::new(GetTransactionReceiptResponse {
                receipt: None,
            }));
        }

        let receipt = map_receipt_from_json(&receipt_json)?;

        Ok(Response::new(GetTransactionReceiptResponse {
            receipt: Some(receipt),
        }))
    }

    async fn get_gas_price(
        &self,
        request: Request<GetGasPriceRequest>,
    ) -> Result<Response<GetGasPriceResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_gasPrice".to_string(),
            params: None,
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let gas_price = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x0".to_string());

        Ok(Response::new(GetGasPriceResponse { gas_price }))
    }

    async fn call(&self, request: Request<CallRequest>) -> Result<Response<CallResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let call = req
            .call
            .ok_or_else(|| Status::invalid_argument("Missing call object"))?;
        let block_number = if req.block_number.is_empty() {
            "latest".to_string()
        } else {
            req.block_number
        };

        let call_obj = serde_json::json!({
            "from": call.from,
            "to": call.to,
            "value": call.value,
            "data": call.data,
            "gasPrice": call.gas_price,
            "gas": if call.gas_limit > 0 { Some(call.gas_limit) } else { None },
        });

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_call".to_string(),
            params: Some(serde_json::json!([call_obj, block_number])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let result = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| {
                Status::internal(
                    json_response
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "Call failed".to_string()),
                )
            })?;

        Ok(Response::new(CallResponse { result }))
    }

    async fn estimate_gas(
        &self,
        request: Request<EstimateGasRequest>,
    ) -> Result<Response<EstimateGasResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let call = req
            .call
            .ok_or_else(|| Status::invalid_argument("Missing call object"))?;

        let call_obj = serde_json::json!({
            "from": call.from,
            "to": call.to,
            "value": call.value,
            "data": call.data,
            "gasPrice": call.gas_price,
            "gas": if call.gas_limit > 0 { Some(call.gas_limit) } else { None },
        });

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_estimateGas".to_string(),
            params: Some(serde_json::json!([call_obj])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let gas_estimate = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| {
                Status::internal(
                    json_response
                        .error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "Gas estimation failed".to_string()),
                )
            })?;

        Ok(Response::new(EstimateGasResponse { gas_estimate }))
    }

    async fn get_code(
        &self,
        request: Request<GetCodeRequest>,
    ) -> Result<Response<GetCodeResponse>, Status> {
        let (api_key, client_ip) = auth_from_request(&request);
        let req = request.into_inner();
        let address = req.address;
        let block_number = if req.block_number.is_empty() {
            "latest".to_string()
        } else {
            req.block_number
        };

        let json_request = crate::rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_getCode".to_string(),
            params: Some(serde_json::json!([address, block_number])),
            id: Some(serde_json::Value::Number(1.into())),
        };

        let json_response = self
            .rpc_server
            .handle_request(json_request, api_key.as_deref(), client_ip)
            .await;

        let code = json_response
            .result
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "0x".to_string());

        Ok(Response::new(GetCodeResponse { code }))
    }
}

/// Helper function to map JSON block to proto Block
#[allow(clippy::result_large_err)]
fn map_block_from_json(json: &serde_json::Value) -> Result<Block, Status> {
    let hash = json["hash"].as_str().unwrap_or("").to_string();
    let parent_hash = json["parentHash"].as_str().unwrap_or("").to_string();
    let parent_hashes = json["parentHashes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let block_number = json["number"]
        .as_str()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let timestamp = json["timestamp"]
        .as_str()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let difficulty = json["difficulty"]
        .as_str()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let nonce = json["nonce"].as_str().unwrap_or("0x0").to_string();
    let transactions_root = json["transactionsRoot"].as_str().unwrap_or("").to_string();
    let stream_type = json["streamType"].as_u64().unwrap_or(0) as u32;

    // Map transactions - handle both transaction hashes (strings) and full transaction objects
    let transactions: Vec<Transaction> = json["transactions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    if v.is_string() {
                        // Transaction hash only - create minimal transaction
                        Some(Transaction {
                            hash: v.as_str().unwrap_or("").to_string(),
                            from: String::new(),
                            to: String::new(),
                            value: String::new(),
                            data: String::new(),
                            nonce: 0,
                            gas_price: String::new(),
                            gas_limit: 0,
                            v: 0,
                            r: String::new(),
                            s: String::new(),
                        })
                    } else {
                        // Full transaction object
                        map_transaction_from_json(v).ok()
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Block {
        hash,
        parent_hash,
        parent_hashes,
        block_number,
        timestamp,
        difficulty,
        nonce,
        transactions_root,
        transactions,
        stream_type,
    })
}

/// Helper function to map JSON transaction to proto Transaction
#[allow(clippy::result_large_err)]
fn map_transaction_from_json(json: &serde_json::Value) -> Result<Transaction, Status> {
    Ok(Transaction {
        hash: json["hash"].as_str().unwrap_or("").to_string(),
        from: json["from"].as_str().unwrap_or("").to_string(),
        to: json["to"].as_str().unwrap_or("").to_string(),
        value: json["value"].as_str().unwrap_or("0x0").to_string(),
        data: json["data"].as_str().unwrap_or("0x").to_string(),
        nonce: json["nonce"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        gas_price: json["gasPrice"].as_str().unwrap_or("0x0").to_string(),
        gas_limit: json["gas"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        v: json["v"]
            .as_str()
            .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        r: json["r"].as_str().unwrap_or("").to_string(),
        s: json["s"].as_str().unwrap_or("").to_string(),
    })
}

/// Helper function to map JSON log to proto Log
#[allow(clippy::result_large_err)]
fn map_log_from_json(json: &serde_json::Value) -> Result<Log, Status> {
    let topics = json["topics"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(Log {
        address: json["address"].as_str().unwrap_or("").to_string(),
        topics,
        data: json["data"].as_str().unwrap_or("0x").to_string(),
        block_number: json["blockNumber"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        block_hash: json["blockHash"].as_str().unwrap_or("").to_string(),
        transaction_hash: json["transactionHash"].as_str().unwrap_or("").to_string(),
        transaction_index: json["transactionIndex"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        log_index: json["logIndex"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
    })
}

/// Helper function to map JSON receipt to proto TransactionReceipt
#[allow(clippy::result_large_err)]
fn map_receipt_from_json(json: &serde_json::Value) -> Result<TransactionReceipt, Status> {
    let logs = json["logs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| map_log_from_json(v).ok())
                .collect()
        })
        .unwrap_or_default();

    Ok(TransactionReceipt {
        transaction_hash: json["transactionHash"].as_str().unwrap_or("").to_string(),
        block_number: json["blockNumber"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        block_hash: json["blockHash"].as_str().unwrap_or("").to_string(),
        transaction_index: json["transactionIndex"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        from: json["from"].as_str().unwrap_or("").to_string(),
        to: json["to"].as_str().unwrap_or("").to_string(),
        gas_used: json["gasUsed"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        cumulative_gas_used: json["cumulativeGasUsed"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        contract_address: json["contractAddress"].as_str().unwrap_or("").to_string(),
        logs,
        status: json["status"].as_str().unwrap_or("0x0").to_string(),
    })
}

/// Start gRPC server
pub async fn start_grpc_server(
    port: u16,
    rpc_server: Arc<RpcServer>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("127.0.0.1:{}", port).parse()?;
    let service = BlockchainServiceImpl::new(rpc_server);

    info!("gRPC server listening on http://{}", addr);
    info!("  Supports: HTTP/2, Binary Protobuf, Streaming");

    tonic::transport::Server::builder()
        .add_service(BlockchainServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
