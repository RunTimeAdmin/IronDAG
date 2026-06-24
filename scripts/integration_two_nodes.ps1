# Integration test: 2-node sync
# Run from repo root: powershell -ExecutionPolicy Bypass -File .\scripts\integration_two_nodes.ps1
# Node 1: miner on 8080/8545. Node 2: sync on 8082/8546 (8082 avoids Node 1's sync server on 8081)

$ErrorActionPreference = "Stop"

Write-Host "=== IronDAG 2-Node Integration Test ===" -ForegroundColor Cyan

# Stop existing nodes
if (Test-Path ".\scripts\stop_local_nodes.ps1") {
    Write-Host "Stopping any existing nodes..." -ForegroundColor Yellow
    & ".\scripts\stop_local_nodes.ps1"
    Start-Sleep -Seconds 2
}

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node1"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node2"

# Build
Write-Host "`nBuilding node..." -ForegroundColor Yellow
Push-Location irondag-blockchain
cargo build --release --bin node
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}
Pop-Location

$nodeBin = ".\irondag-blockchain\target\release\node.exe"
if (-not (Test-Path $nodeBin)) {
    Write-Host "Node binary not found" -ForegroundColor Red
    exit 1
}

# Node 1: miner (P2P 8080, sync 8081, RPC 8545)
Write-Host "`nStarting Node 1 (miner) on 8080/8545..." -ForegroundColor Green
$node1 = Start-Process -FilePath $nodeBin -ArgumentList "--port 8080 --rpc-port 8545 --data-dir data_node1 --single-stream" -WorkingDirectory (Get-Location) -PassThru -WindowStyle Normal

Write-Host "Waiting 15s for Node 1 genesis..." -ForegroundColor Yellow
Start-Sleep -Seconds 15

try {
    $r = Invoke-RestMethod -Uri "http://127.0.0.1:8545" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $b1 = [Convert]::ToInt32($r.result, 16)
    Write-Host "Node 1 OK - block: $b1" -ForegroundColor Green
} catch {
    Write-Host "Node 1 RPC not ready: $_" -ForegroundColor Red
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# Node 2: sync (P2P 8082 to avoid 8081, RPC 8546)
Write-Host "`nStarting Node 2 (sync) on 8082/8546, peering to 127.0.0.1:8080..." -ForegroundColor Green
$node2 = Start-Process -FilePath $nodeBin -ArgumentList "--port 8082 --rpc-port 8546 --data-dir data_node2 --no-mining --peer 127.0.0.1:8080" -WorkingDirectory (Get-Location) -PassThru -WindowStyle Normal

Write-Host "Waiting 25s for Node 2 to connect and sync..." -ForegroundColor Yellow
Start-Sleep -Seconds 25

# Check both
Write-Host "`n=== Sync Status ===" -ForegroundColor Cyan
$n1 = $null; $n2 = $null
try {
    $b = Invoke-RestMethod -Uri "http://127.0.0.1:8545" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $p = Invoke-RestMethod -Uri "http://127.0.0.1:8545" -Method Post -Body '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $n1 = @{ block = [Convert]::ToInt32($b.result, 16); peers = [Convert]::ToInt32($p.result, 16) }
} catch { Write-Host "Node 1: $_" -ForegroundColor Red }

try {
    $b = Invoke-RestMethod -Uri "http://127.0.0.1:8546" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $p = Invoke-RestMethod -Uri "http://127.0.0.1:8546" -Method Post -Body '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $n2 = @{ block = [Convert]::ToInt32($b.result, 16); peers = [Convert]::ToInt32($p.result, 16) }
} catch { Write-Host "Node 2: $_" -ForegroundColor Red }

if ($n1) { Write-Host "Node 1 (miner): block $($n1.block) | peers $($n1.peers)" -ForegroundColor Green }
if ($n2) { Write-Host "Node 2 (sync):  block $($n2.block) | peers $($n2.peers)" -ForegroundColor Green }

if ($n1 -and $n2) {
    $gap = [Math]::Abs($n1.block - $n2.block)
    if ($gap -le 5 -and $n1.peers -ge 1) {
        Write-Host "`nPASS: Sync OK - blocks within $gap, peers connected" -ForegroundColor Green
    } else {
        Write-Host "`nCheck: Sync gap=$gap, Node1 peers=$($n1.peers)" -ForegroundColor Yellow
    }
}

# Save PIDs for stop script
"$($node1.Id)`n$($node2.Id)" | Out-File -FilePath ".\local_node_pids.txt" -Encoding utf8

Write-Host "`nNode 1: http://127.0.0.1:8545  (PID $($node1.Id))"
Write-Host "Node 2: http://127.0.0.1:8546  (PID $($node2.Id))"
Write-Host "Stop: .\scripts\stop_local_nodes.ps1" -ForegroundColor Gray
