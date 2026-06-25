# Local 3-node testnet for development
# Run from project root: .\scripts\local_multinode.ps1

$ErrorActionPreference = "Stop"

Write-Host "=== IronDAG Local 3-Node Testnet ===" -ForegroundColor Cyan

# Clean up any existing data
Write-Host "`nCleaning up old data..." -ForegroundColor Yellow
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node1"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node2"
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue ".\data_node3"

# Build the node
Write-Host "`nBuilding node..." -ForegroundColor Yellow
Set-Location irondag-blockchain
cargo build --release --bin node
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}
Set-Location ..

$nodeBin = ".\irondag-blockchain\target\release\node.exe"

# Start Node 1 (genesis node)
Write-Host "`nStarting Node 1 (genesis) on ports 8080/8545..." -ForegroundColor Green
$node1 = Start-Process -FilePath $nodeBin -ArgumentList "8080 8545 --data-dir .\data_node1" -PassThru -NoNewWindow
Write-Host "Node 1 PID: $($node1.Id)"

# Wait for Node 1 to initialize and create genesis
Write-Host "Waiting 10 seconds for Node 1 to create genesis..." -ForegroundColor Yellow
Start-Sleep -Seconds 10

# Check Node 1 health
Write-Host "Checking Node 1 health..." -ForegroundColor Yellow
try {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:8546" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    Write-Host "Node 1 block: $($response.result)" -ForegroundColor Green
} catch {
    Write-Host "Node 1 not responding! Check logs." -ForegroundColor Red
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# Start Node 2 (connects to Node 1)
Write-Host "`nStarting Node 2 on ports 8081/8546, connecting to Node 1..." -ForegroundColor Green
$node2 = Start-Process -FilePath $nodeBin -ArgumentList "8081 8546 --data-dir .\data_node2 127.0.0.1:8080" -PassThru -NoNewWindow
Write-Host "Node 2 PID: $($node2.Id)"

# Start Node 3 (connects to Node 1)
Write-Host "`nStarting Node 3 on ports 8082/8547, connecting to Node 1..." -ForegroundColor Green
$node3 = Start-Process -FilePath $nodeBin -ArgumentList "8082 8547 --data-dir .\data_node3 127.0.0.1:8080" -PassThru -NoNewWindow
Write-Host "Node 3 PID: $($node3.Id)"

# Wait for peers to connect
Write-Host "`nWaiting 15 seconds for peer connections..." -ForegroundColor Yellow
Start-Sleep -Seconds 15

# Check all nodes
Write-Host "`n=== Node Status ===" -ForegroundColor Cyan
$ports = @(8545, 8546, 8547)
foreach ($port in $ports) {
    try {
        $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
        $blockNum = [Convert]::ToInt32($response.result, 16)
        Write-Host "Port $port : Block $blockNum" -ForegroundColor Green
    } catch {
        Write-Host "Port $port : NOT RESPONDING" -ForegroundColor Red
    }
}

Write-Host "`n=== Local Testnet Running ===" -ForegroundColor Cyan
Write-Host "Node 1: http://127.0.0.1:8546 (PID: $($node1.Id))"
Write-Host "Node 2: http://127.0.0.1:8546 (PID: $($node2.Id))"
Write-Host "Node 3: http://127.0.0.1:8547 (PID: $($node3.Id))"
Write-Host "`nTo stop: Stop-Process -Id $($node1.Id),$($node2.Id),$($node3.Id)"
Write-Host "Or run: .\scripts\stop_local_nodes.ps1"

# Save PIDs for later
@"
$($node1.Id)
$($node2.Id)
$($node3.Id)
"@ | Out-File -FilePath ".\local_node_pids.txt"

Write-Host "`nPIDs saved to local_node_pids.txt" -ForegroundColor Yellow
