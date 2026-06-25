# IronDAG Desktop

**All-in-One Blockchain Experience** — Node control, wallet, mining, explorer, and metrics in one desktop application.

## ✨ Latest Update: Phase 2 Complete (January 2026)

**Performance Improvements:**
- ✅ **Zero Timeout Issues**: RPC calls now respond in <100ms (previously 30+ seconds)
- ✅ **Responsive UI**: No more freezing while mining is active
- ✅ **Concurrent Operations**: Mining, RPC, and UI all work simultaneously
- ✅ **Production Ready**: Tested with 1700+ blocks mined

**Backend Architecture:**
- Fine-grained locking eliminates UI blocking
- Lock-free account operations
- Safe async/sync mixing with tokio::task::block_in_place

---

## Features

✅ **Node Dashboard** — Monitor block height, transactions, peers, and mining status in real-time

✅ **Integrated Wallet** — Create wallets, check balances, and send transactions with Ed25519 signing

✅ **One-Click Mining** — Start/stop BraidCore Mining (ASIC, CPU, ZK proofs) with a single button (GPU planned)

✅ **Live Explorer** — Browse recent blocks, view DAG statistics (blue/red blocks), and monitor network performance

✅ **Performance Metrics** — Real-time TPS tracking, DAG consensus metrics, and per-shard statistics

✅ **Address Book** — Save and manage frequently used addresses with names and notes

✅ **Multi-Account Management** — Track multiple accounts and switch between them easily

✅ **Transaction History** — View transaction history for any address with detailed information

✅ **Account Abstraction** — Create and manage smart contract wallets (multi-sig, social recovery, spending limits)

✅ **Parallel EVM** — Enable/disable parallel execution and view performance statistics

✅ **Time-Locked Transactions** — Schedule transactions to execute at a future block or timestamp

✅ **Gasless Transactions** — Send transactions with a sponsor paying the fees

✅ **Reputation System** — View on-chain reputation scores and factors for any address

✅ **Native Desktop** — Built with Tauri (Rust + React) for Windows, macOS, and Linux

---

## Quick Start

### Prerequisites

1. **Rust** (for building Tauri backend)
2. **Node.js** (for React frontend)
3. **IronDAG Node** running on `127.0.0.1:8546`

### Installation

```bash
npm install
```

### Run Development Mode

**Terminal 1 — Start the IronDAG node:**
```bash
cd /path/to/irondag-blockchain
cargo run --bin node
```

Wait for: `RPC server listening on 127.0.0.1:8546`

**Terminal 2 — Start the desktop app:**
```bash
cd /path/to/irondag-desktop
npm run tauri dev
```

The desktop window will open automatically.

---

## Usage

### Dashboard Tab
- View node status: height, transaction count, peer count, mining state
- **Start/Stop Mining** with one click
- View BraidCore details: block times, max txs, and rewards for all three streams

### Wallet Tab
- Enter any address (0x...) to view balance and nonce
- Balance shown in both raw hex and human-readable IDAG format

### Send Tab
- **Create New Wallet**: Generates a new Ed25519 key pair
- **Send Transaction**: Enter recipient, value (IDAG), and fee (IDAG)
- Transaction signed locally and submitted via `irondag_sendRawTransaction`

### Explorer Tab
- View recent blocks with hash, timestamp, and transaction count
- DAG statistics: total blocks, blue/red blocks, avg txs per block
- Auto-refreshes every 10 seconds

### Metrics Tab
- Real-time TPS (60-second window)
- Network performance metrics
- Per-shard statistics (if sharding enabled)
- Cross-shard transaction flows

### History Tab
- View transaction history for any address
- Filter by address, transaction type, or date range
- Detailed transaction information (hash, from, to, value, fee, status)
- Export transaction history

### Address Book (Send Tab)
- **Add Contact**: Save frequently used addresses with names and notes
- **Remove Contact**: Delete contacts you no longer need
- **Quick Select**: Select from saved contacts when sending transactions
- Data persisted to `address_book.json` in app directory

### Multi-Account Management (Wallet Tab)
- **Add Account**: Track multiple wallet addresses with custom names
- **Remove Account**: Remove accounts you no longer need
- **Switch Accounts**: Quickly switch between tracked accounts
- **Account Overview**: View all accounts and their balances at a glance
- Data persisted to `accounts.json` in app directory

### Account Abstraction Tab (NEW!)
- **Create Smart Contract Wallets**: Basic, multi-sig, social recovery, spending limits, or combined
- **Wallet Management**: View all owned wallets, check details, and manage configurations
- **Multi-Signature Support**: Create wallets requiring n-of-m signatures for transactions
- **Social Recovery**: Set up guardian-based recovery for lost wallets
- **Spending Limits**: Configure daily spending limits for enhanced security

### Parallel EVM (Metrics Tab)
- **Enable/Disable**: Toggle parallel EVM execution for performance boost
- **Statistics**: View parallel execution rate, average speedup, and batch metrics
- **Performance Monitoring**: Track improvements from parallel execution

### Time-Locked & Gasless Transactions (Send Tab)
- **Time-Locked**: Schedule transactions to execute at a specific block number or timestamp
- **Gasless**: Send transactions with a sponsor address paying the fees
- **Combined Options**: Use both features together for advanced transaction scenarios

### Reputation Display (Wallet Tab)
- **Reputation Score**: View 0-100 reputation score with level (High/Medium/Low)
- **Detailed Factors**: See successful/failed transactions, blocks mined, account age, value transacted, and more
- **On-Chain Verification**: All reputation data comes directly from the blockchain

---

## Building for Production

### Windows (x64)

**Prerequisites:**
1. Install [Visual Studio Build Tools 2022](https://visualstudio.microsoft.com/downloads/)
   - Select "Desktop development with C++"
   - Select "Windows 10/11 SDK"
2. Install [Node.js 18+](https://nodejs.org/)
3. Install [Rust](https://rustup.rs/)

**Build Steps:**
```bash
# Install dependencies
npm install

# Build the application
npm run tauri build
```

**Output:**
- MSI Installer: `src-tauri\target\release\bundle\msi\IronDAG-Desktop_0.1.0_x64_en-US.msi`
- Portable EXE: `src-tauri\target\release\irondag-desktop.exe`

**Installation:**
- Double-click the MSI installer
- Follow the installation wizard
- Application will be installed to `C:\Program Files\IronDAG Desktop`
- Desktop shortcut created automatically

**System Requirements:**
- Windows 10/11 (64-bit)
- 4GB RAM minimum, 8GB recommended
- 500MB disk space
- Internet connection for blockchain sync

### macOS
```bash
npm run tauri build
```
Output: `src-tauri/target/release/bundle/dmg/IronDAG-Desktop_0.1.0_x64.dmg`

### Linux
```bash
npm run tauri build
```
Output: `src-tauri/target/release/bundle/appimage/irondag-desktop_0.1.0_amd64.AppImage`

---

## Security Notes

**Current Implementation (MVP):**
- Keys stored **in memory only** (lost when app closes)
- Address book and account data stored in JSON files (unencrypted)
- No encryption at rest
- No password protection

**For Production:**
- Implement encrypted keystore on disk
- Add password/biometric unlock
- Support hardware wallets
- Add multi-sig options

---

## Troubleshooting

### "Failed to fetch status" or "Connection Error"
**Problem**: Desktop app cannot connect to the blockchain node

**Solutions:**
1. **Ensure Node is Running**:
   ```bash
   cd /path/to/irondag-blockchain
   cargo run --release --bin node
   ```
   Wait for: `✅ JSON-RPC server listening on http://127.0.0.1:8546`

2. **Check Node Health**:
   ```powershell
   # Windows PowerShell
   Invoke-WebRequest -Uri 'http://127.0.0.1:8546' -Method POST -ContentType 'application/json' -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   ```
   Should return: `{"jsonrpc":"2.0","result":"0x...","id":1}`

3. **Verify Port**: 
   - Node RPC must be on port `8545`
   - Desktop app expects `http://127.0.0.1:8546`
   - Check firewall isn't blocking port 8546

4. **Check Logs**:
   - Node logs: `irondag-blockchain/node.err`
   - Desktop app console: Open DevTools (Ctrl+Shift+I)

### "Connection hangs" or "UI freezes"
**Problem**: Desktop app becomes unresponsive

**Solution**: This was fixed in Phase 2 (January 2026). Update to latest version:
```bash
git pull origin master
cd irondag-blockchain
cargo build --release --bin node
```
The issue was lock contention - now resolved with fine-grained locking.

### "No key loaded"
- Click "Create New Wallet" in the Send tab first

### "Invalid nonce"
- Wait for pending transactions to be mined
- Node's nonce doesn't match expected value

### "Insufficient balance"
- Wallet doesn't have enough IDAG + fee
- Use Wallet tab to check balance

---

## Tech Stack

- **Tauri 2.x** — Rust backend for native desktop
- **React 18** — Frontend UI framework
- **TypeScript** — Type-safe JavaScript
- **Vite** — Fast build tool
- **Ed25519-dalek** — Transaction signing
- **Reqwest** — HTTP client for RPC calls

---

## License

MIT License — See LICENSE file for details

---

## Links

- **Website**: [irondag.io](https://irondag.io)
- **Whitepaper**: [IronDAG Whitepaper](https://irondag.io/IronDAG_WHITEPAPER.html)
- **Explorer**: [Live Blockchain Explorer](https://irondag.io/explorer/)
- **Main Repo**: [irondag-blockchain](https://github.com/RunTimeAdmin/IronDAG)

---

**Built and operational today — not "coming soon".**

