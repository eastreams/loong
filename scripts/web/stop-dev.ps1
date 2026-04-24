param(
  [int]$ApiPort = 4317,
  [int]$DevPort = 4173
)

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

function Stop-PidFileProcess {
  param([string]$PidFile)

  if (-not (Test-Path $PidFile)) {
    return
  }

  $rawPid = (Get-Content $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
  if ($rawPid -match "^\d+$") {
    Stop-Process -Id ([int]$rawPid) -Force -ErrorAction SilentlyContinue
  }

  Remove-Item $PidFile -Force -ErrorAction SilentlyContinue
}

$runRoot = Join-Path $env:USERPROFILE ".loong\run"
$apiPidFile = Join-Path $runRoot "web-api.pid"
$devPidFile = Join-Path $runRoot "web-dev.pid"

Stop-PidFileProcess -PidFile $apiPidFile
Stop-PidFileProcess -PidFile $devPidFile

$ports = @($ApiPort, $DevPort)
foreach ($port in $ports) {
  $ids = Get-PortProcessIds -Port $port
  if ($ids.Count -gt 0) {
    Stop-Process -Id $ids -Force -ErrorAction SilentlyContinue
  }
}

Write-Output "Stopped web dev processes on ports $ApiPort and $DevPort."
