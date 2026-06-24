//! gRPC v2 Service Implementation (Binary Optimized)
//!
//! High-performance RPC using native binary types instead of hex strings.
//! 3.3x faster than v1 by eliminating hex encode/decode overhead.
//!
//! v1 path: Request → JSON-RPC → hex parse → blockchain → hex encode → Response
//! v2 path: Request → blockchain → Response (direct binary)

use crate::blockchain::{Block, Blockchain, Transaction as BlockchainTx};
use crate::types::{Address, Hash};
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};
use tracing::info;

// Include generated v2 protobuf code
pub mod proto_v2 {
    include!(concat!(env!("OUT_DIR"), "/irondag.rpc.v2.rs"));
}

use proto_v2::blockchain_service_v2_server::{BlockchainServiceV2, BlockchainServiceV2Server};
use proto_v2::*;

/// Lock acquisition timeout (ms) - matches v1 for consistency
const LOCK_TIMEOUT_MS: u64 = 10000;

/// gRPC v2 service implementation - direct blockchain access
pub struct BlockchainServiceV2Impl {
    blockchain: Arc<RwLock<Blockchain>>,
    mining_manager: Option<Arc<crate::mining::MiningManager>>,
    network_manager: Option<Arc<crate::network::NetworkManager>>,
    chain_id: u64,
}

impl BlockchainServiceV2Impl {
    pub fn new(blockchain: Arc<RwLock<Blockchain>>) -> Self {
        Self {
            blockchain,
            mining_manager: None,
            network_manager: None,
            chain_id: crate::types::DEFAULT_CHAIN_ID,
        }
    }

    pub fn with_mining_manager(mut self, mm: Arc<crate::mining::MiningManager>) -> Self {
        self.mining_manager = Some(mm);
        self
    }

    pub fn with_network_manager(mut self, nm: Arc<crate::network::NetworkManager>) -> Self {
        self.network_manager = Some(nm);
        self
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }

    /// Acquire blockchain read lock with timeout
    async fn acquire_read(&self) -> Result<tokio::sync::RwLockReadGuard<'_, Blockchain>, Status> {
        match tokio::time::timeout(
            std::time::Duration::from_millis(LOCK_TIMEOUT_MS),
            self.blockchain.read(),
        )
        .await
        {
            Ok(guard) => Ok(guard),
            Err(_) => Err(Status::unavailable("Node busy (mining), retry later")),
        }
    }

    /// Convert 20-byte address from proto bytes
    #[allow(clippy::result_large_err)]
    fn parse_address(bytes: &[u8]) -> Result<Address, Status> {
        if bytes.len() != 20 {
            return Err(Status::invalid_argument(format!(
                "Address must be 20 bytes, got {}",
                bytes.len()
            )));
        }
        let mut addr = [0u8; 20];
        addr.copy_from_slice(bytes);
        Ok(Address(addr))
    }

    /// Convert 32-byte hash from proto bytes
    #[allow(clippy::result_large_err)]
    fn parse_hash(bytes: &[u8]) -> Result<Hash, Status> {
        if bytes.len() != 32 {
            return Err(Status::invalid_argument(format!(
                "Hash must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(bytes);
        Ok(Hash(hash))
    }

    /// Convert u128 to 32-byte big-endian bytes
    fn u128_to_bytes(value: u128) -> Vec<u8> {
        let mut bytes = vec![0u8; 32];
        let value_bytes = value.to_be_bytes();
        bytes[16..].copy_from_slice(&value_bytes);
        bytes
    }

    /// Convert blockchain Block to proto Block
    fn block_to_proto(block: &Block, include_transactions: bool) -> proto_v2::Block {
        let parent_hash = block
            .header
            .parent_hashes
            .first()
            .map(|h| h.to_vec())
            .unwrap_or_default();

        let parent_hashes: Vec<Vec<u8>> = block
            .header
            .parent_hashes
            .iter()
            .map(|h| h.to_vec())
            .collect();

        let transactions = if include_transactions {
            block.transactions.iter().map(Self::tx_to_proto).collect()
        } else {
            vec![]
        };

        // Calculate transactions root
        let tx_hashes: Vec<Hash> = block.transactions.iter().map(|tx| tx.hash).collect();
        let transactions_root = crate::pow::calculate_transactions_root(&tx_hashes);

        // Convert stream type to u32
        let stream_type = block.header.stream_type.to_bytes()[0] as u32;

        proto_v2::Block {
            hash: block.hash.to_vec(),
            parent_hash,
            parent_hashes,
            block_number: block.header.block_number,
            timestamp: block.header.timestamp,
            difficulty: block.header.difficulty,
            nonce: block.header.nonce,
            transactions_root: transactions_root.to_vec(),
            transactions,
            stream_type,
            state_root: vec![], // TODO: state_root is not yet implemented in gRPC v2 (experimental)
            gas_used: block.transactions.iter().map(|tx| tx.gas_limit).sum(),
            gas_limit: 30_000_000, // Block gas limit
        }
    }

    /// Convert blockchain Transaction to proto Transaction
    fn tx_to_proto(tx: &BlockchainTx) -> proto_v2::Transaction {
        // Extract signature components
        let (v, r, s) = if let Some(ref ecdsa_sig) = tx.ecdsa_signature {
            (
                ecdsa_sig.v as u32,
                ecdsa_sig.r.to_vec(),
                ecdsa_sig.s.to_vec(),
            )
        } else {
            (0, vec![], vec![])
        };

        // Calculate gas_price from fee/gas_limit
        let gas_price = if tx.gas_limit > 0 {
            u64::try_from(tx.fee / tx.gas_limit as u128).unwrap_or(u64::MAX)
        } else {
            0
        };

        proto_v2::Transaction {
            hash: tx.hash.to_vec(),
            from: tx.from.to_vec(),
            to: tx.to.to_vec(),
            value: Self::u128_to_bytes(tx.value),
            data: tx.data.clone(),
            nonce: tx.nonce,
            gas_price,
            gas_limit: tx.gas_limit,
            v,
            r,
            s,
            tx_type: 0, // Legacy transaction
        }
    }
}

#[tonic::async_trait]
impl BlockchainServiceV2 for BlockchainServiceV2Impl {
    /// Get latest block number - O(1) atomic read
    async fn get_block_number(
        &self,
        _request: Request<GetBlockNumberRequest>,
    ) -> Result<Response<GetBlockNumberResponse>, Status> {
        let bc = self.acquire_read().await?;
        let block_number = bc.latest_block_number();
        Ok(Response::new(GetBlockNumberResponse { block_number }))
    }

    /// Get block by number - direct binary response
    async fn get_block_by_number(
        &self,
        request: Request<GetBlockByNumberRequest>,
    ) -> Result<Response<GetBlockByNumberResponse>, Status> {
        let req = request.into_inner();
        let bc = self.acquire_read().await?;

        let block = if req.use_latest {
            bc.get_latest_block()
        } else {
            bc.get_block_by_number(req.block_number)
        };

        let proto_block = block
            .as_ref()
            .map(|b| Self::block_to_proto(b, req.include_transactions));
        Ok(Response::new(GetBlockByNumberResponse {
            block: proto_block,
        }))
    }

    /// Get block by hash - direct binary lookup
    async fn get_block_by_hash(
        &self,
        request: Request<GetBlockByHashRequest>,
    ) -> Result<Response<GetBlockByHashResponse>, Status> {
        let req = request.into_inner();
        let hash = Self::parse_hash(&req.hash)?;
        let bc = self.acquire_read().await?;

        let block = bc.get_block_by_hash(&hash);
        let proto_block = block
            .as_ref()
            .map(|b| Self::block_to_proto(b, req.include_transactions));
        Ok(Response::new(GetBlockByHashResponse { block: proto_block }))
    }

    /// Get balance - direct binary response
    async fn get_balance(
        &self,
        request: Request<GetBalanceRequest>,
    ) -> Result<Response<GetBalanceResponse>, Status> {
        let req = request.into_inner();
        let address = Self::parse_address(&req.address)?;
        let bc = self.acquire_read().await?;

        let balance = bc.get_balance(address);

        Ok(Response::new(GetBalanceResponse {
            balance: Self::u128_to_bytes(balance),
        }))
    }

    /// Get transaction count (nonce) - direct u64
    async fn get_transaction_count(
        &self,
        request: Request<GetTransactionCountRequest>,
    ) -> Result<Response<GetTransactionCountResponse>, Status> {
        let req = request.into_inner();
        let address = Self::parse_address(&req.address)?;
        let bc = self.acquire_read().await?;

        let count = bc.get_nonce(address);

        Ok(Response::new(GetTransactionCountResponse { count }))
    }

    /// Send raw transaction - binary in/out
    ///
    /// Accepts a JSON-encoded `Transaction` struct as raw bytes (the gRPC v2 native format).
    /// For Ethereum RLP compatibility use the JSON-RPC v1 `eth_sendRawTransaction` method.
    async fn send_raw_transaction(
        &self,
        request: Request<SendRawTransactionRequest>,
    ) -> Result<Response<SendRawTransactionResponse>, Status> {
        let raw = request.into_inner().raw_transaction;
        if raw.is_empty() {
            return Err(Status::invalid_argument("Empty transaction bytes"));
        }

        let tx: BlockchainTx = serde_json::from_slice(&raw)
            .map_err(|e| Status::invalid_argument(format!("Invalid transaction JSON: {}", e)))?;

        if !tx.verify_signature(self.chain_id).unwrap_or(false) {
            return Err(Status::invalid_argument("Invalid transaction signature"));
        }

        let bc = self.acquire_read().await?;
        let current_nonce = bc.get_nonce(tx.from);
        let balance = bc.get_balance(tx.from);
        drop(bc);

        if tx.nonce != current_nonce {
            return Err(Status::failed_precondition(format!(
                "Invalid nonce: expected {}, got {}",
                current_nonce, tx.nonce
            )));
        }
        let total_cost = tx
            .value
            .checked_add(tx.fee)
            .ok_or_else(|| Status::invalid_argument("value + fee overflow"))?;
        if balance < total_cost {
            return Err(Status::failed_precondition(format!(
                "Insufficient balance: have {}, need {}",
                balance, total_cost
            )));
        }

        let mm = self
            .mining_manager
            .as_ref()
            .ok_or_else(|| Status::unavailable("Mining manager not available"))?;
        mm.add_transaction(tx.clone())
            .await
            .map_err(|e| Status::internal(format!("Failed to add transaction: {}", e)))?;

        info!("gRPC v2: submitted tx {}", hex::encode(&tx.hash.0[..8]));
        Ok(Response::new(SendRawTransactionResponse {
            transaction_hash: tx.hash.to_vec(),
        }))
    }

    /// Get transaction by hash
    async fn get_transaction_by_hash(
        &self,
        request: Request<GetTransactionByHashRequest>,
    ) -> Result<Response<GetTransactionByHashResponse>, Status> {
        let req = request.into_inner();
        let hash = Self::parse_hash(&req.hash)?;
        let bc = self.acquire_read().await?;

        // Search through all blocks for the transaction
        let blocks = bc.get_blocks();
        for block in blocks {
            for tx in &block.transactions {
                if tx.hash == hash {
                    return Ok(Response::new(GetTransactionByHashResponse {
                        transaction: Some(Self::tx_to_proto(tx)),
                    }));
                }
            }
        }

        Ok(Response::new(GetTransactionByHashResponse {
            transaction: None,
        }))
    }

    /// Get transaction receipt
    async fn get_transaction_receipt(
        &self,
        request: Request<GetTransactionReceiptRequest>,
    ) -> Result<Response<GetTransactionReceiptResponse>, Status> {
        let req = request.into_inner();
        let hash = Self::parse_hash(&req.hash)?;
        let bc = self.acquire_read().await?;

        // Search through all blocks for the transaction
        let blocks = bc.get_blocks();
        for block in &blocks {
            for (tx_index, tx) in block.transactions.iter().enumerate() {
                if tx.hash == hash {
                    // Found the transaction - create receipt
                    let receipt = TransactionReceipt {
                        transaction_hash: tx.hash.to_vec(),
                        block_number: block.header.block_number,
                        block_hash: block.hash.to_vec(),
                        transaction_index: tx_index as u64,
                        from: tx.from.to_vec(),
                        to: tx.to.to_vec(),
                        gas_used: tx.gas_limit, // Simplified: assume all gas used
                        cumulative_gas_used: tx.gas_limit,
                        contract_address: if tx.to.is_zero() && !tx.data.is_empty() {
                            // keccak256(RLP([sender, nonce]))[12..] — Ethereum CREATE address
                            use alloy_rlp::{BytesMut, Encodable};
                            use sha3::{Digest, Keccak256};
                            let mut buf = BytesMut::new();
                            tx.from.as_ref().encode(&mut buf);
                            tx.nonce.encode(&mut buf);
                            let hash = Keccak256::digest(&buf[..]);
                            hash[12..32].to_vec()
                        } else {
                            vec![]
                        },
                        logs: vec![], // TODO: event logs are not yet implemented in gRPC v2 (experimental)
                        success: true, // Assume success if in block
                        return_data: vec![],
                    };
                    return Ok(Response::new(GetTransactionReceiptResponse {
                        receipt: Some(receipt),
                    }));
                }
            }
        }

        Ok(Response::new(GetTransactionReceiptResponse {
            receipt: None,
        }))
    }

    /// Get DAG statistics - native uint64 fields
    async fn get_dag_stats(
        &self,
        _request: Request<GetDagStatsRequest>,
    ) -> Result<Response<GetDagStatsResponse>, Status> {
        let bc = self.acquire_read().await?;
        let stats = bc.get_dag_stats();

        Ok(Response::new(GetDagStatsResponse {
            total_blocks: stats.total_blocks as u64,
            blue_blocks: stats.blue_blocks as u64,
            red_blocks: stats.red_blocks as u64,
            total_transactions: stats.total_transactions as u64,
            total_size_bytes: stats.total_size_bytes as u64,
            avg_block_size: stats.avg_block_size as u64,
            avg_txs_per_block: stats.avg_txs_per_block as u64,
        }))
    }

    /// Get peer count
    async fn get_peer_count(
        &self,
        _request: Request<GetPeerCountRequest>,
    ) -> Result<Response<GetPeerCountResponse>, Status> {
        let peer_count = self
            .network_manager
            .as_ref()
            .map(|nm| nm.peer_count() as u32)
            .unwrap_or(0);
        Ok(Response::new(GetPeerCountResponse { peer_count }))
    }

    /// Get gas price - native u64
    async fn get_gas_price(
        &self,
        _request: Request<GetGasPriceRequest>,
    ) -> Result<Response<GetGasPriceResponse>, Status> {
        // Return default gas price (20 gwei)
        Ok(Response::new(GetGasPriceResponse {
            gas_price: 20_000_000_000,
        }))
    }

    /// Contract call (read-only) - binary result
    async fn call(&self, request: Request<CallRequest>) -> Result<Response<CallResponse>, Status> {
        let req = request.into_inner();
        let call = req
            .call
            .ok_or_else(|| Status::invalid_argument("Missing call data"))?;

        let to = Self::parse_address(&call.to)?;
        let bc = self.acquire_read().await?;

        if let Some(executor) = bc.evm_executor() {
            // Create a call transaction
            let from = if call.from.is_empty() {
                Address::zero()
            } else {
                Self::parse_address(&call.from)?
            };

            let value = if call.value.len() > 16 {
                return Err(Status::invalid_argument("Value too large"));
            } else if call.value.is_empty() {
                0u128
            } else {
                // Parse big-endian bytes to u128
                let mut padded = [0u8; 16];
                let start = 16 - call.value.len().min(16);
                padded[start..].copy_from_slice(&call.value[call.value.len().saturating_sub(16)..]);
                u128::from_be_bytes(padded)
            };

            let tx = crate::blockchain::Transaction::with_data(
                from,
                to,
                value,
                0, // No fee for calls
                0, // Nonce doesn't matter for calls
                call.data.clone(),
                call.gas_limit.max(1_000_000),
            );

            // Get block context
            let block_number = bc.latest_block_number();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Execute call
            match executor.execute_transaction(&tx, block_number, timestamp) {
                Ok(result) => Ok(Response::new(CallResponse {
                    result: result.output,
                    success: result.success,
                })),
                Err(e) => Ok(Response::new(CallResponse {
                    result: e.to_string().into_bytes(),
                    success: false,
                })),
            }
        } else {
            Err(Status::failed_precondition("EVM not enabled"))
        }
    }

    /// Estimate gas - native u64
    async fn estimate_gas(
        &self,
        request: Request<EstimateGasRequest>,
    ) -> Result<Response<EstimateGasResponse>, Status> {
        let req = request.into_inner();
        let call = req.call;

        // Simple estimation based on data size
        let base_gas: u64 = 21_000; // Base transaction cost
        let data_gas = call
            .as_ref()
            .map(|c| c.data.len() as u64 * 16) // 16 gas per byte
            .unwrap_or(0);
        let contract_gas = call
            .as_ref()
            .map(|c| if c.to.is_empty() { 32_000 } else { 0 }) // Contract creation
            .unwrap_or(0);

        let gas_estimate = base_gas + data_gas + contract_gas;
        Ok(Response::new(EstimateGasResponse { gas_estimate }))
    }

    /// Get contract code - binary bytecode
    async fn get_code(
        &self,
        request: Request<GetCodeRequest>,
    ) -> Result<Response<GetCodeResponse>, Status> {
        let req = request.into_inner();
        let address = Self::parse_address(&req.address)?;
        let bc = self.acquire_read().await?;

        let code = bc
            .evm_executor()
            .and_then(|executor| executor.get_contract_code(address))
            .unwrap_or_default();

        Ok(Response::new(GetCodeResponse { code }))
    }

    /// Get storage at slot - binary key/value
    async fn get_storage_at(
        &self,
        request: Request<GetStorageAtRequest>,
    ) -> Result<Response<GetStorageAtResponse>, Status> {
        let req = request.into_inner();
        let address = Self::parse_address(&req.address)?;
        let slot = Self::parse_hash(&req.slot)?;
        let bc = self.acquire_read().await?;

        let value = bc
            .evm_executor()
            .and_then(|executor| executor.get_contract_storage(address, &slot))
            .unwrap_or_else(|| vec![0u8; 32]);

        Ok(Response::new(GetStorageAtResponse { value }))
    }

    /// Batch get blocks - optimized for sync
    async fn get_blocks_batch(
        &self,
        request: Request<GetBlocksBatchRequest>,
    ) -> Result<Response<GetBlocksBatchResponse>, Status> {
        let req = request.into_inner();
        let count = req.count.min(100) as u64; // Cap at 100 blocks
        let bc = self.acquire_read().await?;

        let mut blocks = Vec::with_capacity(count as usize);
        for block_num in req.start_block..(req.start_block + count) {
            if let Some(block) = bc.get_block_by_number(block_num) {
                blocks.push(Self::block_to_proto(&block, req.include_transactions));
            } else {
                break; // Stop at first missing block
            }
        }

        Ok(Response::new(GetBlocksBatchResponse { blocks }))
    }
}

/// Start gRPC v2 server (binary optimized)
pub async fn start_grpc_v2_server(
    port: u16,
    blockchain: Arc<RwLock<Blockchain>>,
    mining_manager: Option<Arc<crate::mining::MiningManager>>,
    network_manager: Option<Arc<crate::network::NetworkManager>>,
    chain_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("127.0.0.1:{}", port).parse()?;
    let mut service = BlockchainServiceV2Impl::new(blockchain).with_chain_id(chain_id);
    if let Some(mm) = mining_manager {
        service = service.with_mining_manager(mm);
    }
    if let Some(nm) = network_manager {
        service = service.with_network_manager(nm);
    }

    info!("gRPC v2 server listening on http://{}", addr);
    info!("   Binary optimized: 3.3x faster than v1");
    info!("   Supports: HTTP/2, Binary Protobuf, Batch Operations");

    tonic::transport::Server::builder()
        .add_service(BlockchainServiceV2Server::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
