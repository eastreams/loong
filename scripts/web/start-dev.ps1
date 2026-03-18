param(
  [string]$ApiBind = "127.0.0.1:4317",
  [string]$PreviewHost = "127.0.0.1",
  [int]$PreviewPort = 4173
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
$logRoot = Join-Path $env:USERPROFILE ".loongclaw\logs"

New-Item -ItemType Directory -Force -Path $logRoot | Out-Null

$apiLog = Join-Path $logRoot "web-api.log"
$apiErr = Join-Path $logRoot "web-api.err.log"
$previewLog = Join-Path $logRoot "web-preview.log"
$previewErr = Join-Path $logRoot "web-preview.err.log"

$userApiKey = [Environment]::GetEnvironmentVariable("ARK_API_KEY", "User")
if ($userApiKey) {
  $env:ARK_API_KEY = $userApiKey
}

Stop-PortProcesses -Port 4317
Stop-PortProcesses -Port $PreviewPort

$daemonExe = Join-Path $repoRoot "target\debug\loongclaw.exe"
if (-not (Test-Path $daemonExe)) {
  throw "Missing daemon binary: $daemonExe"
}

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

$distDir = Join-Path $webRoot "dist"
if (-not (Test-Path $distDir)) {
  Push-Location $webRoot
  try {
    npm.cmd run build | Out-Null
  } finally {
    Pop-Location
  }
}

$previewProc = Start-Process `
  -FilePath $viteCmd `
  -ArgumentList "preview", "--host", $PreviewHost, "--port", "$PreviewPort" `
  -WorkingDirectory $webRoot `
  -RedirectStandardOutput $previewLog `
  -RedirectStandardError $previewErr `
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

$previewReady = $false
for ($i = 0; $i -lt 20; $i++) {
  Start-Sleep -Milliseconds 500
  try {
    $status = (Invoke-WebRequest -UseBasicParsing "http://$PreviewHost`:$PreviewPort/" -TimeoutSec 3).StatusCode
    if ($status -ge 200 -and $status -lt 500) {
      $previewReady = $true
      break
    }
  } catch {
  }
}

if (-not $apiReady) {
  throw "Web API did not become ready. Check $apiErr"
}

if (-not $previewReady) {
  throw "Web preview did not become ready. Check $previewErr"
}

Write-Output "Web API: http://$ApiBind"
Write-Output "Web Preview: http://$PreviewHost`:$PreviewPort"
Write-Output "Logs: $logRoot"
Write-Output "API PID: $($apiProc.Id)"
Write-Output "Preview PID: $($previewProc.Id)"
