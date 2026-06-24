# Local 2-node test: Node 1 = miner, Node 2 = sync peer
# Run from repo root: .\scripts\local_two_nodes.ps1
# If you get "cannot be loaded" or "disabled": run instead:
#   powershell -ExecutionPolicy Bypass -File .\scripts\local_two_nodes.ps1
# Or use the batch file: .\scripts\local_two_nodes.cmd  (double-click or run from CMD)
# Two console windows will open – one per node – so you can see each node's output.

$ErrorActionPreference = "Stop"

Write-Host "=== IronDAG 2-Node Test ===" -ForegroundColor Cyan
Write-Host "Two node windows will open so you can see logs." -ForegroundColor Gray

# Stop any existing local nodes
if (Test-Path ".\scripts\stop_local_nodes.ps1") {
    Write-Host "Stopping any existing nodes..." -ForegroundColor Yellow
    & ".\scripts\stop_local_nodes.ps1"
    Start-Sleep -Seconds 2
}

# Clean data for fresh test (optional - comment out to keep chain)
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node1"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node2"

# Build
Write-Host "`nBuilding node..." -ForegroundColor Yellow
Set-Location irondag-blockchain
cargo build --release --bin node
if ($LASTEXITCODE -ne 0) {
    Set-Location ..
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}
Set-Location ..

$root = Get-Location
$nodeBin = Join-Path $root "irondag-blockchain\target\release\node.exe"
if (-not (Test-Path $nodeBin)) {
    Write-Host "Node binary not found: $nodeBin" -ForegroundColor Red
    exit 1
}
$data1 = Join-Path $root "data_node1"
$data2 = Join-Path $root "data_node2"

# Node 1: miner – open in its own window so you can see output
Write-Host "`nStarting Node 1 (miner) in new window – title: Node 1 - Miner..." -ForegroundColor Green
# --rpc-no-auth: curl/RPC without API key (dev only). --advertise: distinct QUIC handshake keys on localhost.
$node1 = Start-Process -FilePath $nodeBin -ArgumentList "--port 8080 --rpc-port 8545 --data-dir `"$data1`" --single-stream --rpc-no-auth --advertise 127.0.0.1:8080" -WorkingDirectory (Get-Location) -PassThru -WindowStyle Normal
Write-Host "Node 1 PID: $($node1.Id) (check the other window for logs)"

Write-Host "Waiting 12s for Node 1 genesis and RPC..." -ForegroundColor Yellow
Start-Sleep -Seconds 12

# Check Node 1
try {
    $r = Invoke-RestMethod -Uri "http://127.0.0.1:8545" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $b1 = [Convert]::ToInt32($r.result, 16)
    Write-Host "Node 1 block: $b1" -ForegroundColor Green
} catch {
    Write-Host "Node 1 RPC not ready: $_" -ForegroundColor Red
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# Node 2: sync peer – open in its own window so you can see output
Write-Host "`nStarting Node 2 (sync) in new window – title: Node 2 - Sync..." -ForegroundColor Green
$node2 = Start-Process -FilePath $nodeBin -ArgumentList "--port 8082 --rpc-port 8546 --data-dir `"$data2`" --no-mining --peer 127.0.0.1:8080 --rpc-no-auth --advertise 127.0.0.1:8082" -WorkingDirectory (Get-Location) -PassThru -WindowStyle Normal
Write-Host "Node 2 PID: $($node2.Id) (check the other window for logs)"

Write-Host "Waiting 20s for Node 2 to connect and sync..." -ForegroundColor Yellow
Start-Sleep -Seconds 20

# Check both
Write-Host "`n=== Status ===" -ForegroundColor Cyan
foreach ($port in @(8545, 8546)) {
    try {
        $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
        $b = [Convert]::ToInt32($r.result, 16)
        $p = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 3
        $pc = [Convert]::ToInt32($p.result, 16)
        Write-Host "Port $port : block $b , peers $pc" -ForegroundColor Green
    } catch {
        Write-Host "Port $port : NOT RESPONDING" -ForegroundColor Red
    }
}

Write-Host "`n=== 2-Node Test Running ===" -ForegroundColor Cyan
Write-Host "Node 1 (miner): http://127.0.0.1:8545  PID $($node1.Id)"
Write-Host "Node 2 (sync): http://127.0.0.1:8546  PID $($node2.Id)"
Write-Host "Stop: .\scripts\stop_local_nodes.ps1"
@"
$($node1.Id)
$($node2.Id)
"@ | Out-File -FilePath ".\local_node_pids.txt"
Write-Host "`nPIDs in local_node_pids.txt" -ForegroundColor Gray
