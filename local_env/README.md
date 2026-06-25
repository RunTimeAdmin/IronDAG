# Local 3-Node Testnet

Stable local environment: **2 BraidCore Mining nodes** + **1 RPC-only node**, all peered.

## Layout

| Node | Role            | P2P Port | RPC Port | Data Dir   |
|------|-----------------|----------|----------|------------|
| 1    | Bootstrap + mining | 30303  | **8545** | data/node1 |
| 2    | Peer + mining   | 30304    | 8546     | data/node2 |
| 3    | Peer, RPC-only  | 30305    | 8547     | data/node3 |

- **Primary RPC for clients:** `http://127.0.0.1:8546` (Node 1).
- Node 3 is RPC-only (no mining) for a stable, low-CPU RPC endpoint.

## Quick start

From repo root:

```powershell
# Start 3 nodes (each in its own window)
.\local_env\start_local_testnet.ps1

# Or run in background with logs (for long runs)
.\local_env\start_local_testnet.ps1 -Background

# Wipe data and start fresh
.\local_env\start_local_testnet.ps1 -Clean
```

Stop:

```powershell
.\local_env\stop_local_testnet.ps1
```

## Long runs (several hours)

1. Use **release** build (script builds it if missing).
2. Run in **background** so logs go to files and the session stays stable:
   ```powershell
   .\local_env\start_local_testnet.ps1 -Background
   ```
3. Logs: `local_env\logs\node1.out`, `node1.err`, etc.
4. To stop later: `.\local_env\stop_local_testnet.ps1`.

## Node dashboard (GUI)

Browser view of all three nodes (like explorer.irondag.io style):

1. Open **`local_env\node-dashboard.html`** in your browser (double-click or drag into Chrome/Edge).
2. If you see no data and CORS errors in the console, serve the folder and open the dashboard from the server:
   ```powershell
   cd irondag\local_env
   python -m http.server 8888
   ```
   Then open **http://localhost:8888/node-dashboard.html**. Leave the terminal open.

The page shows Node 1/2/3: status (Up/Down), block height, peers, TPS, and refreshes every 10 seconds.

## Monitor (errors, hangs, TPS)

Run in a separate window while the testnet is up:

```powershell
# Poll every 15s, print blocks / peers / TPS; flag errors and hangs (no block progress 90s+)
.\local_env\monitor_nodes.ps1

# Poll every 30s and append to log file
.\local_env\monitor_nodes.ps1 -Interval 30 -LogFile
```

- **Errors:** RPC timeouts or down nodes → printed in red and appended to `local_env\monitor_errors.txt`.
- **Hangs:** If a node’s block number doesn’t increase for 90s → reported as `HANG?`.
- **TPS:** From `irondag_getTps` (10s window) on the first responding node.

## Health check

```powershell
# Block height on primary RPC
$r = Invoke-RestMethod -Uri "http://127.0.0.1:8546" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json"
[Convert]::ToInt32($r.result, 16)
```

MetaMask: RPC URL `http://127.0.0.1:8546`, Chain ID `11567`, symbol IDAG.

