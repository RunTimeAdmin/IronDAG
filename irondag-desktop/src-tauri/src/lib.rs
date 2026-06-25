use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::collections::{HashMap, VecDeque};
use std::process::{Child, Command, Stdio};
use std::time::Instant;
use std::fs;
use std::path::PathBuf;
use std::io::{BufRead, BufReader};
use std::thread;
use tauri::State;
use zeroize::Zeroize;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use aes_gcm::aead::Aead;
use argon2::{Algorithm, Argon2, Params, Version};

// ----------------------------------------------------------------------------
// Key Management (simple in-memory keystore for MVP)
// ----------------------------------------------------------------------------

struct KeyStore {
    secret_key: Option<[u8; 32]>,
    /// Derived AES key for encrypting data files (from keystore password)
    encryption_key: Option<[u8; 32]>,
}

impl KeyStore {
    fn new() -> Self {
        Self { secret_key: None, encryption_key: None }
    }

    fn set_key(&mut self, key: [u8; 32]) {
        self.secret_key = Some(key);
    }

    fn get_key(&self) -> Option<[u8; 32]> {
        self.secret_key
    }

    fn has_key(&self) -> bool {
        self.secret_key.is_some()
    }

    fn set_encryption_key(&mut self, key: [u8; 32]) {
        self.encryption_key = Some(key);
    }

    fn get_encryption_key(&self) -> Option<[u8; 32]> {
        self.encryption_key
    }

    #[allow(dead_code)]
    // Reserved for future keystore UI integration
    fn is_unlocked(&self) -> bool {
        self.encryption_key.is_some()
    }

    fn clear_key(&mut self) {
        if let Some(ref mut key) = self.secret_key {
            key.zeroize();
        }
        self.secret_key = None;
    }

    fn clear_encryption_key(&mut self) {
        if let Some(ref mut key) = self.encryption_key {
            key.zeroize();
        }
        self.encryption_key = None;
    }

    /// Derive address from secret key (simplified: use public key hash)
    fn get_address(&self) -> Option<[u8; 20]> {
        use ed25519_dalek::SigningKey;
        use sha3::{Digest, Keccak256};

        let secret = self.secret_key?;
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        let pub_bytes = verifying_key.to_bytes();

        // Hash public key with Keccak256 and take last 20 bytes as address
        let mut hasher = Keccak256::new();
        hasher.update(&pub_bytes);
        let result = hasher.finalize();
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&result[12..32]);
        Some(addr)
    }
}

// ----------------------------------------------------------------------------
// Encrypted Keystore File Format
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct EncryptedKeystore {
    salt: String,      // hex-encoded 16-byte salt
    nonce: String,     // hex-encoded 12-byte nonce
    ciphertext: String, // hex-encoded encrypted private key
}

// ----------------------------------------------------------------------------
// Encryption Helper Functions
// ----------------------------------------------------------------------------

/// Get the data directory for storing keystore and data files
fn get_data_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Could not determine data directory".to_string())?;
    let app_dir = data_dir.join("irondag");
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("Failed to create data directory: {}", e))?;
    Ok(app_dir)
}

/// Get the keystore file path
fn get_keystore_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join("keystore.enc"))
}

/// Derive a 32-byte AES key from password using Argon2id
fn derive_key_from_password(password: &str, salt: &[u8; 16]) -> Result<[u8; 32], String> {
    let params = Params::new(19456, 2, 1, Some(32))
        .map_err(|e| format!("Failed to create Argon2 params: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    
    let mut key = [0u8; 32];
    argon2.hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Key derivation failed: {}", e))?;
    Ok(key)
}

/// Encrypt data using AES-256-GCM
fn encrypt_data(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12]), String> {
    use rand::Rng;
    
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Failed to create cipher: {}", e))?;
    
    let mut rng = rand::thread_rng();
    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes);
    
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {}", e))?;
    
    Ok((ciphertext, nonce_bytes))
}

/// Decrypt data using AES-256-GCM
fn decrypt_data(key: &[u8; 32], nonce: &[u8; 12], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Failed to create cipher: {}", e))?;
    
    let nonce = Nonce::from_slice(nonce);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;
    
    Ok(plaintext)
}

// ----------------------------------------------------------------------------
// Address Book
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Contact {
    name: String,
    address: String,
    notes: Option<String>,
}

/// Encrypted file format for address book and accounts
#[derive(Serialize, Deserialize)]
struct EncryptedFile {
    nonce: String,     // hex-encoded 12-byte nonce
    ciphertext: String, // hex-encoded encrypted JSON
}

struct AddressBook {
    contacts: HashMap<String, Contact>, // key = address
    storage_path: PathBuf,
}

impl AddressBook {
    fn new(storage_path: PathBuf) -> Self {
        let mut book = Self {
            contacts: HashMap::new(),
            storage_path,
        };
        book.load(None);
        book
    }

    fn load(&mut self, encryption_key: Option<[u8; 32]>) {
        if let Ok(data) = fs::read_to_string(&self.storage_path) {
            // Try to parse as encrypted file first
            if let Ok(encrypted) = serde_json::from_str::<EncryptedFile>(&data) {
                if let Some(key) = encryption_key {
                    // Decrypt the data
                    if let Ok(nonce_bytes) = hex::decode(&encrypted.nonce)
                        .and_then(|n| n.try_into().map_err(|_| hex::FromHexError::InvalidStringLength))
                    {
                        let nonce: [u8; 12] = nonce_bytes;
                        if let Ok(ciphertext) = hex::decode(&encrypted.ciphertext) {
                            if let Ok(plaintext) = decrypt_data(&key, &nonce, &ciphertext) {
                                if let Ok(contacts) = serde_json::from_slice(&plaintext) {
                                    self.contacts = contacts;
                                }
                            }
                        }
                    }
                }
                // If decryption failed, contacts remain empty
            } else {
                // Try to parse as plain JSON (backward compatibility)
                if let Ok(contacts) = serde_json::from_str(&data) {
                    self.contacts = contacts;
                }
            }
        }
    }

    fn save(&self, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        let data = serde_json::to_string_pretty(&self.contacts)
            .map_err(|e| format!("Serialize error: {}", e))?;
        
        if let Some(key) = encryption_key {
            // Encrypt the data
            let (ciphertext, nonce) = encrypt_data(&key, data.as_bytes())?;
            let encrypted = EncryptedFile {
                nonce: hex::encode(nonce),
                ciphertext: hex::encode(&ciphertext),
            };
            let json = serde_json::to_string_pretty(&encrypted)
                .map_err(|e| format!("Failed to serialize encrypted file: {}", e))?;
            fs::write(&self.storage_path, json)
                .map_err(|e| format!("Write error: {}", e))?;
        } else {
            // Save as plain JSON
            fs::write(&self.storage_path, data)
                .map_err(|e| format!("Write error: {}", e))?;
        }
        Ok(())
    }

    fn add_contact(&mut self, contact: Contact, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        self.contacts.insert(contact.address.clone(), contact);
        self.save(encryption_key)
    }

    fn remove_contact(&mut self, address: &str, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        self.contacts.remove(address);
        self.save(encryption_key)
    }

    fn get_contacts(&self) -> Vec<Contact> {
        self.contacts.values().cloned().collect()
    }
}

// ----------------------------------------------------------------------------
// Multi-Account Wallet
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Account {
    name: String,
    address: String,
    // Note: We don't store private keys, only addresses. Keys stay in KeyStore.
}

struct Accounts {
    accounts: Vec<Account>,
    storage_path: PathBuf,
}

impl Accounts {
    fn new(storage_path: PathBuf) -> Self {
        let mut accts = Self {
            accounts: Vec::new(),
            storage_path,
        };
        accts.load(None);
        accts
    }

    fn load(&mut self, encryption_key: Option<[u8; 32]>) {
        if let Ok(data) = fs::read_to_string(&self.storage_path) {
            // Try to parse as encrypted file first
            if let Ok(encrypted) = serde_json::from_str::<EncryptedFile>(&data) {
                if let Some(key) = encryption_key {
                    // Decrypt the data
                    if let Ok(nonce_bytes) = hex::decode(&encrypted.nonce)
                        .and_then(|n| n.try_into().map_err(|_| hex::FromHexError::InvalidStringLength))
                    {
                        let nonce: [u8; 12] = nonce_bytes;
                        if let Ok(ciphertext) = hex::decode(&encrypted.ciphertext) {
                            if let Ok(plaintext) = decrypt_data(&key, &nonce, &ciphertext) {
                                if let Ok(accounts) = serde_json::from_slice(&plaintext) {
                                    self.accounts = accounts;
                                }
                            }
                        }
                    }
                }
                // If decryption failed, accounts remain empty
            } else {
                // Try to parse as plain JSON (backward compatibility)
                if let Ok(accounts) = serde_json::from_str(&data) {
                    self.accounts = accounts;
                }
            }
        }
    }

    fn save(&self, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        let data = serde_json::to_string_pretty(&self.accounts)
            .map_err(|e| format!("Serialize error: {}", e))?;
        
        if let Some(key) = encryption_key {
            // Encrypt the data
            let (ciphertext, nonce) = encrypt_data(&key, data.as_bytes())?;
            let encrypted = EncryptedFile {
                nonce: hex::encode(nonce),
                ciphertext: hex::encode(&ciphertext),
            };
            let json = serde_json::to_string_pretty(&encrypted)
                .map_err(|e| format!("Failed to serialize encrypted file: {}", e))?;
            fs::write(&self.storage_path, json)
                .map_err(|e| format!("Write error: {}", e))?;
        } else {
            // Save as plain JSON
            fs::write(&self.storage_path, data)
                .map_err(|e| format!("Write error: {}", e))?;
        }
        Ok(())
    }

    fn add_account(&mut self, account: Account, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        // Check for duplicates
        if !self.accounts.iter().any(|a| a.address == account.address) {
            self.accounts.push(account);
            self.save(encryption_key)?;
        }
        Ok(())
    }

    fn remove_account(&mut self, address: &str, encryption_key: Option<[u8; 32]>) -> Result<(), String> {
        self.accounts.retain(|a| a.address != address);
        self.save(encryption_key)
    }

    fn get_accounts(&self) -> Vec<Account> {
        self.accounts.clone()
    }
}

// ----------------------------------------------------------------------------
// RPC Configuration
// ----------------------------------------------------------------------------

#[derive(Clone)]
struct RpcConfig {
    url: Arc<Mutex<String>>,
    api_key: Option<String>,
}

// ----------------------------------------------------------------------------
// Node Process Management
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct NodeProcessInfo {
    id: String,
    pid: u32,
    p2p_port: u16,
    rpc_port: u16,
    http_port: Option<u16>,
    data_dir: Option<String>,
    enable_mining: bool,
    peers: Vec<String>,
    log_streaming: bool,
    exit_code: Option<i32>,
}

struct LogBuffer {
    enabled: bool,
    max_lines: usize,
    lines: VecDeque<String>,
}

impl LogBuffer {
    fn new(enabled: bool, max_lines: usize) -> Self {
        Self {
            enabled,
            max_lines,
            lines: VecDeque::new(),
        }
    }

    fn push_line(&mut self, line: String) {
        if !self.enabled {
            return;
        }
        if self.lines.len() >= self.max_lines {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

struct NodeProcessEntry {
    child: Child,
    info: NodeProcessInfo,
    log_buffer: Arc<Mutex<LogBuffer>>,
}

struct NodeProcessManager {
    processes: HashMap<String, NodeProcessEntry>,
}

impl NodeProcessManager {
    fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }
}

fn spawn_log_reader<R: std::io::Read + Send + 'static>(
    reader: R,
    log_buffer: Arc<Mutex<LogBuffer>>,
    prefix: &'static str,
) {
    thread::spawn(move || {
        let buf_reader = BufReader::new(reader);
        for line in buf_reader.lines().flatten() {
            if let Ok(mut buffer) = log_buffer.lock() {
                buffer.push_line(format!("[{}] {}", prefix, line));
            }
        }
    });
}

fn find_repo_root() -> Result<PathBuf, String> {
    let mut current = std::env::current_dir().map_err(|e| e.to_string())?;
    for _ in 0..6 {
        if current.join("irondag-blockchain").exists() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    Err("Could not find repo root containing irondag-blockchain".to_string())
}

fn resolve_node_binary(repo_root: &PathBuf, binary_path: Option<String>) -> Result<PathBuf, String> {
    if let Some(path) = binary_path {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
        return Err("Provided node binary path does not exist".to_string());
    }

    let release = if cfg!(windows) {
        repo_root.join("irondag-blockchain").join("target").join("release").join("node.exe")
    } else {
        repo_root.join("irondag-blockchain").join("target").join("release").join("node")
    };
    if release.exists() {
        return Ok(release);
    }

    let debug = if cfg!(windows) {
        repo_root.join("irondag-blockchain").join("target").join("debug").join("node.exe")
    } else {
        repo_root.join("irondag-blockchain").join("target").join("debug").join("node")
    };
    if debug.exists() {
        return Ok(debug);
    }

    Err("Node binary not found. Build irondag-blockchain first.".to_string())
}

fn resolve_data_dir(repo_root: &PathBuf, data_dir: &str) -> PathBuf {
    let candidate = PathBuf::from(data_dir);
    if candidate.is_absolute() {
        candidate
    } else {
        repo_root.join("irondag-blockchain").join(candidate)
    }
}

#[tauri::command]
fn reset_data_dir(data_dir: String) -> Result<(), String> {
    let repo_root = find_repo_root()?;
    let resolved = resolve_data_dir(&repo_root, &data_dir);
    if resolved.exists() {
        fs::remove_dir_all(&resolved).map_err(|e| format!("Failed to remove data dir: {}", e))?;
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Test Runner
// ----------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct TestDefinition {
    name: String,
    label: String,
    description: String,
}

struct TestDefinitionInternal {
    name: String,
    label: String,
    description: String,
    script_path: PathBuf,
    working_dir: PathBuf,
}

#[derive(Serialize)]
struct TestResult {
    name: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    duration_ms: u128,
}

fn available_tests(repo_root: &PathBuf) -> Vec<TestDefinitionInternal> {
    let base = repo_root.join("irondag-blockchain");
    let tests = vec![
        ("rpc", "RPC Test", "Basic JSON-RPC connectivity checks", base.join("test_rpc.ps1")),
        ("tx_flow", "Transaction Flow", "End-to-end transaction flow test", base.join("test_transaction_flow.ps1")),
        ("tx_send", "Transaction Sending", "Send transactions via RPC", base.join("test_transaction_sending.ps1")),
        ("tx_batch", "Transaction Batch", "Run batch transaction tests", base.join("test_transactions.ps1")),
        ("metamask", "MetaMask Connection", "Validate MetaMask connectivity", base.join("test_metamask_connection.ps1")),
        ("contract_deploy", "Contract Deployment", "Deploy a sample contract", base.join("test_contract_deployment.ps1")),
        ("ecdsa_signature", "ECDSA Signature", "Verify ECDSA signature flow", base.join("test_ecdsa_signature.ps1")),
        ("crash_test", "Crash Test", "Node crash resilience test", base.join("crash_test.ps1")),
        ("persistence", "Persistence Check", "Verify block/state persistence", base.join("check_persistence.ps1")),
    ];

    tests
        .into_iter()
        .filter(|(_, _, _, script)| script.exists())
        .map(|(name, label, description, script_path)| TestDefinitionInternal {
            name: name.to_string(),
            label: label.to_string(),
            description: description.to_string(),
            script_path,
            working_dir: base.clone(),
        })
        .collect()
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    #[allow(dead_code)]
    id: Value,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

async fn call_rpc(
    cfg: &RpcConfig,
    method: &str,
    params: Option<Value>,
) -> Result<Value, String> {
    use std::time::Duration;
    
    // Get current URL from config
    let url = cfg.url.lock().map_err(|e| e.to_string())?.clone();
    
    // Create client with timeout to prevent hanging
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;
    
    let req = JsonRpcRequest {
        jsonrpc: "2.0",
        method,
        params,
        id: 1,
    };

    let mut http_req = client.post(&url).json(&req);
    if let Some(ref key) = cfg.api_key {
        http_req = http_req.header("X-API-Key", key);
    }

    let resp = http_req.send().await.map_err(|e| {
        if e.is_timeout() {
            "Connection timeout - node may not be running".to_string()
        } else if e.is_connect() {
            "Cannot connect to node - ensure it's running".to_string()
        } else {
            e.to_string()
        }
    })?;
    let body: JsonRpcResponse = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = body.error {
        return Err(format!("RPC error {}: {}", err.code, err.message));
    }

    Ok(body.result.unwrap_or(Value::Null))
}

// ----------------------------------------------------------------------------
// Input Validation Helpers
// ----------------------------------------------------------------------------

fn validate_address(addr: &str) -> Result<String, String> {
    let addr = addr.strip_prefix("0x").unwrap_or(addr);
    if addr.len() != 40 {
        return Err(format!("Invalid address: expected 40 hex characters, got {}", addr.len()));
    }
    if !addr.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid address: contains non-hex characters".to_string());
    }
    Ok(format!("0x{}", addr.to_lowercase()))
}

fn validate_private_key(key: &str) -> Result<Vec<u8>, String> {
    let key = key.strip_prefix("0x").unwrap_or(key);
    if key.len() != 64 {
        return Err(format!("Invalid private key: expected 64 hex characters, got {}", key.len()));
    }
    hex::decode(key).map_err(|e| format!("Invalid private key hex: {}", e))
}

#[allow(dead_code)]
// Reserved for future amount validation in send flow
fn validate_amount(amount: &str) -> Result<u64, String> {
    amount.parse::<u64>().map_err(|_| format!("Invalid amount '{}': must be a positive integer", amount))
}

// ----------------------------------------------------------------------------
// Tauri Commands
// ----------------------------------------------------------------------------

#[tauri::command]
async fn get_node_status(state: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&state, "irondag_getNodeStatus", None).await
}

#[tauri::command]
fn get_rpc_url(state: State<'_, RpcConfig>) -> Result<String, String> {
    let url = state.url.lock().map_err(|e| e.to_string())?;
    Ok(url.clone())
}

#[tauri::command]
fn set_rpc_url(state: State<'_, RpcConfig>, new_url: String) -> Result<(), String> {
    let mut url = state.url.lock().map_err(|e| e.to_string())?;
    *url = new_url.clone();
    println!("✅ RPC URL updated to: {}", new_url);
    Ok(())
}

#[tauri::command]
fn get_node_processes(manager: State<'_, Arc<Mutex<NodeProcessManager>>>) -> Result<Vec<NodeProcessInfo>, String> {
    let mut manager = manager.lock().map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let mut finished = Vec::new();

    for (id, entry) in manager.processes.iter_mut() {
        match entry.child.try_wait() {
            Ok(Some(status)) => {
                entry.info.exit_code = status.code();
                if let Ok(buffer) = entry.log_buffer.lock() {
                    entry.info.log_streaming = buffer.enabled;
                }
                results.push(entry.info.clone());
                finished.push(id.clone());
            }
            Ok(None) => {
                entry.info.exit_code = None;
                if let Ok(buffer) = entry.log_buffer.lock() {
                    entry.info.log_streaming = buffer.enabled;
                }
                results.push(entry.info.clone());
            }
            Err(e) => {
                return Err(format!("Failed to query process {}: {}", id, e));
            }
        }
    }

    for id in finished {
        manager.processes.remove(&id);
    }

    Ok(results)
}

#[tauri::command]
fn start_node(
    manager: State<'_, Arc<Mutex<NodeProcessManager>>>,
    rpc_config: State<'_, RpcConfig>,
    node_id: String,
    p2p_port: u16,
    rpc_port: u16,
    http_port: Option<u16>,
    data_dir: Option<String>,
    enable_mining: bool,
    peers: Option<Vec<String>>,
    log_streaming: bool,
    binary_path: Option<String>,
    no_test_txs: Option<bool>,
) -> Result<NodeProcessInfo, String> {
    let repo_root = find_repo_root()?;
    let binary = resolve_node_binary(&repo_root, binary_path)?;

    let mut manager = manager.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = manager.processes.get_mut(&node_id) {
        if let Ok(None) = existing.child.try_wait() {
            return Err(format!("Node {} is already running", node_id));
        }
        manager.processes.remove(&node_id);
    }

    let mut command = Command::new(binary);
    
    // Node expects positional args: [p2p_port] [rpc_port] [http_api_port]
    command
        .arg(p2p_port.to_string())
        .arg(rpc_port.to_string());

    // HTTP port is optional 3rd positional arg
    if let Some(http) = http_port {
        command.arg(http.to_string());
    }

    if let Some(ref dir) = data_dir {
        let resolved = resolve_data_dir(&repo_root, dir);
        command.arg("--data-dir").arg(resolved);
    }

    // Mining is enabled by default in the node; only add flag to disable
    if !enable_mining {
        command.arg("--disable-mining");
    }

    if no_test_txs.unwrap_or(false) {
        command.arg("--no-test-txs");
    }

    let mut peer_list = Vec::new();
    if let Some(peers) = peers {
        for peer in peers {
            let trimmed = peer.trim();
            if !trimmed.is_empty() {
                peer_list.push(trimmed.to_string());
                command.arg(trimmed);
            }
        }
    }

    command
        .current_dir(&repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| format!("Failed to start node: {}", e))?;
    let pid = child.id();
    let log_buffer = Arc::new(Mutex::new(LogBuffer::new(log_streaming, 2000)));

    if let Some(stdout) = child.stdout.take() {
        spawn_log_reader(stdout, log_buffer.clone(), "stdout");
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_reader(stderr, log_buffer.clone(), "stderr");
    }

    let info = NodeProcessInfo {
        id: node_id.clone(),
        pid,
        p2p_port,
        rpc_port,
        http_port,
        data_dir,
        enable_mining,
        peers: peer_list,
        log_streaming,
        exit_code: None,
    };

    manager.processes.insert(
        node_id,
        NodeProcessEntry {
            child,
            info: info.clone(),
            log_buffer,
        },
    );
    
    // Update RPC config to point to the started node
    if let Ok(mut url) = rpc_config.url.lock() {
        *url = format!("http://127.0.0.1:{}", rpc_port);
        println!("✅ Updated desktop app RPC URL to: {}", *url);
    }

    Ok(info)
}

#[tauri::command]
fn stop_node(manager: State<'_, Arc<Mutex<NodeProcessManager>>>, node_id: String) -> Result<(), String> {
    let mut manager = manager.lock().map_err(|e| e.to_string())?;
    let mut entry = manager
        .processes
        .remove(&node_id)
        .ok_or_else(|| format!("Node {} is not running", node_id))?;

    entry.child.kill().map_err(|e| e.to_string())?;
    let _ = entry.child.wait();
    Ok(())
}

#[tauri::command]
fn stop_all_nodes(manager: State<'_, Arc<Mutex<NodeProcessManager>>>) -> Result<(), String> {
    let mut manager = manager.lock().map_err(|e| e.to_string())?;
    let keys: Vec<String> = manager.processes.keys().cloned().collect();
    for key in keys {
        if let Some(mut entry) = manager.processes.remove(&key) {
            let _ = entry.child.kill();
            let _ = entry.child.wait();
        }
    }
    Ok(())
}

#[tauri::command]
fn set_node_log_streaming(
    manager: State<'_, Arc<Mutex<NodeProcessManager>>>,
    node_id: String,
    enabled: bool,
) -> Result<(), String> {
    let manager = manager.lock().map_err(|e| e.to_string())?;
    let entry = manager
        .processes
        .get(&node_id)
        .ok_or_else(|| format!("Node {} is not running", node_id))?;
    let mut buffer = entry.log_buffer.lock().map_err(|e| e.to_string())?;
    buffer.enabled = enabled;
    Ok(())
}

#[tauri::command]
fn get_node_logs(
    manager: State<'_, Arc<Mutex<NodeProcessManager>>>,
    node_id: String,
    max_lines: Option<usize>,
) -> Result<Vec<String>, String> {
    let manager = manager.lock().map_err(|e| e.to_string())?;
    let entry = manager
        .processes
        .get(&node_id)
        .ok_or_else(|| format!("Node {} is not running", node_id))?;
    let buffer = entry.log_buffer.lock().map_err(|e| e.to_string())?;
    let take = max_lines.unwrap_or(200).min(buffer.lines.len());
    Ok(buffer.lines.iter().rev().take(take).cloned().collect())
}

#[tauri::command]
fn clear_node_logs(
    manager: State<'_, Arc<Mutex<NodeProcessManager>>>,
    node_id: String,
) -> Result<(), String> {
    let manager = manager.lock().map_err(|e| e.to_string())?;
    let entry = manager
        .processes
        .get(&node_id)
        .ok_or_else(|| format!("Node {} is not running", node_id))?;
    let mut buffer = entry.log_buffer.lock().map_err(|e| e.to_string())?;
    buffer.lines.clear();
    Ok(())
}

#[tauri::command]
fn list_tests() -> Result<Vec<TestDefinition>, String> {
    let repo_root = find_repo_root()?;
    let tests = available_tests(&repo_root);
    Ok(tests
        .into_iter()
        .map(|t| TestDefinition {
            name: t.name,
            label: t.label,
            description: t.description,
        })
        .collect())
}

#[tauri::command]
async fn run_test(name: String) -> Result<TestResult, String> {
    let repo_root = find_repo_root()?;
    let tests = available_tests(&repo_root);
    let test = tests
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| "Unknown test name".to_string())?;

    let script_path = test.script_path;
    let working_dir = test.working_dir;
    let name_clone = test.name.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<TestResult, String> {
        let start = Instant::now();
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script_path)
            .current_dir(&working_dir)
            .output()
            .map_err(|e| e.to_string())?;

        Ok(TestResult {
            name: name_clone,
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration_ms: start.elapsed().as_millis(),
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(result)
}

#[tauri::command]
async fn get_mining_status(state: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&state, "irondag_getMiningStatus", None).await
}

#[tauri::command]
async fn start_mining(state: State<'_, RpcConfig>) -> Result<(), String> {
    let _ = call_rpc(&state, "irondag_startMining", None).await?;
    Ok(())
}

#[tauri::command]
async fn stop_mining(state: State<'_, RpcConfig>) -> Result<(), String> {
    let _ = call_rpc(&state, "irondag_stopMining", None).await?;
    Ok(())
}

#[tauri::command]
async fn get_balance(state: State<'_, RpcConfig>, address: String) -> Result<String, String> {
    validate_address(&address)?;
    let params = Some(serde_json::json!([address, "latest"]));
    let result = call_rpc(&state, "eth_getBalance", params).await?;
    if let Some(balance_str) = result.as_str() {
        Ok(balance_str.to_string())
    } else {
        Err("Unexpected balance format".to_string())
    }
}

#[tauri::command]
async fn get_nonce(state: State<'_, RpcConfig>, address: String) -> Result<String, String> {
    validate_address(&address)?;
    let params = Some(serde_json::json!([address, "latest"]));
    let result = call_rpc(&state, "eth_getTransactionCount", params).await?;
    if let Some(nonce_str) = result.as_str() {
        Ok(nonce_str.to_string())
    } else {
        Err("Unexpected nonce format".to_string())
    }
}

// ----------------------------------------------------------------------------
// Explorer Commands
// ----------------------------------------------------------------------------

#[tauri::command]
async fn get_latest_blocks(state: State<'_, RpcConfig>, count: u64) -> Result<Value, String> {
    // Get current block height
    let height_result = call_rpc(&state, "eth_blockNumber", None).await?;
    let height_str = height_result.as_str().ok_or("Invalid block height")?;
    let height = u64::from_str_radix(height_str.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Invalid height: {}", e))?;

    let mut blocks = Vec::new();
    let start = if height >= count { height - count + 1 } else { 0 };

    for block_num in (start..=height).rev().take(count as usize) {
        let params = Some(serde_json::json!([format!("0x{:x}", block_num), true]));
        if let Ok(block) = call_rpc(&state, "eth_getBlockByNumber", params).await {
            blocks.push(block);
        }
    }

    Ok(serde_json::json!(blocks))
}

#[tauri::command]
async fn get_dag_stats(state: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&state, "irondag_getDagStats", None).await
}

#[tauri::command]
async fn get_tps(state: State<'_, RpcConfig>) -> Result<Value, String> {
    let params = Some(serde_json::json!([60])); // 60 second window
    call_rpc(&state, "irondag_getTps", params).await
}

#[tauri::command]
async fn get_shard_stats(state: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&state, "irondag_getShardStats", None).await
}

#[tauri::command]
async fn get_mining_dashboard(state: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&state, "irondag_getMiningDashboard", None).await
}

// ----------------------------------------------------------------------------
// Key Management Commands
// ----------------------------------------------------------------------------

#[tauri::command]
fn create_new_key(keystore: State<'_, Arc<Mutex<KeyStore>>>) -> Result<String, String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let secret_key: [u8; 32] = rng.gen();

    let mut ks = keystore.lock().map_err(|e| e.to_string())?;
    ks.set_key(secret_key);

    let address = ks.get_address().ok_or("Failed to derive address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[tauri::command]
fn import_key(
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    private_key_hex: String,
) -> Result<String, String> {
    let bytes = validate_private_key(&private_key_hex)?;
    if bytes.len() != 32 {
        return Err("Private key must be 32 bytes".to_string());
    }

    let mut secret_key = [0u8; 32];
    secret_key.copy_from_slice(&bytes);

    let mut ks = keystore.lock().map_err(|e| e.to_string())?;
    ks.set_key(secret_key);

    let address = ks.get_address().ok_or("Failed to derive address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[tauri::command]
fn get_wallet_address(keystore: State<'_, Arc<Mutex<KeyStore>>>) -> Result<String, String> {
    let ks = keystore.lock().map_err(|e| e.to_string())?;
    if !ks.has_key() {
        return Err("No key loaded. Create or import a key first.".to_string());
    }
    let address = ks.get_address().ok_or("Failed to derive address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[tauri::command]
fn export_private_key(keystore: State<'_, Arc<Mutex<KeyStore>>>) -> Result<String, String> {
    let ks = keystore.lock().map_err(|e| e.to_string())?;
    if let Some(key) = ks.get_key() {
        Ok(format!("0x{}", hex::encode(key)))
    } else {
        Err("No key loaded".to_string())
    }
}

// ----------------------------------------------------------------------------
// Encrypted Keystore Commands
// ----------------------------------------------------------------------------

#[tauri::command]
fn has_keystore() -> Result<bool, String> {
    let keystore_path = get_keystore_path()?;
    Ok(keystore_path.exists())
}

#[tauri::command]
fn create_keystore(
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    password: String,
) -> Result<String, String> {
    use rand::Rng;
    
    // Get the current in-memory signing key
    let secret_key = {
        let ks = keystore.lock().map_err(|e| e.to_string())?;
        ks.get_key().ok_or("No key loaded. Create or import a key first.")?
    };
    
    // Generate random 16-byte salt
    let mut rng = rand::thread_rng();
    let mut salt = [0u8; 16];
    rng.fill(&mut salt);
    
    // Derive 32-byte AES key from password via Argon2id
    let derived_key = derive_key_from_password(&password, &salt)?;
    
    // Encrypt the private key bytes with AES-256-GCM (random 12-byte nonce)
    let (ciphertext, nonce) = encrypt_data(&derived_key, &secret_key)?;
    
    // Save to keystore.enc as JSON
    let encrypted_keystore = EncryptedKeystore {
        salt: hex::encode(salt),
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(&ciphertext),
    };
    
    let keystore_path = get_keystore_path()?;
    let json = serde_json::to_string_pretty(&encrypted_keystore)
        .map_err(|e| format!("Failed to serialize keystore: {}", e))?;
    fs::write(&keystore_path, json)
        .map_err(|e| format!("Failed to write keystore: {}", e))?;
    
    // Store the derived key for encrypting data files
    {
        let mut ks = keystore.lock().map_err(|e| e.to_string())?;
        ks.set_encryption_key(derived_key);
    }
    
    // Return the wallet address
    let ks = keystore.lock().map_err(|e| e.to_string())?;
    let address = ks.get_address().ok_or("Failed to derive address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[tauri::command]
fn unlock_keystore(
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    password: String,
) -> Result<String, String> {
    // Read keystore.enc
    let keystore_path = get_keystore_path()?;
    let json = fs::read_to_string(&keystore_path)
        .map_err(|e| format!("Failed to read keystore: {}", e))?;
    
    let encrypted: EncryptedKeystore = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse keystore: {}", e))?;
    
    // Parse salt, nonce, ciphertext
    let salt_bytes: [u8; 16] = hex::decode(&encrypted.salt)
        .map_err(|e| format!("Invalid salt: {}", e))?
        .try_into()
        .map_err(|_| "Salt must be 16 bytes")?;
    
    let nonce_bytes: [u8; 12] = hex::decode(&encrypted.nonce)
        .map_err(|e| format!("Invalid nonce: {}", e))?
        .try_into()
        .map_err(|_| "Nonce must be 12 bytes")?;
    
    let ciphertext = hex::decode(&encrypted.ciphertext)
        .map_err(|e| format!("Invalid ciphertext: {}", e))?;
    
    // Derive AES key from password + salt via Argon2id
    let derived_key = derive_key_from_password(&password, &salt_bytes)?;
    
    // Decrypt ciphertext
    let plaintext = decrypt_data(&derived_key, &nonce_bytes, &ciphertext)?;
    
    if plaintext.len() != 32 {
        return Err("Decrypted key has invalid length".to_string());
    }
    
    // Load the decrypted private key into in-memory key state
    let mut secret_key = [0u8; 32];
    secret_key.copy_from_slice(&plaintext);
    
    {
        let mut ks = keystore.lock().map_err(|e| e.to_string())?;
        ks.set_key(secret_key);
        ks.set_encryption_key(derived_key);
    }
    
    // Return the wallet address
    let ks = keystore.lock().map_err(|e| e.to_string())?;
    let address = ks.get_address().ok_or("Failed to derive address")?;
    Ok(format!("0x{}", hex::encode(address)))
}

#[tauri::command]
fn lock_keystore(keystore: State<'_, Arc<Mutex<KeyStore>>>) -> Result<(), String> {
    let mut ks = keystore.lock().map_err(|e| e.to_string())?;
    ks.clear_key();
    ks.clear_encryption_key();
    Ok(())
}

#[tauri::command]
fn delete_keystore(
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    password: String,
) -> Result<(), String> {
    // Read keystore.enc to verify password
    let keystore_path = get_keystore_path()?;
    let json = fs::read_to_string(&keystore_path)
        .map_err(|e| format!("Failed to read keystore: {}", e))?;
    
    let encrypted: EncryptedKeystore = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse keystore: {}", e))?;
    
    // Parse salt and nonce
    let salt_bytes: [u8; 16] = hex::decode(&encrypted.salt)
        .map_err(|e| format!("Invalid salt: {}", e))?
        .try_into()
        .map_err(|_| "Salt must be 16 bytes")?;
    
    let nonce_bytes: [u8; 12] = hex::decode(&encrypted.nonce)
        .map_err(|e| format!("Invalid nonce: {}", e))?
        .try_into()
        .map_err(|_| "Nonce must be 12 bytes")?;
    
    let ciphertext = hex::decode(&encrypted.ciphertext)
        .map_err(|e| format!("Invalid ciphertext: {}", e))?;
    
    // Derive key and attempt to decrypt to verify password
    let derived_key = derive_key_from_password(&password, &salt_bytes)?;
    let _ = decrypt_data(&derived_key, &nonce_bytes, &ciphertext)?;
    
    // Password verified, delete the keystore file
    fs::remove_file(&keystore_path)
        .map_err(|e| format!("Failed to delete keystore: {}", e))?;
    
    // Zero out in-memory key
    let mut ks = keystore.lock().map_err(|e| e.to_string())?;
    ks.clear_key();
    ks.clear_encryption_key();
    
    Ok(())
}

// ----------------------------------------------------------------------------
// Transaction History
// ----------------------------------------------------------------------------

#[tauri::command]
async fn get_address_transactions(
    rpc: State<'_, RpcConfig>,
    address: String,
    limit: Option<u64>,
) -> Result<Value, String> {
    validate_address(&address)?;
    let params = Some(serde_json::json!([address, limit.unwrap_or(50)]));
    call_rpc(&rpc, "irondag_getAddressTransactions", params).await
}

// ----------------------------------------------------------------------------
// Address Book Commands
// ----------------------------------------------------------------------------

#[tauri::command]
fn add_contact(
    address_book: State<'_, Arc<Mutex<AddressBook>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    name: String,
    address: String,
    notes: Option<String>,
) -> Result<(), String> {
    validate_address(&address)?;
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut book = address_book.lock().map_err(|e| e.to_string())?;
    book.add_contact(Contact { name, address, notes }, encryption_key)
}

#[tauri::command]
fn remove_contact(
    address_book: State<'_, Arc<Mutex<AddressBook>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    address: String,
) -> Result<(), String> {
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut book = address_book.lock().map_err(|e| e.to_string())?;
    book.remove_contact(&address, encryption_key)
}

#[tauri::command]
fn get_contacts(
    address_book: State<'_, Arc<Mutex<AddressBook>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
) -> Result<Vec<Contact>, String> {
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut book = address_book.lock().map_err(|e| e.to_string())?;
    // Reload with encryption key if available
    book.load(encryption_key);
    Ok(book.get_contacts())
}

// ----------------------------------------------------------------------------
// Multi-Account Commands
// ----------------------------------------------------------------------------

#[tauri::command]
fn add_account(
    accounts: State<'_, Arc<Mutex<Accounts>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    name: String,
    address: String,
) -> Result<(), String> {
    validate_address(&address)?;
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut accts = accounts.lock().map_err(|e| e.to_string())?;
    accts.add_account(Account { name, address }, encryption_key)
}

#[tauri::command]
fn remove_account(
    accounts: State<'_, Arc<Mutex<Accounts>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    address: String,
) -> Result<(), String> {
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut accts = accounts.lock().map_err(|e| e.to_string())?;
    accts.remove_account(&address, encryption_key)
}

#[tauri::command]
fn get_accounts(
    accounts: State<'_, Arc<Mutex<Accounts>>>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
) -> Result<Vec<Account>, String> {
    let encryption_key = keystore.lock().map_err(|e| e.to_string())?.get_encryption_key();
    let mut accts = accounts.lock().map_err(|e| e.to_string())?;
    // Reload with encryption key if available
    accts.load(encryption_key);
    Ok(accts.get_accounts())
}

// ----------------------------------------------------------------------------
// Account Abstraction Commands
// ----------------------------------------------------------------------------

#[tauri::command]
async fn create_wallet(
    rpc: State<'_, RpcConfig>,
    wallet_type: String,
    owner: String,
    config: Value,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "wallet_type": wallet_type,
        "owner": owner,
        "config": config
    }));
    call_rpc(&rpc, "irondag_createWallet", params).await
}

#[tauri::command]
async fn get_wallet(rpc: State<'_, RpcConfig>, address: String) -> Result<Value, String> {
    let params = Some(serde_json::json!([address]));
    call_rpc(&rpc, "irondag_getWallet", params).await
}

#[tauri::command]
async fn get_owner_wallets(rpc: State<'_, RpcConfig>, owner: String) -> Result<Value, String> {
    let params = Some(serde_json::json!([owner]));
    call_rpc(&rpc, "irondag_getOwnerWallets", params).await
}

#[tauri::command]
async fn is_contract_wallet(rpc: State<'_, RpcConfig>, address: String) -> Result<bool, String> {
    let params = Some(serde_json::json!([address]));
    let result = call_rpc(&rpc, "irondag_isContractWallet", params).await?;
    result.as_bool().ok_or("Invalid response".to_string())
}

#[tauri::command]
async fn create_multisig_transaction(
    rpc: State<'_, RpcConfig>,
    wallet_address: String,
    to: String,
    value: String,
    data: Option<String>,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "wallet_address": wallet_address,
        "to": to,
        "value": value,
        "data": data
    }));
    call_rpc(&rpc, "irondag_createMultisigTransaction", params).await
}

#[tauri::command]
async fn add_multisig_signature(
    rpc: State<'_, RpcConfig>,
    tx_hash: String,
    signer: String,
    signature: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "tx_hash": tx_hash,
        "signer": signer,
        "signature": signature
    }));
    call_rpc(&rpc, "irondag_addMultisigSignature", params).await
}

#[tauri::command]
async fn get_pending_multisig_transactions(
    rpc: State<'_, RpcConfig>,
    wallet_address: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([wallet_address]));
    call_rpc(&rpc, "irondag_getPendingMultisigTransactions", params).await
}

#[tauri::command]
async fn initiate_recovery(
    rpc: State<'_, RpcConfig>,
    wallet_address: String,
    new_owner: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "wallet_address": wallet_address,
        "new_owner": new_owner
    }));
    call_rpc(&rpc, "irondag_initiateRecovery", params).await
}

#[tauri::command]
async fn approve_recovery(
    rpc: State<'_, RpcConfig>,
    request_id: String,
    guardian_address: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "request_id": request_id,
        "guardian_address": guardian_address
    }));
    call_rpc(&rpc, "irondag_approveRecovery", params).await
}

#[tauri::command]
async fn get_recovery_status(
    rpc: State<'_, RpcConfig>,
    request_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([request_id]));
    call_rpc(&rpc, "irondag_getRecoveryStatus", params).await
}

#[tauri::command]
async fn complete_recovery(
    rpc: State<'_, RpcConfig>,
    request_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([request_id]));
    call_rpc(&rpc, "irondag_completeRecovery", params).await
}

#[tauri::command]
async fn cancel_recovery(
    rpc: State<'_, RpcConfig>,
    request_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([request_id]));
    call_rpc(&rpc, "irondag_cancelRecovery", params).await
}

#[tauri::command]
async fn create_batch_transaction(
    rpc: State<'_, RpcConfig>,
    wallet_address: String,
    operations: Value,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "wallet_address": wallet_address,
        "operations": operations
    }));
    call_rpc(&rpc, "irondag_createBatchTransaction", params).await
}

#[tauri::command]
async fn execute_batch_transaction(
    rpc: State<'_, RpcConfig>,
    batch_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([batch_id]));
    call_rpc(&rpc, "irondag_executeBatchTransaction", params).await
}

#[tauri::command]
async fn get_batch_status(
    rpc: State<'_, RpcConfig>,
    batch_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([batch_id]));
    call_rpc(&rpc, "irondag_getBatchStatus", params).await
}

#[tauri::command]
async fn estimate_batch_gas(
    rpc: State<'_, RpcConfig>,
    operations: Value,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([operations]));
    call_rpc(&rpc, "irondag_estimateBatchGas", params).await
}

// ----------------------------------------------------------------------------
// Parallel EVM Commands
// ----------------------------------------------------------------------------

#[tauri::command]
async fn enable_parallel_evm(
    rpc: State<'_, RpcConfig>,
    enabled: bool,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({"enabled": enabled}));
    call_rpc(&rpc, "irondag_enableParallelEVM", params).await
}

#[tauri::command]
async fn get_parallel_evm_stats(rpc: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&rpc, "irondag_getParallelEVMStats", None).await
}

#[tauri::command]
async fn estimate_parallel_improvement(
    rpc: State<'_, RpcConfig>,
    transactions: Value,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([transactions]));
    call_rpc(&rpc, "irondag_estimateParallelImprovement", params).await
}

// ----------------------------------------------------------------------------
// Quick Wins Commands
// ----------------------------------------------------------------------------

#[tauri::command]
async fn create_time_locked_transaction(
    rpc: State<'_, RpcConfig>,
    from: String,
    to: String,
    value: String,
    fee: String,
    execute_at_block: Option<u64>,
    execute_at_timestamp: Option<u64>,
) -> Result<Value, String> {
    let mut params_obj = serde_json::json!({
        "from": from,
        "to": to,
        "value": value,
        "fee": fee
    });
    if let Some(block) = execute_at_block {
        params_obj["execute_at_block"] = serde_json::json!(block);
    }
    if let Some(timestamp) = execute_at_timestamp {
        params_obj["execute_at_timestamp"] = serde_json::json!(timestamp);
    }
    let params = Some(params_obj);
    call_rpc(&rpc, "irondag_createTimeLockedTransaction", params).await
}

#[tauri::command]
async fn get_time_locked_transactions(rpc: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&rpc, "irondag_getTimeLockedTransactions", None).await
}

#[tauri::command]
async fn create_gasless_transaction(
    rpc: State<'_, RpcConfig>,
    from: String,
    to: String,
    value: String,
    fee: String,
    sponsor: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "from": from,
        "to": to,
        "value": value,
        "fee": fee,
        "sponsor": sponsor
    }));
    call_rpc(&rpc, "irondag_createGaslessTransaction", params).await
}

#[tauri::command]
async fn get_sponsored_transactions(
    rpc: State<'_, RpcConfig>,
    sponsor: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!([sponsor]));
    call_rpc(&rpc, "irondag_getSponsoredTransactions", params).await
}

#[tauri::command]
async fn get_reputation(rpc: State<'_, RpcConfig>, address: String) -> Result<Value, String> {
    let params = Some(serde_json::json!([address]));
    call_rpc(&rpc, "irondag_getReputation", params).await
}

#[tauri::command]
async fn get_reputation_factors(rpc: State<'_, RpcConfig>, address: String) -> Result<Value, String> {
    let params = Some(serde_json::json!([address]));
    call_rpc(&rpc, "irondag_getReputationFactors", params).await
}

// ----------------------------------------------------------------------------
// Privacy, Oracles, Randomness, Automations
// ----------------------------------------------------------------------------

#[tauri::command]
async fn irondag_get_privacy_stats(rpc: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&rpc, "irondag_getPrivacyStats", None).await
}

#[tauri::command]
async fn irondag_create_private_transaction(
    rpc: State<'_, RpcConfig>,
    from: String,
    to: String,
    amount: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "from": from,
        "to": to,
        "amount": amount
    }));
    call_rpc(&rpc, "irondag_createPrivateTransaction", params).await
}

#[tauri::command]
async fn irondag_get_price_feeds(rpc: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&rpc, "irondag_getPriceFeeds", None).await
}

#[tauri::command]
async fn irondag_get_price(rpc: State<'_, RpcConfig>, feed_id: String) -> Result<Value, String> {
    let params = Some(serde_json::json!({ "feed_id": feed_id }));
    call_rpc(&rpc, "irondag_getPrice", params).await
}

#[tauri::command]
async fn irondag_get_randomness(
    rpc: State<'_, RpcConfig>,
    request_id: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({ "request_id": request_id }));
    call_rpc(&rpc, "irondag_getRandomness", params).await
}

#[tauri::command]
async fn irondag_request_randomness(rpc: State<'_, RpcConfig>) -> Result<Value, String> {
    call_rpc(&rpc, "irondag_requestRandomness", None).await
}

#[tauri::command]
async fn irondag_get_recurring_transactions(
    rpc: State<'_, RpcConfig>,
    address: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({ "address": address }));
    call_rpc(&rpc, "irondag_getRecurringTransactions", params).await
}

#[tauri::command]
async fn irondag_create_recurring_transaction(
    rpc: State<'_, RpcConfig>,
    from: String,
    to: String,
    value: String,
    interval_seconds: u64,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "from": from,
        "to": to,
        "value": value,
        "interval_seconds": interval_seconds
    }));
    call_rpc(&rpc, "irondag_createRecurringTransaction", params).await
}

#[tauri::command]
async fn irondag_get_stop_loss_orders(
    rpc: State<'_, RpcConfig>,
    address: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({ "address": address }));
    call_rpc(&rpc, "irondag_getStopLossOrders", params).await
}

#[tauri::command]
async fn irondag_create_stop_loss(
    rpc: State<'_, RpcConfig>,
    token_symbol: String,
    amount: String,
    trigger_price: String,
    order_type: String,
) -> Result<Value, String> {
    let params = Some(serde_json::json!({
        "token_symbol": token_symbol,
        "amount": amount,
        "trigger_price": trigger_price,
        "order_type": order_type
    }));
    call_rpc(&rpc, "irondag_createStopLoss", params).await
}

// ----------------------------------------------------------------------------
// Transaction Signing & Sending
// ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Transaction {
    from: [u8; 20],
    to: [u8; 20],
    value: u128,
    fee: u128,
    nonce: u64,
    data: Vec<u8>,
    gas_limit: u64,
    hash: [u8; 32],
    signature: Vec<u8>,
    public_key: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pq_signature: Option<Value>, // placeholder for PQ
}

impl Transaction {
    fn calculate_hash(&self) -> [u8; 32] {
        use sha3::{Digest, Keccak256};
        let mut hasher = Keccak256::new();
        hasher.update(&self.from);
        hasher.update(&self.to);
        hasher.update(&self.value.to_le_bytes());
        hasher.update(&self.fee.to_le_bytes());
        hasher.update(&self.nonce.to_le_bytes());
        hasher.update(&self.data);
        hasher.update(&self.gas_limit.to_le_bytes());
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result[..]);
        hash
    }

    fn sign(mut self, secret_key: &[u8; 32]) -> Self {
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(secret_key);
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes: [u8; 32] = verifying_key.to_bytes();

        self.public_key = public_key_bytes.to_vec();

        let message = &self.hash;
        let signature = signing_key.sign(message);

        self.signature = signature.to_bytes().into();

        self
    }
}

#[tauri::command]
async fn send_transaction(
    rpc: State<'_, RpcConfig>,
    keystore: State<'_, Arc<Mutex<KeyStore>>>,
    to_address: String,
    value_hex: String,
    fee_hex: String,
) -> Result<String, String> {
    // Validate to address
    validate_address(&to_address)?;

    // Parse to address
    let to_hex = to_address.trim_start_matches("0x");
    let to_bytes = hex::decode(to_hex).map_err(|e| format!("Invalid to address: {}", e))?;
    if to_bytes.len() != 20 {
        return Err("To address must be 20 bytes".to_string());
    }
    let mut to = [0u8; 20];
    to.copy_from_slice(&to_bytes);

    // Parse value and fee
    let value = u128::from_str_radix(value_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Invalid value: {}", e))?;
    let fee = u128::from_str_radix(fee_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Invalid fee: {}", e))?;

    // Get secret key and from address (scope the lock)
    let (secret_key, from) = {
        let ks = keystore.lock().map_err(|e| e.to_string())?;
        let secret_key = ks.get_key().ok_or("No key loaded")?;
        let from = ks.get_address().ok_or("Failed to derive address")?;
        (secret_key, from)
    }; // Lock is dropped here

    // Get current nonce from node
    let from_str = format!("0x{}", hex::encode(from));
    let nonce_hex = get_nonce(rpc.clone(), from_str).await?;
    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Invalid nonce: {}", e))?;

    // Build transaction
    let mut tx = Transaction {
        from,
        to,
        value,
        fee,
        nonce,
        data: vec![],
        gas_limit: 21_000,
        hash: [0; 32],
        signature: vec![],
        public_key: vec![],
        pq_signature: None,
    };
    tx.hash = tx.calculate_hash();
    tx = tx.sign(&secret_key);

    // Send via RPC
    let tx_json = serde_json::to_value(&tx).map_err(|e| format!("Failed to serialize: {}", e))?;
    let params = Some(serde_json::json!([tx_json]));
    let result = call_rpc(&rpc, "irondag_sendRawTransaction", params).await?;

    // Extract tx hash from result
    if let Some(hash_str) = result.get("hash").and_then(|h| h.as_str()) {
        Ok(hash_str.to_string())
    } else {
        Err("Unexpected response from node".to_string())
    }
}

// ----------------------------------------------------------------------------
// Settings Persistence
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppSettings {
    rpc_url: String,
    active_tab: String,
    window_width: u32,
    window_height: u32,
    auto_start_node: bool,
    log_level: String,
    theme: String, // "dark" for now, prep for light mode
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            rpc_url: "http://127.0.0.1:8546".to_string(),
            active_tab: "dashboard".to_string(),
            window_width: 1200,
            window_height: 800,
            auto_start_node: false,
            log_level: "info".to_string(),
            theme: "dark".to_string(),
        }
    }
}

fn get_settings_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join("settings.json"))
}

fn load_settings() -> AppSettings {
    let settings_path = match get_settings_path() {
        Ok(p) => p,
        Err(_) => return AppSettings::default(),
    };
    
    if let Ok(data) = fs::read_to_string(&settings_path) {
        if let Ok(settings) = serde_json::from_str::<AppSettings>(&data) {
            return settings;
        }
    }
    
    AppSettings::default()
}

fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let settings_path = get_settings_path()?;
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(&settings_path, json)
        .map_err(|e| format!("Failed to write settings: {}", e))
}

#[tauri::command]
fn get_settings() -> Result<AppSettings, String> {
    Ok(load_settings())
}

#[tauri::command]
fn update_setting(key: String, value: String) -> Result<AppSettings, String> {
    let mut settings = load_settings();
    
    match key.as_str() {
        "rpc_url" => settings.rpc_url = value,
        "active_tab" => settings.active_tab = value,
        "window_width" => {
            settings.window_width = value.parse()
                .map_err(|_| format!("Invalid window_width: {}", value))?;
        }
        "window_height" => {
            settings.window_height = value.parse()
                .map_err(|_| format!("Invalid window_height: {}", value))?;
        }
        "auto_start_node" => {
            settings.auto_start_node = value.parse()
                .map_err(|_| format!("Invalid auto_start_node: {}", value))?;
        }
        "log_level" => {
            // Validate log level
            if !["debug", "info", "warn", "error"].contains(&value.as_str()) {
                return Err(format!("Invalid log_level: {}. Must be debug, info, warn, or error", value));
            }
            settings.log_level = value;
        }
        "theme" => {
            settings.theme = value;
        }
        _ => return Err(format!("Unknown setting key: {}", key)),
    }
    
    save_settings(&settings)?;
    Ok(settings)
}

#[tauri::command]
fn reset_settings() -> Result<AppSettings, String> {
    let settings_path = get_settings_path()?;
    if settings_path.exists() {
        fs::remove_file(&settings_path)
            .map_err(|e| format!("Failed to delete settings: {}", e))?;
    }
    Ok(AppSettings::default())
}

// ----------------------------------------------------------------------------
// Main Entry Point
// ----------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let keystore = Arc::new(Mutex::new(KeyStore::new()));
    let node_manager = Arc::new(Mutex::new(NodeProcessManager::new()));
    
    // Get app data directory for storage (use dirs::data_dir() for proper OS-specific location)
    let app_dir = get_data_dir().unwrap_or_else(|_| {
        // Fallback to current directory if data_dir is not available
        let fallback = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        std::fs::create_dir_all(&fallback).ok();
        fallback
    });
    
    let address_book = Arc::new(Mutex::new(AddressBook::new(
        app_dir.join("address_book.json")
    )));
    let accounts = Arc::new(Mutex::new(Accounts::new(
        app_dir.join("accounts.json")
    )));
    
    // Clean up any orphaned node processes from previous crashes
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // Check for orphaned node.exe processes
        let output = Command::new("powershell")
            .args([
                "-Command",
                "Get-Process | Where-Object {$_.ProcessName -eq 'node' -and $_.Path -like '*irondag-blockchain*'} | Select-Object -ExpandProperty Id"
            ])
            .output();
        
        if let Ok(output) = output {
            let pids_str = String::from_utf8_lossy(&output.stdout);
            let pids: Vec<&str> = pids_str.lines().filter(|s| !s.trim().is_empty()).collect();
            
            if !pids.is_empty() {
                println!("⚠️  Found {} orphaned node process(es) from previous session", pids.len());
                println!("🧹 Cleaning up orphaned nodes...");
                for pid in pids {
                    let _ = Command::new("taskkill")
                        .args(["/F", "/PID", pid.trim()])
                        .output();
                }
                println!("✅ Cleanup complete");
            }
        }
    }
    
    // Register cleanup handler to stop all nodes on app exit
    let cleanup_manager = node_manager.clone();
    ctrlc::set_handler(move || {
        println!("\n🛑 Shutting down desktop app, stopping all nodes...");
        if let Ok(mut manager) = cleanup_manager.lock() {
            let keys: Vec<String> = manager.processes.keys().cloned().collect();
            for key in keys {
                if let Some(mut entry) = manager.processes.remove(&key) {
                    let _ = entry.child.kill();
                    let _ = entry.child.wait();
                    println!("✅ Stopped node: {}", key);
                }
            }
        }
        std::process::exit(0);
    }).ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(RpcConfig {
            url: Arc::new(Mutex::new({
                // Load RPC URL from settings if available
                let settings = load_settings();
                settings.rpc_url
            })),
            api_key: None,
        })
        .manage(keystore)
        .manage(node_manager)
        .manage(address_book)
        .manage(accounts)
        .invoke_handler(tauri::generate_handler![
            get_node_status,
            get_rpc_url,
            set_rpc_url,
            get_mining_status,
            start_mining,
            stop_mining,
            get_node_processes,
            start_node,
            stop_node,
            stop_all_nodes,
            reset_data_dir,
            set_node_log_streaming,
            get_node_logs,
            clear_node_logs,
            list_tests,
            run_test,
            get_balance,
            get_nonce,
            create_new_key,
            import_key,
            get_wallet_address,
            export_private_key,
            // Encrypted Keystore Commands
            has_keystore,
            create_keystore,
            unlock_keystore,
            lock_keystore,
            delete_keystore,
            send_transaction,
            get_latest_blocks,
            get_dag_stats,
            get_tps,
            get_shard_stats,
            get_address_transactions,
            add_contact,
            remove_contact,
            get_contacts,
            add_account,
            remove_account,
            get_accounts,
            get_mining_dashboard,
            // Account Abstraction
            create_wallet,
            get_wallet,
            get_owner_wallets,
            is_contract_wallet,
            create_multisig_transaction,
            add_multisig_signature,
            get_pending_multisig_transactions,
            initiate_recovery,
            approve_recovery,
            get_recovery_status,
            complete_recovery,
            cancel_recovery,
            create_batch_transaction,
            execute_batch_transaction,
            get_batch_status,
            estimate_batch_gas,
            // Parallel EVM
            enable_parallel_evm,
            get_parallel_evm_stats,
            estimate_parallel_improvement,
            // Privacy / Oracles / Automations
            irondag_get_privacy_stats,
            irondag_create_private_transaction,
            irondag_get_price_feeds,
            irondag_get_price,
            irondag_get_randomness,
            irondag_request_randomness,
            irondag_get_recurring_transactions,
            irondag_create_recurring_transaction,
            irondag_get_stop_loss_orders,
            irondag_create_stop_loss,
            // Quick Wins
            create_time_locked_transaction,
            get_time_locked_transactions,
            create_gasless_transaction,
            get_sponsored_transactions,
            get_reputation,
            get_reputation_factors,
            // Settings
            get_settings,
            update_setting,
            reset_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running IronDAG Desktop");
}
