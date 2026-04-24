param(
  [string]$Bind = "127.0.0.1:4318",
  [switch]$Build,
  [switch]$BuildDaemon
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

function Resolve-DaemonExePath {
  param([string]$RepoRoot)

  $primaryDaemonExe = Join-Path $RepoRoot "target\debug\loong.exe"
  if (Test-Path $primaryDaemonExe) {
    return $primaryDaemonExe
  }

  $legacyDaemonExe = Join-Path $RepoRoot "target\debug\loongclaw.exe"
  if (Test-Path $legacyDaemonExe) {
    return $legacyDaemonExe
  }

  return $primaryDaemonExe
}

function Resolve-DaemonExe {
  param(
    [string]$RepoRoot,
    [switch]$BuildDaemon
  )

  $daemonExe = Resolve-DaemonExePath -RepoRoot $RepoRoot
  $shouldBuild = $BuildDaemon -or (-not (Test-Path $daemonExe))

  if ($shouldBuild) {
    Push-Location $RepoRoot
    try {
      cargo build --bin loong
      if ($LASTEXITCODE -ne 0) {
        throw "Failed to build daemon binary with cargo build --bin loong"
      }
    } finally {
      Pop-Location
    }

    $daemonExe = Resolve-DaemonExePath -RepoRoot $RepoRoot
  }

  if (-not (Test-Path $daemonExe)) {
    throw "Missing daemon binary: $daemonExe`nRun with -BuildDaemon or build loong manually."
  }

  return $daemonExe
}

$scriptRoot = (Resolve-Path $PSScriptRoot).Path
$repoRoot = (Resolve-Path (Join-Path $scriptRoot "..\..")).Path
$webRoot = Join-Path $repoRoot "web"
$distRoot = Join-Path $webRoot "dist"
$runtimeRoot = Join-Path $env:USERPROFILE ".loong"
$logRoot = Join-Path $runtimeRoot "logs"
$runRoot = Join-Path $runtimeRoot "run"

New-Item -ItemType Directory -Force -Path $logRoot | Out-Null
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

$uiLog = Join-Path $logRoot "web-same-origin.log"
$uiErr = Join-Path $logRoot "web-same-origin.err.log"
$uiPidFile = Join-Path $runRoot "web-same-origin.pid"

$bindParts = $Bind.Split(":")
if ($bindParts.Length -lt 2) {
  throw "Bind must look like host:port, got: $Bind"
}
$port = [int]$bindParts[-1]

Stop-PidFileProcess -PidFile $uiPidFile
Stop-PortProcesses -Port $port

$daemonExe = Resolve-DaemonExe -RepoRoot $repoRoot -BuildDaemon:$BuildDaemon

if ($Build) {
  Push-Location $webRoot
  try {
    npm.cmd run build
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

Set-Content -Path $uiPidFile -Value $uiProc.Id -NoNewline

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
