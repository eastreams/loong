$ErrorActionPreference = "Stop"

function Get-PortProcessIds {
  param([int]$Port)

  $lines = netstat -ano -p tcp | Select-String -Pattern "[:.]$Port\s"
  $ids = @()
  foreach ($line in $lines) {
    $parts = ($line.ToString().Trim() -split "\s+") | Where-Object { $_ }
    if ($parts.Length -ge 5) {
      $procId = $parts[-1]
      if ($procId -match "^\d+$") {
        $ids += [int]$procId
      }
    }
  }
  return $ids | Sort-Object -Unique
}

$ports = @(4317, 4173)
foreach ($port in $ports) {
  $ids = Get-PortProcessIds -Port $port
  if ($ids.Count -gt 0) {
    Stop-Process -Id $ids -Force -ErrorAction SilentlyContinue
  }
}

Write-Output "Stopped web dev processes on ports 4317 and 4173."
