param(
    [string]$Prefix = "$HOME/.local/bin",
    [switch]$Setup
)

$ErrorActionPreference = "Stop"

function Write-Usage {
    @"
Usage: pwsh ./scripts/install.ps1 [-Prefix <dir>] [-Setup]

Options:
  -Prefix <dir>   Install directory for loongclaw (default: $HOME/.local/bin)
  -Setup          Run 'loongclaw setup --force' after install
"@
}

if ($args -contains "-h" -or $args -contains "--help") {
    Write-Usage
    exit 0
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo not found in PATH. Install Rust first: https://rustup.rs"
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

Write-Host "==> Building loongclaw (release)"
Push-Location $repoRoot
try {
    cargo build -p loongclaw-daemon --bin loongclaw --release | Out-Host
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
$sourceBinary = Join-Path $repoRoot "target/release/loongclaw"
if (-not (Test-Path $sourceBinary)) {
    $sourceBinary = Join-Path $repoRoot "target/release/loongclaw.exe"
}
$destBinary = Join-Path $Prefix (Split-Path -Leaf $sourceBinary)
Copy-Item -Force $sourceBinary $destBinary

Write-Host "==> Installed loongclaw to $destBinary"

if ($Setup) {
    Write-Host "==> Running initial setup"
    & $destBinary setup --force | Out-Host
}

$pathItems = ($env:PATH -split [IO.Path]::PathSeparator)
if (-not ($pathItems -contains $Prefix)) {
    Write-Host ""
    Write-Host "Add to PATH if needed:"
    Write-Host "  `$env:PATH = \"$Prefix$([IO.Path]::PathSeparator)$env:PATH\""
}

Write-Host ""
Write-Host "Done. Try:"
Write-Host "  loongclaw --help"
