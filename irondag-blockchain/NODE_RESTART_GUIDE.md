# Node Restart Guide

## Quick Restart Commands

### Restart Node 1 (Primary)
```powershell
cd d:\Pyrax\irondag-blockchain
.\start_node1.ps1
```

Or manually:
```powershell
cargo run --release --bin node 8080 8545 8081 --data-dir data
```

### Restart Node 2 (Sync)
```powershell
cd d:\Pyrax\irondag-blockchain
.\restart_node2.ps1
```

Or manually:
```powershell
cargo run --release --bin node 8083 8546 8082 --data-dir data-node2 127.0.0.1:8080
```

## Stopping Nodes

### Method 1: Keyboard Interrupt (Recommended)
- In the node's console window, press `Ctrl+C`
- This gracefully shuts down the node

### Method 2: Kill Process
```powershell
# Kill all node processes
taskkill /F /IM node.exe

# Or kill specific PID (check with Get-Process -Name node)
taskkill /F /PID <process_id>
```

## Port Configuration

### Node 1 (Primary)
- **P2P Port:** 8080
- **RPC Port:** 8545
- **HTTP API Port:** 8081
- **Data Directory:** `data/`

### Node 2 (Sync)
- **P2P Port:** 8083 (changed from 30304/30305 due to Windows permission issues)
- **RPC Port:** 8546
- **HTTP API Port:** 8082
- **Data Directory:** `data-node2/`
- **Peer:** `127.0.0.1:8080` (Node 1)

## Troubleshooting

### Port Already in Use
If you get "port already in use" error:
1. Check what's using the port: `netstat -an | findstr ":8080"`
2. Kill the process using that port
3. Or use a different port

### Windows Permission Error (10013)
If you get "access forbidden" error:
- Try a different port (avoid ports like 30304)
- Ports 30305+ usually work fine

### Node Won't Start
1. Check if another instance is running: `Get-Process -Name node`
2. Kill existing processes: `taskkill /F /IM node.exe`
3. Try restarting

## Monitoring

After restarting, check status:
```powershell
.\monitor_sync.ps1
```

This shows:
- Node 1 block height
- Node 2 block height
- Sync progress

---

**Last Updated:** 2026-01-12
