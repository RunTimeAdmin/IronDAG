# Kill any local IronDAG node processes (Windows).
# Run from repo root: .\scripts\stop_local_nodes.ps1
# Does not kill Node.js unless it was started from a path containing "irondag".

Write-Host "Stopping local IronDAG nodes..." -ForegroundColor Yellow

$killed = 0

# 1) PIDs file from local_multinode.ps1
if (Test-Path ".\local_node_pids.txt") {
    $nodePids = Get-Content ".\local_node_pids.txt"
    foreach ($nodePid in $nodePids) {
        if ($nodePid -match '^\d+$') {
            try {
                Stop-Process -Id ([int]$nodePid) -Force -ErrorAction Stop
                Write-Host "Stopped process $nodePid (from local_node_pids.txt)" -ForegroundColor Green
                $killed++
            } catch {
                Write-Host "Process $nodePid not running" -ForegroundColor Gray
            }
        }
    }
    Remove-Item ".\local_node_pids.txt" -Force -ErrorAction SilentlyContinue
}

# 2) Any process named node.exe / node whose path contains irondag (our binary)
Get-Process -Name "node" -ErrorAction SilentlyContinue | ForEach-Object {
    $path = $_.Path
    if ($path -and ($path -like "*irondag*" -or $path -like "*MondoShawan*")) {
        try {
            Stop-Process -Id $_.Id -Force -ErrorAction Stop
            Write-Host "Stopped process $($_.Id) ($path)" -ForegroundColor Green
            $killed++
        } catch {
            Write-Host "Could not stop $($_.Id): $_" -ForegroundColor Red
        }
    }
}

# 3) Anything listening on node ports (default + common test ports) that is named "node"
$ports = 8080, 8545, 9090, 9091, 9092, 8546
foreach ($port in $ports) {
    try {
        $conn = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty OwningProcess -Unique
        foreach ($pid in $conn) {
            $p = Get-Process -Id $pid -ErrorAction SilentlyContinue
            if ($p -and $p.ProcessName -eq "node") {
                Stop-Process -Id $pid -Force -ErrorAction Stop
                Write-Host "Stopped process $pid (was listening on port $port)" -ForegroundColor Green
                $killed++
            }
        }
    } catch {
        # Get-NetTCPConnection not available or no permission
    }
}

if ($killed -eq 0) {
    Write-Host "No local node processes found." -ForegroundColor Gray
} else {
    Write-Host "Done. Stopped $killed process(es)." -ForegroundColor Green
}
