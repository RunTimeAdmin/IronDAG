# Test local multi-node sync and transactions
# Run after local_multinode.ps1: .\scripts\test_local_cluster.ps1

$ErrorActionPreference = "Continue"

Write-Host "=== Local Cluster Test Suite ===" -ForegroundColor Cyan

$ports = @(8545, 8546, 8547)

# Test 1: All nodes responding
Write-Host "`n[TEST 1] Node Health Check" -ForegroundColor Yellow
$allHealthy = $true
foreach ($port in $ports) {
    try {
        $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
        $blockNum = [Convert]::ToInt32($response.result, 16)
        Write-Host "  Port $port : OK (block $blockNum)" -ForegroundColor Green
    } catch {
        Write-Host "  Port $port : FAILED" -ForegroundColor Red
        $allHealthy = $false
    }
}

if (-not $allHealthy) {
    Write-Host "`nSome nodes not healthy. Aborting tests." -ForegroundColor Red
    exit 1
}

# Test 2: Block sync check
Write-Host "`n[TEST 2] Block Sync Check" -ForegroundColor Yellow
$blocks = @()
foreach ($port in $ports) {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $blocks += [Convert]::ToInt32($response.result, 16)
}
$maxBlock = ($blocks | Measure-Object -Maximum).Maximum
$minBlock = ($blocks | Measure-Object -Minimum).Minimum
$diff = $maxBlock - $minBlock

Write-Host "  Block range: $minBlock - $maxBlock (diff: $diff)"
if ($diff -le 5) {
    Write-Host "  PASS: Nodes are in sync (within 5 blocks)" -ForegroundColor Green
} else {
    Write-Host "  WARN: Nodes may not be syncing (diff > 5)" -ForegroundColor Yellow
}

# Test 3: Chain ID consistency
Write-Host "`n[TEST 3] Chain ID Consistency" -ForegroundColor Yellow
$chainIds = @()
foreach ($port in $ports) {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $chainIds += $response.result
}
$uniqueChainIds = $chainIds | Select-Object -Unique
if ($uniqueChainIds.Count -eq 1) {
    Write-Host "  PASS: All nodes have chain ID $($uniqueChainIds[0])" -ForegroundColor Green
} else {
    Write-Host "  FAIL: Chain ID mismatch! $($chainIds -join ', ')" -ForegroundColor Red
}

# Test 4: Nonce consistency
Write-Host "`n[TEST 4] Nonce Consistency (test address)" -ForegroundColor Yellow
$testAddr = "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"
$nonces = @()
foreach ($port in $ports) {
    $body = "{`"jsonrpc`":`"2.0`",`"method`":`"eth_getTransactionCount`",`"params`":[`"$testAddr`",`"latest`"],`"id`":1}"
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body $body -ContentType "application/json" -TimeoutSec 5
    $nonces += $response.result
}
$uniqueNonces = $nonces | Select-Object -Unique
if ($uniqueNonces.Count -eq 1) {
    Write-Host "  PASS: All nodes report nonce $($uniqueNonces[0])" -ForegroundColor Green
} else {
    Write-Host "  FAIL: Nonce mismatch! $($nonces -join ', ')" -ForegroundColor Red
}

# Test 5: Wait and check sync progress
Write-Host "`n[TEST 5] Sync Progress (10 second wait)" -ForegroundColor Yellow
$initialBlocks = @()
foreach ($port in $ports) {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $initialBlocks += [Convert]::ToInt32($response.result, 16)
}

Write-Host "  Waiting 10 seconds..."
Start-Sleep -Seconds 10

$finalBlocks = @()
foreach ($port in $ports) {
    $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port" -Method Post -Body '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' -ContentType "application/json" -TimeoutSec 5
    $finalBlocks += [Convert]::ToInt32($response.result, 16)
}

Write-Host "  Initial blocks: $($initialBlocks -join ', ')"
Write-Host "  Final blocks:   $($finalBlocks -join ', ')"

$allProgressed = $true
for ($i = 0; $i -lt 3; $i++) {
    if ($finalBlocks[$i] -le $initialBlocks[$i]) {
        $allProgressed = $false
    }
}

if ($allProgressed) {
    Write-Host "  PASS: All nodes progressing" -ForegroundColor Green
} else {
    Write-Host "  WARN: Some nodes not progressing" -ForegroundColor Yellow
}

# Summary
Write-Host "`n=== Test Summary ===" -ForegroundColor Cyan
Write-Host "Run this script periodically to monitor cluster health."
Write-Host "For transaction tests, use: node test_explorer_tx.js"
