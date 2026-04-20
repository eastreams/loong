param(
  [string]$ApiBind = "127.0.0.1:4317",
  [string]$DevHost = "127.0.0.1",
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

function Stop-PortProcesses {
  param([int]$Port)

  $ids = Get-PortProcessIds -Port $Port
  if ($ids.Count -gt 0) {
    Stop-Process -Id $ids -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$webRoot = Join-Path $repoRoot "web"
$runtimeRoot = Join-Path $env:USERPROFILE ".loong"
$logRoot = Join-Path $runtimeRoot "logs"

New-Item -ItemType Directory -Force -Path $logRoot | Out-Null

$apiLog = Join-Path $logRoot "web-api.log"
$apiErr = Join-Path $logRoot "web-api.err.log"
$devLog = Join-Path $logRoot "web-dev.log"
$devErr = Join-Path $logRoot "web-dev.err.log"

Stop-PortProcesses -Port 4317
Stop-PortProcesses -Port $DevPort

function Resolve-DaemonExe {
  param([string]$RepoRoot)

  Push-Location $RepoRoot
  try {
    cargo build --bin loong
    if ($LASTEXITCODE -ne 0) {
      throw "Failed to build daemon binary with cargo build --bin loong"
    }
  } finally {
    Pop-Location
  }

  $builtDaemonExe = Join-Path $RepoRoot "target\debug\loong.exe"
  if (-not (Test-Path $builtDaemonExe)) {
    $legacyDaemonExe = Join-Path $RepoRoot "target\debug\loongclaw.exe"
    if (Test-Path $legacyDaemonExe) {
      return $legacyDaemonExe
    }
    throw "Missing daemon binary after build: $builtDaemonExe"
  }

  return $builtDaemonExe
}

$daemonExe = Resolve-DaemonExe -RepoRoot $repoRoot

$apiProc = Start-Process `
  -FilePath $daemonExe `
  -ArgumentList "web", "serve", "--bind", $ApiBind `
  -WorkingDirectory $repoRoot `
  -RedirectStandardOutput $apiLog `
  -RedirectStandardError $apiErr `
  -WindowStyle Hidden `
  -PassThru

$viteCmd = Join-Path $webRoot "node_modules\.bin\vite.cmd"
if (-not (Test-Path $viteCmd)) {
  throw "Missing Vite binary: $viteCmd"
}

$devProc = Start-Process `
  -FilePath $viteCmd `
  -ArgumentList "--host", $DevHost, "--port", "$DevPort" `
  -WorkingDirectory $webRoot `
  -RedirectStandardOutput $devLog `
  -RedirectStandardError $devErr `
  -WindowStyle Hidden `
  -PassThru

$apiReady = $false
for ($i = 0; $i -lt 20; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    $status = (Invoke-WebRequest -UseBasicParsing "http://$ApiBind/healthz" -TimeoutSec 3).StatusCode
    if ($status -eq 200) {
      $apiReady = $true
      break
    }
  } catch {
  }
}

$devReady = $false
for ($i = 0; $i -lt 20; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    $status = (Invoke-WebRequest -UseBasicParsing "http://$DevHost`:$DevPort/" -TimeoutSec 3).StatusCode
    if ($status -ge 200 -and $status -lt 500) {
      $devReady = $true
      break
    }
  } catch {
  }
}

if (-not $apiReady) {
  throw "Web API did not become ready. Check $apiErr"
}

if (-not $devReady) {
  throw "Web dev server did not become ready. Check $devErr"
}

Write-Output "Web API: http://$ApiBind"
Write-Output "Web Dev: http://$DevHost`:$DevPort"
Write-Output "Logs: $logRoot"
Write-Output "API PID: $($apiProc.Id)"
Write-Output "Dev PID: $($devProc.Id)"
