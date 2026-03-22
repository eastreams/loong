param(
  [string]$Bind = "127.0.0.1:4318",
  [switch]$Build
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

$scriptRoot = (Resolve-Path $PSScriptRoot).Path
$repoRoot = (Resolve-Path (Join-Path $scriptRoot "..\..")).Path
$webRoot = Join-Path $repoRoot "web"
$distRoot = Join-Path $webRoot "dist"
$logRoot = Join-Path $env:USERPROFILE ".loongclaw\logs"

New-Item -ItemType Directory -Force -Path $logRoot | Out-Null

$uiLog = Join-Path $logRoot "web-same-origin.log"
$uiErr = Join-Path $logRoot "web-same-origin.err.log"

$bindParts = $Bind.Split(":")
if ($bindParts.Length -lt 2) {
  throw "Bind must look like host:port, got: $Bind"
}
$port = [int]$bindParts[-1]
Stop-PortProcesses -Port $port

$daemonExe = Join-Path $repoRoot "target\debug\loongclaw.exe"
if (-not (Test-Path $daemonExe)) {
  throw "Missing daemon binary: $daemonExe"
}

if ($Build) {
  Push-Location $webRoot
  try {
    npm.cmd run build | Out-Null
    if ($LASTEXITCODE -ne 0) {
      throw "Web build failed. Fix the build first, then rerun this script."
    }
  } finally {
    Pop-Location
  }
}

$distIndex = Join-Path $distRoot "index.html"
if (-not (Test-Path $distIndex)) {
  throw "Missing built Web assets: $distIndex`nRun: cd web; npm.cmd run build"
}

$uiProc = Start-Process `
  -FilePath $daemonExe `
  -ArgumentList "web", "serve", "--bind", $Bind, "--static-root", $distRoot `
  -WorkingDirectory $repoRoot `
  -RedirectStandardOutput $uiLog `
  -RedirectStandardError $uiErr `
  -WindowStyle Hidden `
  -PassThru

$uiReady = $false
for ($i = 0; $i -lt 20; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    $status = (Invoke-WebRequest -UseBasicParsing "http://$Bind/" -TimeoutSec 3).StatusCode
    if ($status -ge 200 -and $status -lt 500) {
      $uiReady = $true
      break
    }
  } catch {
  }
}

if (-not $uiReady) {
  throw "Same-origin Web server did not become ready. Check $uiErr"
}

Write-Output "Web UI + API: http://$Bind"
Write-Output "Mode: same-origin-static"
Write-Output "Logs: $logRoot"
Write-Output "PID: $($uiProc.Id)"
