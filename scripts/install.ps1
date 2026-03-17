param(
    [string]$Prefix = "$HOME/.local/bin",
    [switch]$Onboard,
    [string]$Version = $(if ($env:LOONGCLAW_INSTALL_VERSION) { $env:LOONGCLAW_INSTALL_VERSION } else { "latest" }),
    [switch]$Source,
    [string]$Repository = $(if ($env:LOONGCLAW_INSTALL_REPO) { $env:LOONGCLAW_INSTALL_REPO } else { "loongclaw-ai/loongclaw" })
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$ReleaseBaseUrl = if ($env:LOONGCLAW_INSTALL_RELEASE_BASE_URL) { $env:LOONGCLAW_INSTALL_RELEASE_BASE_URL } else { "https://github.com/$Repository/releases" }

function Write-Usage {
    @"
Usage: pwsh ./scripts/install.ps1 [-Prefix <dir>] [-Onboard] [-Version <tag>] [-Source]

Options:
  -Prefix <dir>   Install directory for loongclaw (default: $HOME/.local/bin)
  -Onboard        Run 'loongclaw onboard' after install
  -Version <tag>  Release tag to install (default: latest)
  -Source         Build from local source instead of downloading a release binary
"@
}

if ($args -contains "-h" -or $args -contains "--help") {
    Write-Usage
    exit 0
}

function Normalize-ReleaseTag([string]$Raw) {
    if ([string]::IsNullOrWhiteSpace($Raw) -or $Raw -eq "latest") {
        return "latest"
    }
    if ($Raw.StartsWith("v")) {
        return $Raw
    }
    return "v$Raw"
}

function New-MissingReleaseGuidance([string]$Repo) {
    $repoName = ($Repo -split "/")[-1]
    return @"
no GitHub release is published for $Repo yet.

Install from a local checkout instead:
  git clone https://github.com/$Repo.git
  cd $repoName
  pwsh ./scripts/install.ps1 -Source -Onboard
"@
}

function Resolve-LatestReleaseTag([string]$Repo) {
    $headers = @{ "User-Agent" = "LoongClaw-Install" }
    try {
        $release = Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases/latest"
    } catch {
        throw (New-MissingReleaseGuidance -Repo $Repo)
    }
    if (-not $release.tag_name) {
        throw "failed to resolve latest release tag for $Repo"
    }
    return [string]$release.tag_name
}

function Resolve-ReleaseTarget([string]$Platform, [string]$Arch) {
    $normalizedPlatform = $Platform.ToUpperInvariant()
    $normalizedArch = $Arch.ToLowerInvariant()

    switch -Wildcard ($normalizedPlatform) {
        "WINDOWS_NT" {
            switch ($normalizedArch) {
                "amd64" { return "x86_64-pc-windows-msvc" }
                default { throw "unsupported Windows architecture: $Arch" }
            }
        }
        default {
            throw "unsupported platform for install.ps1: $Platform"
        }
    }
}

function Get-ReleaseArchiveName([string]$PackageName, [string]$Tag, [string]$Target) {
    return "$PackageName-$Tag-$Target.zip"
}

function Get-ReleaseChecksumName([string]$PackageName, [string]$Tag, [string]$Target) {
    return "$(Get-ReleaseArchiveName -PackageName $PackageName -Tag $Tag -Target $Target).sha256"
}

function Install-FromSource {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $repoRoot = Resolve-Path (Join-Path $scriptDir "..")
    $cargoToml = Join-Path $repoRoot "Cargo.toml"
    if (-not (Test-Path $cargoToml)) {
        throw "-Source requires running this installer from a loongclaw repository checkout"
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo not found in PATH. Install Rust first: https://rustup.rs"
    }

    Write-Host "==> Building loongclaw from source (release)"
    Push-Location $repoRoot
    $hadReleaseBuild = Test-Path Env:LOONGCLAW_RELEASE_BUILD
    $previousReleaseBuild = $env:LOONGCLAW_RELEASE_BUILD
    try {
        $env:LOONGCLAW_RELEASE_BUILD = "1"
        cargo build -p loongclaw-daemon --bin loongclaw --release --locked | Out-Host
    } finally {
        if ($hadReleaseBuild) {
            $env:LOONGCLAW_RELEASE_BUILD = $previousReleaseBuild
        } elseif (Test-Path Env:LOONGCLAW_RELEASE_BUILD) {
            Remove-Item Env:LOONGCLAW_RELEASE_BUILD
        }
        Pop-Location
    }

    $sourceBinary = Join-Path $repoRoot "target/release/loongclaw.exe"
    if (-not (Test-Path $sourceBinary)) {
        throw "built binary not found at $sourceBinary"
    }

    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    $destBinary = Join-Path $Prefix "loongclaw.exe"
    Copy-Item -Force $sourceBinary $destBinary
    return $destBinary
}

function Install-FromRelease {
    $releaseTag = Normalize-ReleaseTag $Version
    if ($releaseTag -eq "latest") {
        $releaseTag = Resolve-LatestReleaseTag $Repository
    }

    $target = Resolve-ReleaseTarget -Platform $env:OS -Arch $env:PROCESSOR_ARCHITECTURE
    $packageName = "loongclaw"
    $archiveName = Get-ReleaseArchiveName -PackageName $packageName -Tag $releaseTag -Target $target
    $checksumName = Get-ReleaseChecksumName -PackageName $packageName -Tag $releaseTag -Target $target
    $releaseBase = "$ReleaseBaseUrl/download/$releaseTag"
    $archiveUrl = "$releaseBase/$archiveName"
    $checksumUrl = "$releaseBase/$checksumName"

    $tmpRoot = Join-Path ([IO.Path]::GetTempPath()) ("loongclaw-install-" + [guid]::NewGuid().ToString("N"))
    $extractRoot = Join-Path $tmpRoot "extract"
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null

    try {
        $archivePath = Join-Path $tmpRoot $archiveName
        $checksumPath = Join-Path $tmpRoot $checksumName

        Write-Host "==> Downloading loongclaw $releaseTag for $target"
        Invoke-WebRequest -Headers @{ "User-Agent" = "LoongClaw-Install" } -Uri $archiveUrl -OutFile $archivePath
        Invoke-WebRequest -Headers @{ "User-Agent" = "LoongClaw-Install" } -Uri $checksumUrl -OutFile $checksumPath

        $checksumText = (Get-Content -Raw -Path $checksumPath).Trim()
        if ([string]::IsNullOrWhiteSpace($checksumText)) {
            throw "checksum file $checksumName did not contain a SHA256 value"
        }
        $expectedSha = $checksumText.Split([char[]]" `t`r`n", [System.StringSplitOptions]::RemoveEmptyEntries)[0].ToLowerInvariant()
        $actualSha = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
        if ($expectedSha -ne $actualSha) {
            throw "checksum verification failed for $archiveName"
        }

        Expand-Archive -Path $archivePath -DestinationPath $extractRoot -Force
        $sourceBinary = Join-Path $extractRoot "loongclaw.exe"
        if (-not (Test-Path $sourceBinary)) {
            throw "extracted binary not found at $sourceBinary"
        }

        New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
        $destBinary = Join-Path $Prefix "loongclaw.exe"
        Copy-Item -Force $sourceBinary $destBinary
        return $destBinary
    } finally {
        if (Test-Path $tmpRoot) {
            Remove-Item -Recurse -Force $tmpRoot
        }
    }
}

$destBinary = if ($Source) { Install-FromSource } else { Install-FromRelease }

Write-Host "==> Installed loongclaw to $destBinary"

if ($Onboard) {
    Write-Host "==> Running guided onboarding"
    & $destBinary onboard | Out-Host
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
