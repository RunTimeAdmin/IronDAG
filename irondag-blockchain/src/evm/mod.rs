//! EVM (Ethereum Virtual Machine) integration
//!
//! Full EVM integration using SputnikVM (evm crate) for smart contract execution.
//!
//! This module provides EVM transaction execution, contract deployment,
//! and state management using the SputnikVM library.
//!
//! ## Architecture
//!
//! The EVM executor uses SputnikVM with proper storage exposure via ApplyBackend.

use crate::blockchain::Transaction;
use crate::types::{keccak256, Address, DEFAULT_CHAIN_ID};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use tracing::debug;

// SputnikVM imports for EVM execution with proper storage exposure
use evm::backend::{Apply, ApplyBackend, Backend, Basic, Log};
use primitive_types::{H160, H256, U256 as SputnikU256};

/// Local constant for chain ID (uses DEFAULT_CHAIN_ID as fallback)
const DEFAULT_CHAIN_ID_LOCAL: u64 = DEFAULT_CHAIN_ID;

/// EVM state manager
///
/// Manages EVM account state, contract storage, and execution environment.
#[derive(Clone)]
pub struct EvmState {
    /// Contract code storage (address -> bytecode)
    contracts: Arc<RwLock<HashMap<Address, Vec<u8>>>>,
    /// Account balances in EVM (separate from blockchain balances)
    balances: Arc<RwLock<HashMap<Address, u128>>>,
    /// Account nonces in EVM
    nonces: Arc<RwLock<HashMap<Address, u64>>>,
}

impl EvmState {
    /// Creates a new empty EVM state.
    pub fn new() -> Self {
        Self {
            contracts: Arc::new(RwLock::new(HashMap::new())),
            balances: Arc::new(RwLock::new(HashMap::new())),
            nonces: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns the account balance for the given address.
    pub fn get_balance(&self, address: Address) -> u128 {
        let balances = self.balances.read().unwrap_or_else(|e| e.into_inner());
        balances.get(&address).copied().unwrap_or_else(|| {
            debug!(
                "No balance found for {}, defaulting to 0",
                hex::encode(&address)
            );
            0
        })
    }

    /// Sets the account balance for the given address.
    pub fn set_balance(&self, address: Address, balance: u128) {
        let mut balances = self.balances.write().unwrap_or_else(|e| e.into_inner());
        balances.insert(address, balance);
    }

    /// Returns the account nonce for the given address.
    pub fn get_nonce(&self, address: Address) -> u64 {
        let nonces = self.nonces.read().unwrap_or_else(|e| e.into_inner());
        nonces.get(&address).copied().unwrap_or_else(|| {
            debug!(
                "No nonce found for {}, defaulting to 0",
                hex::encode(&address)
            );
            0
        })
    }

    /// Sets the account nonce for the given address.
    pub fn set_nonce(&self, address: Address, nonce: u64) {
        let mut nonces = self.nonces.write().unwrap_or_else(|e| e.into_inner());
        nonces.insert(address, nonce);
    }

    /// Stores contract bytecode at the given address.
    pub fn store_contract(&self, address: Address, code: Vec<u8>) {
        let mut contracts = self.contracts.write().unwrap_or_else(|e| e.into_inner());
        contracts.insert(address, code);
    }

    /// Returns the contract bytecode for the given address.
    pub fn get_contract_code(&self, address: Address) -> Option<Vec<u8>> {
        let contracts = self.contracts.read().unwrap_or_else(|e| e.into_inner());
        contracts.get(&address).cloned()
    }

    /// Returns true if the address has associated contract code.
    pub fn is_contract(&self, address: Address) -> bool {
        let contracts = self.contracts.read().unwrap_or_else(|e| e.into_inner());
        contracts.contains_key(&address)
    }
}

// ============================================================================
// SPUTNIKVM BACKEND IMPLEMENTATION
// ============================================================================

/// Convert native Address to SputnikVM H160
fn native_to_sputnik_address(address: Address) -> H160 {
    H160::from(address.0)
}

/// Convert SputnikVM H160 to native Address
fn sputnik_to_native_address(address: H160) -> Address {
    let bytes: [u8; 20] = address.into();
    Address(bytes)
}

/// Convert 32-byte storage key/value to H256
#[allow(dead_code)]
fn bytes32_to_h256(bytes: &[u8; 32]) -> H256 {
    H256::from(bytes)
}

/// Convert H256 to 32-byte array
fn h256_to_bytes32(h: H256) -> [u8; 32] {
    let mut arr = [0u8; 32];
    arr.copy_from_slice(h.as_bytes());
    arr
}

/// Block environment for SputnikVM execution
#[derive(Clone, Debug)]
pub struct SputnikVicinity {
    pub block_number: u64,
    pub block_timestamp: u64,
    pub block_gas_limit: u64,
    pub block_coinbase: Address,
    pub chain_id: u64,
    pub origin: Address,
    pub gas_price: u128,
}

/// SputnikVM backend that wraps EvmTransactionExecutor's state
///
/// Implements both Backend (read-only) and ApplyBackend (state mutation) traits.
/// The apply() method is the KEY - it receives storage changes as (H256, H256) pairs.
pub struct SputnikBackend<'a> {
    /// Reference to the executor for state access
    executor: &'a EvmTransactionExecutor,
    /// Block environment
    vicinity: SputnikVicinity,
    /// EVM event logs captured from apply() — no longer discarded
    captured_logs: Arc<RwLock<Vec<EvmLog>>>,
}

impl<'a> SputnikBackend<'a> {
    /// Creates a new SputnikBackend wrapping the executor.
    pub fn new(executor: &'a EvmTransactionExecutor, vicinity: SputnikVicinity) -> Self {
        Self {
            executor,
            vicinity,
            captured_logs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Take the captured EVM logs out of the backend, leaving it empty.
    pub fn take_logs(&self) -> Vec<EvmLog> {
        let mut logs = self
            .captured_logs
            .write()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *logs)
    }
}

impl<'a> Backend for SputnikBackend<'a> {
    fn gas_price(&self) -> SputnikU256 {
        SputnikU256::from(self.vicinity.gas_price)
    }

    fn origin(&self) -> H160 {
        native_to_sputnik_address(self.vicinity.origin)
    }

    fn block_hash(&self, number: SputnikU256) -> H256 {
        // SEC-013: Look up block hash from ring buffer (shared with executor)
        let block_num = number.low_u64();
        let block_hashes = self
            .executor
            .block_hashes
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for (num, hash) in block_hashes.iter() {
            if *num == block_num {
                return *hash;
            }
        }
        H256::default() // Block not in recent history
    }

    fn block_number(&self) -> SputnikU256 {
        SputnikU256::from(self.vicinity.block_number)
    }

    fn block_coinbase(&self) -> H160 {
        native_to_sputnik_address(self.vicinity.block_coinbase)
    }

    fn block_timestamp(&self) -> SputnikU256 {
        SputnikU256::from(self.vicinity.block_timestamp)
    }

    fn block_difficulty(&self) -> SputnikU256 {
        SputnikU256::zero() // Post-merge, difficulty is 0
    }

    fn block_randomness(&self) -> Option<H256> {
        // Post-merge, use prevrandao - we can implement this later
        None
    }

    fn block_gas_limit(&self) -> SputnikU256 {
        SputnikU256::from(self.vicinity.block_gas_limit)
    }

    fn block_base_fee_per_gas(&self) -> SputnikU256 {
        SputnikU256::from(1u64) // Minimum base fee
    }

    fn chain_id(&self) -> SputnikU256 {
        SputnikU256::from(self.vicinity.chain_id)
    }

    fn exists(&self, address: H160) -> bool {
        let native = sputnik_to_native_address(address);
        // Check if account has balance, nonce, or code
        // CRITICAL: For contracts, we MUST return true if code exists
        // Otherwise SputnikVM will skip code execution
        let has_code = self
            .executor
            .get_contract_code(native)
            .map(|c| !c.is_empty())
            .unwrap_or_else(|| {
                debug!(
                    "No contract code found for {}, defaulting to false",
                    hex::encode(&native)
                );
                false
            });
        let has_balance = self.executor.state.get_balance(native) > 0;
        let has_nonce = self.executor.state.get_nonce(native) > 0;
        has_code || has_balance || has_nonce
    }

    fn basic(&self, address: H160) -> Basic {
        let native = sputnik_to_native_address(address);
        // SEC-010: Read from StateStore first (primary), then EvmState cache
        let balance = self.executor.get_account_balance(native);
        let nonce = self.executor.get_account_nonce(native);

        Basic {
            balance: SputnikU256::from(balance),
            nonce: SputnikU256::from(nonce),
        }
    }

    fn code(&self, address: H160) -> Vec<u8> {
        let native = sputnik_to_native_address(address);
        self.executor.get_contract_code(native).unwrap_or_else(|| {
            debug!(
                "No contract code found for {}, defaulting to empty",
                hex::encode(&native)
            );
            Vec::new()
        })
    }

    fn storage(&self, address: H160, index: H256) -> H256 {
        let native_addr = sputnik_to_native_address(address);
        let key = h256_to_bytes32(index);

        if let Some(value) = self.executor.get_contract_storage(native_addr, &key) {
            if value.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&value);
                H256::from(arr)
            } else {
                H256::zero()
            }
        } else {
            H256::zero()
        }
    }

    fn original_storage(&self, address: H160, index: H256) -> Option<H256> {
        // Return the committed (pre-transaction) value for the storage slot.
        // Some(value) tells SputnikVM the original value is known, enabling
        // accurate gas metering for SSTORE (EIP-2200 / EIP-2929).
        // None means the slot was never written, i.e. the original value is zero.
        let native_addr = sputnik_to_native_address(address);
        let key = h256_to_bytes32(index);

        if let Some(value) = self.executor.get_contract_storage(native_addr, &key) {
            if value.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&value);
                return Some(H256::from(arr));
            }
        }
        // Return None to indicate the slot is empty (zero)
        None
    }
}

impl<'a> ApplyBackend for SputnikBackend<'a> {
    fn apply<A, I, L>(&mut self, values: A, logs: L, _delete_empty: bool)
    where
        A: IntoIterator<Item = Apply<I>>,
        I: IntoIterator<Item = (H256, H256)>,
        L: IntoIterator<Item = Log>,
    {
        tracing::info!(target: "evm::sputnik", "ApplyBackend::apply() called - persisting state changes");

        // Capture EVM logs instead of discarding them
        {
            let mut captured = self
                .captured_logs
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for log in logs {
                let topics: Vec<[u8; 32]> = log
                    .topics
                    .into_iter()
                    .map(|t| {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(t.as_bytes());
                        arr
                    })
                    .collect();
                captured.push(EvmLog {
                    address: sputnik_to_native_address(log.address),
                    topics,
                    data: log.data,
                });
            }
        }

        // C8: Collect all persistent sled writes into a single atomic batch.
        // If the node crashes mid-block, either all changes land or none do -
        // no partial state is possible.
        let mut batch: Option<crate::storage::Batch> =
            self.executor.database.as_ref().map(|db| db.begin_batch());

        for apply in values {
            match apply {
                Apply::Modify {
                    address,
                    basic,
                    code,
                    storage,
                    reset_storage: _,
                } => {
                    let native_addr = sputnik_to_native_address(address);

                    // Update balance (in-memory cache AND collect into batch)
                    let balance = basic.balance.as_u128();
                    if balance > 0 || self.executor.state.get_balance(native_addr) > 0 {
                        self.executor.state.set_balance(native_addr, balance);
                        // SEC-010 / C8: Queue balance write in atomic batch
                        if let Some(ref mut b) = batch {
                            b.put_balance(&native_addr, balance);
                        }
                        tracing::debug!(target: "evm::sputnik",
                            address = %hex::encode(&native_addr.0[0..8]),
                            balance = %balance,
                            "Updated balance");
                    }

                    // Update nonce (in-memory cache AND collect into batch)
                    let nonce = basic.nonce.as_u64();
                    if nonce > 0 || self.executor.state.get_nonce(native_addr) > 0 {
                        self.executor.state.set_nonce(native_addr, nonce);
                        // SEC-010 / C8: Queue nonce write in atomic batch
                        if let Some(ref mut b) = batch {
                            b.put_nonce(&native_addr, nonce);
                        }
                    }

                    // Store contract code if changed (in-memory AND batch)
                    if let Some(code_bytes) = code {
                        if !code_bytes.is_empty() {
                            self.executor
                                .state
                                .store_contract(native_addr, code_bytes.clone());
                            let code_len = code_bytes.len();
                            // C8: Queue code write in atomic batch
                            if let Some(ref mut b) = batch {
                                b.put_contract_code(&native_addr, &code_bytes);
                            }
                            tracing::debug!(target: "evm::sputnik",
                                address = %hex::encode(&native_addr.0[0..8]),
                                code_len = code_len,
                                "Stored contract code");
                        }
                    }

                    // Persist storage changes (collect into batch instead of individual writes)
                    for (key, value) in storage {
                        let key_bytes = h256_to_bytes32(key);
                        let value_bytes = h256_to_bytes32(value);

                        tracing::info!(target: "evm::sputnik",
                            address = %hex::encode(&native_addr.0[0..8]),
                            slot = %hex::encode(&key_bytes[0..4]),
                            value = %hex::encode(&value_bytes[0..4]),
                            "Storage change from apply()");

                        // C8: Queue storage write in atomic batch
                        if let Some(ref mut b) = batch {
                            b.put_storage(&native_addr, &key_bytes, &value_bytes);
                        }
                    }

                    tracing::info!(target: "evm::sputnik",
                        address = %hex::encode(&native_addr.0[0..8]),
                        "Applied storage changes via ApplyBackend");
                }
                Apply::Delete { address } => {
                    let native_addr = sputnik_to_native_address(address);
                    tracing::info!(target: "evm::sputnik",
                        address = %hex::encode(&native_addr.0[0..8]),
                        "Deleting account");
                    // Zero out in-memory state
                    self.executor.state.set_balance(native_addr, 0);
                    self.executor.state.set_nonce(native_addr, 0);
                    // C8: Queue zeroed balance + nonce in atomic batch
                    if let Some(ref mut b) = batch {
                        b.put_balance(&native_addr, 0);
                        b.put_nonce(&native_addr, 0);
                    }
                }
            }
        }

        // C8: Commit all queued writes atomically - all-or-nothing
        if let Some(b) = batch {
            if let Some(ref db) = self.executor.database {
                if let Err(err) = db.commit_batch(b) {
                    tracing::error!(target: "evm::sputnik",
                        error = %err,
                        "Atomic batch commit failed - state may be inconsistent");
                } else {
                    tracing::debug!(target: "evm::sputnik", "Atomic batch committed successfully");
                }
            }
        }
    }
}

/// EVM transaction executor
///
/// Production-grade EVM execution engine using SputnikVM.
pub struct EvmTransactionExecutor {
    state: EvmState,
    database: Option<Arc<crate::storage::Database>>,
    /// SEC-013: Ring buffer for last 256 block hashes (shared with SputnikBackend)
    block_hashes: Arc<RwLock<VecDeque<(u64, H256)>>>,
}

impl EvmTransactionExecutor {
    /// Creates a new EVM transaction executor without persistent storage.
    pub fn new() -> Self {
        Self {
            state: EvmState::new(),
            database: None,
            block_hashes: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// Creates an executor with database for persistent storage.
    pub fn with_database(database: Arc<crate::storage::Database>) -> Self {
        Self {
            state: EvmState::new(),
            database: Some(database),
            block_hashes: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// Updates the block hash ring buffer for BLOCKHASH opcode support.
    pub fn update_block_hash(&self, block_number: u64, hash: [u8; 32]) {
        let mut block_hashes = self.block_hashes.write().unwrap_or_else(|e| e.into_inner());
        if block_hashes.len() >= 256 {
            block_hashes.pop_front();
        }
        block_hashes.push_back((block_number, H256::from(hash)));
        tracing::debug!(target: "evm::blockhash",
            block_number = block_number,
            hash = %hex::encode(&hash[..8]),
            "Added block hash to ring buffer");
    }

    /// Execute a transaction using SputnikVM (evm crate)
    ///
    /// This is the PRIMARY execution engine with proper storage exposure
    /// via the ApplyBackend::apply() method.
    fn execute_sputnik(
        &self,
        tx: &Transaction,
        block_number: u64,
        block_timestamp: u64,
        commit: bool,
    ) -> Result<ExecutionResult, String> {
        use evm::executor::stack::{MemoryStackState, StackExecutor, StackSubstateMetadata};
        use evm::Config;

        tracing::debug!(target: "evm::sputnik",
            from = %hex::encode(&tx.from.0[0..8]),
            to = %hex::encode(&tx.to.0[0..8]),
            data_len = tx.data.len(),
            gas_limit = tx.gas_limit,
            "Starting SputnikVM execution");

        // Build the vicinity (block environment)
        let gas_price = if tx.fee > 0 && tx.gas_limit > 0 {
            tx.fee / tx.gas_limit as u128
        } else {
            1u128 // Minimum gas price to avoid division by zero
        };
        let vicinity = SputnikVicinity {
            block_number,
            block_timestamp,
            block_gas_limit: tx.gas_limit,
            block_coinbase: Address::zero(), // Default coinbase
            chain_id: tx.chain_id.unwrap_or(DEFAULT_CHAIN_ID_LOCAL),
            origin: tx.from,
            gas_price,
        };

        // Create the backend
        let mut backend = SputnikBackend::new(self, vicinity);

        // Configure for Shanghai fork (supports PUSH0 opcode from EIP-3855)
        // Verified: SputnikVM 0.41 handles PUSH0 (EIP-3855) correctly.
        // See test_push0_opcode_shanghai in tests/evm_integration.rs for regression guard.
        let config = Config::shanghai();

        // Create substate metadata with gas limit
        let metadata = StackSubstateMetadata::new(tx.gas_limit, &config);

        // Create memory stack state
        let state = MemoryStackState::new(metadata, &mut backend);

        // Create the executor with empty precompiles
        let mut executor = StackExecutor::new_with_precompiles(state, &config, &());

        // Execute based on transaction type
        let (exit_reason, output) = if tx.to == Address::zero() && !tx.data.is_empty() {
            // Contract deployment
            tracing::info!(target: "evm::sputnik", "Contract deployment (CREATE)");

            executor.transact_create(
                native_to_sputnik_address(tx.from),
                SputnikU256::from(tx.value),
                tx.data.clone(),
                tx.gas_limit,
                Vec::new(), // access list
            )
        } else {
            // Contract call
            tracing::info!(target: "evm::sputnik",
                contract = %hex::encode(&tx.to.0[0..8]),
                "Contract call");
            executor.transact_call(
                native_to_sputnik_address(tx.from),
                native_to_sputnik_address(tx.to),
                SputnikU256::from(tx.value),
                tx.data.clone(),
                tx.gas_limit,
                Vec::new(), // access list
            )
        };

        // Handle execution result
        use evm::ExitReason;

        let used_gas = tx.gas_limit.saturating_sub(executor.gas());

        tracing::info!(target: "evm::sputnik",
            exit_reason = ?exit_reason,
            gas_used = used_gas,
            "Execution completed");

        match exit_reason {
            ExitReason::Succeed(success_reason) => {
                tracing::info!(target: "evm::sputnik",
                    success = ?success_reason,
                    gas_used = used_gas,
                    output_len = output.len(),
                    "Execution succeeded");

                // Get created address for contract deployment
                let output_bytes = if tx.to == Address::zero() && !tx.data.is_empty() {
                    // For CREATE, output is the deployed contract address
                    // SEC-008: Use sender's nonce from EVM state, not tx.nonce
                    let sender_nonce = self.get_account_nonce(tx.from);
                    let contract_addr = self.generate_contract_address(tx.from, sender_nonce);
                    tracing::info!(target: "evm::sputnik",
                        contract_address = %hex::encode(&contract_addr.0[0..8]),
                        "Contract created");
                    contract_addr.as_ref().to_vec()
                } else {
                    output.clone()
                };

                // Apply state changes if commit is true
                let evm_logs = if commit {
                    let state = executor.into_state();
                    let (values, logs) = state.deconstruct();

                    // Apply changes to the backend (THIS persists storage AND captures logs)
                    backend.apply(values, logs, true);

                    tracing::info!(target: "evm::sputnik", "State changes committed");
                    backend.take_logs()
                } else {
                    vec![]
                };

                Ok(ExecutionResult {
                    success: true,
                    gas_used: used_gas,
                    output: output_bytes,
                    logs: evm_logs,
                })
            }
            ExitReason::Revert(revert_reason) => {
                tracing::warn!(target: "evm::sputnik",
                    revert = ?revert_reason,
                    gas_used = used_gas,
                    output = %hex::encode(&output),
                    "Execution reverted");

                Err(format!(
                    "SputnikVM reverted ({:?}, gas {}): 0x{}",
                    revert_reason,
                    used_gas,
                    hex::encode(&output)
                ))
            }
            ExitReason::Fatal(fatal_reason) => {
                tracing::error!(target: "evm::sputnik",
                    fatal = ?fatal_reason,
                    "Fatal execution error");

                Err(format!("SputnikVM fatal error: {:?}", fatal_reason))
            }
            ExitReason::Error(error_reason) => {
                tracing::error!(target: "evm::sputnik",
                    error = ?error_reason,
                    "Execution error");

                Err(format!("SputnikVM error: {:?}", error_reason))
            }
        }
    }

    /// Executes a transaction in the EVM using SputnikVM.
    pub fn execute_transaction(
        &self,
        tx: &Transaction,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<ExecutionResult, String> {
        // Contract deployment (to = zero address)
        if tx.to == Address::zero() && !tx.data.is_empty() {
            return self.execute_deployment(tx, block_number, block_timestamp);
        }

        // Contract call with data
        if !tx.data.is_empty() {
            let code = self.get_contract_code(tx.to).unwrap_or_else(|| {
                debug!(
                    "No contract code found for {}, defaulting to empty",
                    hex::encode(tx.to)
                );
                Vec::new()
            });
            if code.is_empty() {
                return Err(format!(
                    "No contract code found at address 0x{}",
                    hex::encode(tx.to)
                ));
            }

            // Use SputnikVM execution
            tracing::debug!(target: "evm", "Using SputnikVM for execution");
            return self.execute_sputnik(tx, block_number, block_timestamp, true);
        }

        // Simple transfer (no data, non-zero value)
        if tx.value > 0 {
            // TODO: Implement native transfer logic
            return Ok(ExecutionResult {
                success: true,
                gas_used: 21000,
                output: vec![],
                logs: vec![],
            });
        }

        Err("Not an EVM transaction".to_string())
    }

    /// Execute contract deployment
    fn execute_deployment(
        &self,
        tx: &Transaction,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<ExecutionResult, String> {
        // Use SputnikVM for deployment (proper storage exposure)
        tracing::debug!(target: "evm", "Using SputnikVM for deployment");
        match self.execute_sputnik(tx, block_number, block_timestamp, true) {
            Ok(result) => {
                tracing::info!(target: "evm::sputnik", "SputnikVM deployment succeeded");
                return Ok(result);
            }
            Err(err) => {
                tracing::warn!(target: "evm",
                    error = %err,
                    "SputnikVM deployment failed");
                return Err(err);
            }
        }
    }

    /// Executes a read-only contract call (eth_call style) without committing state.
    pub fn execute_readonly(
        &self,
        tx: &Transaction,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<ExecutionResult, String> {
        if tx.data.is_empty() {
            return Err("Not an EVM call".to_string());
        }
        if tx.to != Address::zero() {
            let code = self.get_contract_code(tx.to).unwrap_or_else(|| {
                debug!(
                    "No contract code found for {}, defaulting to empty",
                    hex::encode(tx.to)
                );
                Vec::new()
            });
            if code.is_empty() {
                return Err(format!(
                    "No contract code found at address 0x{}",
                    hex::encode(tx.to)
                ));
            }
        }
        self.execute_sputnik(tx, block_number, block_timestamp, false)
    }

    /// Generates a contract address from sender and nonce (Ethereum CREATE opcode).
    pub fn generate_contract_address(&self, sender: Address, nonce: u64) -> Address {
        use alloy_rlp::{BufMut, BytesMut, Encodable};

        // Ethereum-style address derivation: keccak256(rlp([sender, nonce]))[12..]
        // We need to encode as an RLP list, not just concatenated items.

        // First, encode each item to get their lengths
        let mut sender_buf = BytesMut::new();
        sender.as_ref().encode(&mut sender_buf);
        let sender_encoded = sender_buf.freeze();

        let mut nonce_buf = BytesMut::new();
        nonce.encode(&mut nonce_buf);
        let nonce_encoded = nonce_buf.freeze();

        // Calculate total payload length (sender encoding + nonce encoding)
        let payload_len = sender_encoded.len() + nonce_encoded.len();

        // Create the list: list prefix + payload
        let mut buf = BytesMut::new();

        // Write RLP list prefix
        if payload_len < 56 {
            // Short list: 0xc0 + length
            buf.put_u8(0xc0 + payload_len as u8);
        } else {
            // Long list: 0xf7 + length of length + length
            let len_bytes = payload_len.to_be_bytes();
            let leading_zeros = len_bytes.iter().take_while(|&&b| b == 0).count();
            let significant_bytes = &len_bytes[leading_zeros..];

            buf.put_u8(0xf7 + significant_bytes.len() as u8);
            buf.put_slice(significant_bytes);
        }

        // Append the encoded items
        buf.put_slice(&sender_encoded);
        buf.put_slice(&nonce_encoded);

        let hash = keccak256(&buf[..]);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&hash.as_ref()[12..32]);
        Address(addr)
    }

    /// Deploys a contract to the EVM.
    pub fn deploy_contract(
        &self,
        from: Address,
        code: Vec<u8>,
        value: u128,
        gas_limit: u64,
        nonce: u64,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<(Address, ExecutionResult), String> {
        // Create deployment transaction
        let tx = Transaction::with_data(
            from,
            Address::zero(), // Zero address for deployment
            value,
            0, // Fee will be calculated
            nonce,
            code,
            gas_limit,
        );

        // Execute deployment
        let result = self.execute_transaction(&tx, block_number, block_timestamp)?;

        // Extract contract address from output (first 20 bytes)
        let contract_address = if result.output.len() >= 20 {
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&result.output[..20]);
            Address(addr)
        } else {
            // Generate from transaction
            self.generate_contract_address(from, nonce)
        };

        Ok((contract_address, result))
    }

    /// Calls a contract with the given data.
    pub fn call_contract(
        &self,
        from: Address,
        to: Address,
        data: Vec<u8>,
        value: u128,
        gas_limit: u64,
        block_number: u64,
        block_timestamp: u64,
    ) -> Result<ExecutionResult, String> {
        // Create call transaction
        let tx = Transaction::with_data(
            from, to, value, 0, // Fee will be calculated
            0, // Nonce will be handled by caller
            data, gas_limit,
        );

        // Execute call
        self.execute_transaction(&tx, block_number, block_timestamp)
    }

    /// Returns the contract code, checking database first then memory.
    pub fn get_contract_code(&self, address: Address) -> Option<Vec<u8>> {
        // Try database first (persistent storage)
        if let Some(ref db) = self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(code)) = state_store.get_contract_code(&address) {
                return Some(code);
            }
        }

        // Fall back to memory state
        self.state.get_contract_code(address)
    }

    /// Returns the account nonce, checking database first then memory.
    pub fn get_account_nonce(&self, address: Address) -> u64 {
        if let Some(ref db) = self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(nonce)) = state_store.get_nonce(&address) {
                return nonce;
            }
        }

        self.state.get_nonce(address)
    }

    /// Returns the account balance, checking database first then memory.
    pub fn get_account_balance(&self, address: Address) -> u128 {
        if let Some(ref db) = self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(balance)) = state_store.get_balance(&address) {
                return balance;
            }
        }

        self.state.get_balance(address)
    }

    /// Returns a reference to the in-memory EVM state.
    pub fn state(&self) -> &EvmState {
        &self.state
    }

    /// Executes a raw SputnikVM call for testing/debugging purposes.
    pub fn execute_sputnik_raw(
        &self,
        caller: Address,
        target: Address,
        data: Vec<u8>,
        gas_limit: u64,
        block_number: u64,
        block_timestamp: u64,
        commit: bool,
    ) -> Result<ExecutionResult, String> {
        // Create a transaction-like struct for execution
        let tx = Transaction::with_data(
            caller, target, 0, // value
            0, // fee
            0, // nonce
            data, gas_limit,
        );
        self.execute_sputnik(&tx, block_number, block_timestamp, commit)
    }

    /// Returns the contract storage value for the given key.
    pub fn get_contract_storage(&self, address: Address, key: &[u8]) -> Option<Vec<u8>> {
        // Try database first (persistent storage)
        if let Some(ref db) = self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            if let Ok(Some(value)) = state_store.get_contract_storage(&address, key) {
                return Some(value);
            }
        }

        // Fall back to memory state (currently not implemented)
        None
    }

    /// Sets the contract storage value for the given key.
    pub fn set_contract_storage(
        &self,
        address: Address,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), String> {
        let key_prefix_len = 4.min(key.len());
        tracing::debug!(target: "evm::storage",
            address = %hex::encode(&address.0[0..8]),
            key = %hex::encode(&key[0..key_prefix_len]),
            value_len = value.len(),
            "Setting contract storage");

        // Persist to database
        if let Some(ref db) = self.database {
            use crate::storage::StateStore;
            let state_store = StateStore::new(db);
            state_store
                .put_contract_storage(&address, key, value)
                .map_err(|e| format!("Failed to store contract storage: {:?}", e))
        } else {
            Err("No database available for storage persistence".to_string())
        }
    }
}

/// An EVM event log entry, capturing the contract address, indexed topics, and data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmLog {
    /// Address of the contract that emitted the log.
    pub address: Address,
    /// List of indexed topics (up to 4, each 32 bytes). Topic 0 is typically the event signature hash.
    pub topics: Vec<[u8; 32]>,
    /// Non-indexed log data (ABI-encoded).
    pub data: Vec<u8>,
}

/// Execution result from EVM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Whether execution was successful
    pub success: bool,
    /// Gas used during execution
    pub gas_used: u64,
    /// Output data (return value or contract address)
    pub output: Vec<u8>,
    /// EVM event logs emitted during execution
    pub logs: Vec<EvmLog>,
}

// Parallel EVM module
pub mod parallel;

// State snapshot module for parallel execution
pub mod state_snapshot;

// Benchmarking module
pub mod benchmark;

// Integration helpers
pub mod integration;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;
    use tempfile::TempDir;

    /// TST-003: Test EVM state persistence across executor restart
    ///
    /// This test verifies that account balances and nonces persist to the
    /// sled database and can be retrieved after dropping and recreating
    /// the EvmTransactionExecutor.
    #[test]
    fn test_evm_state_persistence_across_restart() {
        // 1. Create a temp directory for sled DB
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let db_path = temp_dir.path().join("evm_test.db");

        // Test account address
        let test_address = Address([0xABu8; 20]);
        let expected_balance: u128 = 1_000_000_000_000_000_000; // 1 ETH in wei
        let expected_nonce: u64 = 42;

        // 2. Create EvmTransactionExecutor with that DB path
        let db = Arc::new(Database::open(&db_path).expect("Failed to open database"));
        let executor = EvmTransactionExecutor::with_database(db);

        // 3. Fund an account (set balance via state)
        executor.state.set_balance(test_address, expected_balance);
        executor.state.set_nonce(test_address, expected_nonce);

        // Also persist to database via StateStore (simulating what ApplyBackend does)
        {
            use crate::storage::StateStore;
            let state_store = StateStore::new(&executor.database.as_ref().unwrap());
            state_store
                .put_balance(&test_address, expected_balance)
                .expect("Failed to persist balance");
            state_store
                .put_nonce(&test_address, expected_nonce)
                .expect("Failed to persist nonce");
        }

        // 4. Verify balance is set in memory
        let balance_before = executor.state.get_balance(test_address);
        let nonce_before = executor.state.get_nonce(test_address);
        assert_eq!(
            balance_before, expected_balance,
            "Balance not set correctly in memory"
        );
        assert_eq!(
            nonce_before, expected_nonce,
            "Nonce not set correctly in memory"
        );

        // Verify database read works
        {
            use crate::storage::StateStore;
            let state_store = StateStore::new(&executor.database.as_ref().unwrap());
            let db_balance = state_store
                .get_balance(&test_address)
                .expect("Failed to read balance from DB")
                .expect("Balance not found in DB");
            let db_nonce = state_store
                .get_nonce(&test_address)
                .expect("Failed to read nonce from DB")
                .expect("Nonce not found in DB");
            assert_eq!(
                db_balance, expected_balance,
                "Balance not persisted to DB correctly"
            );
            assert_eq!(
                db_nonce, expected_nonce,
                "Nonce not persisted to DB correctly"
            );
        }

        // 5. Drop the executor (releases database reference)
        drop(executor);

        // 6. Create a NEW executor from the same DB path
        let db2 = Arc::new(Database::open(&db_path).expect("Failed to reopen database"));
        let executor2 = EvmTransactionExecutor::with_database(db2);

        // 7. Verify the balance persists (read from database via get_account_balance)
        let balance_after = executor2.get_account_balance(test_address);
        let nonce_after = executor2.get_account_nonce(test_address);
        assert_eq!(
            balance_after, expected_balance,
            "Balance did not persist across restart"
        );
        assert_eq!(
            nonce_after, expected_nonce,
            "Nonce did not persist across restart"
        );
    }
}
