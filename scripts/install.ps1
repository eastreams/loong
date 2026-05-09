param(
    [string]$Prefix = "$HOME/.local/bin",
    [switch]$Onboard,
    [string]$Version = $(if ($env:LOONG_INSTALL_VERSION) { $env:LOONG_INSTALL_VERSION } else { "latest" }),
    [switch]$Source,
    [string]$Repository = $(if ($env:LOONG_INSTALL_REPO) { $env:LOONG_INSTALL_REPO } else { "eastreams/loong" })
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$Prefix = [IO.Path]::GetFullPath(($Prefix -replace '^~', $HOME))
$ReleaseBaseUrl = if ($env:LOONG_INSTALL_RELEASE_BASE_URL) {
    $env:LOONG_INSTALL_RELEASE_BASE_URL
} elseif ($env:LOONG_INSTALL_RELEASE_BASE_URL) {
    $env:LOONG_INSTALL_RELEASE_BASE_URL
} else {
    "https://github.com/$Repository/releases"
}
$BinName = "loong"

function Write-Usage {
    @"
Usage: pwsh ./scripts/install.ps1 [-Prefix <dir>] [-Onboard] [-Version <tag>] [-Source]

Options:
  -Prefix <dir>   Install directory for loong (default: $HOME/.local/bin)
  -Onboard        Run 'loong onboard' after install
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
    $headers = @{ "User-Agent" = "Loong-Install" }
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
    $targetLabel = switch ($Target) {
        "aarch64-apple-darwin" { "macos-arm64"; break }
        "x86_64-apple-darwin" { "macos-x64"; break }
        "aarch64-linux-android" { "android-arm64"; break }
        "aarch64-unknown-linux-gnu" { "linux-arm64-gnu"; break }
        "x86_64-unknown-linux-gnu" { "linux-x64-gnu"; break }
        "x86_64-unknown-linux-musl" { "linux-x64-musl"; break }
        "x86_64-pc-windows-msvc" { "windows-x64"; break }
        default { throw "unsupported release target label for $Target" }
    }

    $extension = if ($Target -like "*-pc-windows-*") { "zip" } else { "tar.gz" }
    return "$PackageName-$Tag-$targetLabel.$extension"
}

function Get-ReleaseChecksumName([string]$PackageName, [string]$Tag, [string]$Target) {
    return "loong-$Tag-SHA256SUMS.txt"
}

function Install-Binary([string]$SourceBinary) {
    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    $primaryBinary = Join-Path $Prefix "$BinName.exe"
    Copy-Item -Force $SourceBinary $primaryBinary
    return $primaryBinary
}

function Remove-LegacyBinaryIfPresent {
    $legacyBinaryName = "loongclaw.exe"
    $legacyBinary = Join-Path $Prefix $legacyBinaryName
    $legacyBinaryItem = Get-Item -LiteralPath $legacyBinary -Force -ErrorAction SilentlyContinue
    if ($null -eq $legacyBinaryItem) {
        return
    }
    if ($legacyBinaryItem.PSIsContainer) {
        return
    }

    Remove-Item -LiteralPath $legacyBinary -Force
    Write-Host "==> Removed legacy loongclaw compatibility command from $legacyBinary"
}

function Install-FromSource {
    $scriptDir = $PSScriptRoot
    $repoRoot = Resolve-Path (Join-Path $scriptDir "..")
    $cargoToml = Join-Path $repoRoot "Cargo.toml"
    if (-not (Test-Path $cargoToml)) {
        throw "-Source requires running this installer from a loong repository checkout"
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo not found in PATH. Install Rust first: https://rustup.rs"
    }

    Write-Host "==> Building loong from source (release)"
    Push-Location $repoRoot
    $hadReleaseBuild = (Test-Path Env:LOONG_RELEASE_BUILD)
    $previousReleaseBuild = $env:LOONG_RELEASE_BUILD
    try {
        $env:LOONG_RELEASE_BUILD = "1"
        cargo build -p loong --bin $BinName --release --locked | Out-Host
    } finally {
        if ($hadReleaseBuild) {
            $env:LOONG_RELEASE_BUILD = $previousReleaseBuild
        } elseif (Test-Path Env:LOONG_RELEASE_BUILD) {
            Remove-Item Env:LOONG_RELEASE_BUILD
        }
        Pop-Location
    }

    $sourceBinary = Join-Path $repoRoot "target/release/$BinName.exe"
    if (-not (Test-Path $sourceBinary)) {
        throw "built binary not found at $sourceBinary"
    }

    return Install-Binary -SourceBinary $sourceBinary
}

function Install-FromRelease {
    $releaseTag = Normalize-ReleaseTag $Version
    if ($releaseTag -eq "latest") {
        $releaseTag = Resolve-LatestReleaseTag $Repository
    }

    $target = Resolve-ReleaseTarget -Platform $env:OS -Arch $env:PROCESSOR_ARCHITECTURE
    $packageName = "loong"
    $archiveName = Get-ReleaseArchiveName -PackageName $packageName -Tag $releaseTag -Target $target
    $checksumName = Get-ReleaseChecksumName -PackageName $packageName -Tag $releaseTag -Target $target
    $releaseBase = "$ReleaseBaseUrl/download/$releaseTag"
    $archiveUrl = "$releaseBase/$archiveName"
    $checksumUrl = "$releaseBase/$checksumName"

    $tmpRoot = Join-Path ([IO.Path]::GetTempPath()) ("loong-install-" + [guid]::NewGuid().ToString("N"))
    $extractRoot = Join-Path $tmpRoot "extract"
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null

    try {
        $archivePath = Join-Path $tmpRoot $archiveName
        $checksumPath = Join-Path $tmpRoot $checksumName

        Write-Host "==> Downloading loong $releaseTag for $target"
        Invoke-WebRequest -Headers @{ "User-Agent" = "Loong-Install" } -Uri $archiveUrl -OutFile $archivePath
        Invoke-WebRequest -Headers @{ "User-Agent" = "Loong-Install" } -Uri $checksumUrl -OutFile $checksumPath

        $checksumText = Get-Content -Raw -Path $checksumPath
        $checksumEntry = $checksumText -split "`r?`n" | Where-Object {
            $_ -match "^[0-9a-fA-F]+\s+$([regex]::Escape($archiveName))$"
        } | Select-Object -First 1
        if ([string]::IsNullOrWhiteSpace($checksumEntry)) {
            throw "checksum manifest $checksumName did not contain an entry for $archiveName"
        }
        $expectedSha = $checksumEntry.Split([char[]]" `t", [System.StringSplitOptions]::RemoveEmptyEntries)[0].ToLowerInvariant()
        $actualSha = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
        if ($expectedSha -ne $actualSha) {
            throw "checksum verification failed for $archiveName"
        }

        Expand-Archive -Path $archivePath -DestinationPath $extractRoot -Force
        $sourceBinary = Join-Path $extractRoot "$BinName.exe"
        if (-not (Test-Path $sourceBinary)) {
            throw "extracted binary not found at $sourceBinary"
        }

        return Install-Binary -SourceBinary $sourceBinary
    } finally {
        if (Test-Path $tmpRoot) {
            Remove-Item -Recurse -Force $tmpRoot
        }
    }
}

function Resolve-NormalizedPathEntryOrNull([string]$PathEntry) {
    if ([string]::IsNullOrWhiteSpace($PathEntry)) {
        return $null
    }

    try {
        return [IO.Path]::GetFullPath($PathEntry)
    } catch {
        return $null
    }
}

$primaryBinary = if ($Source) { Install-FromSource } else { Install-FromRelease }
Remove-LegacyBinaryIfPresent

Write-Host "==> Installed loong to $primaryBinary"

$normalizedPrefix = $Prefix
$pathItems = ($env:PATH -split [IO.Path]::PathSeparator) |
    Where-Object { $_ } |
    ForEach-Object { Resolve-NormalizedPathEntryOrNull $_ } |
    Where-Object { $_ }
$alreadyInSessionPath = $pathItems | Where-Object { $_ -ieq $normalizedPrefix }
if (-not $alreadyInSessionPath) {
    $currentUserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    $userPathItems = if ($currentUserPath) {
        ($currentUserPath -split [IO.Path]::PathSeparator) |
            Where-Object { $_ } |
            ForEach-Object { Resolve-NormalizedPathEntryOrNull $_ } |
            Where-Object { $_ }
    } else { @() }
    $alreadyInUserPath = $userPathItems | Where-Object { $_ -ieq $normalizedPrefix }
    if (-not $alreadyInUserPath) {
        $newUserPath = if ($currentUserPath) { "$normalizedPrefix$([IO.Path]::PathSeparator)$currentUserPath" } else { $normalizedPrefix }
        try {
            [Environment]::SetEnvironmentVariable("PATH", $newUserPath, "User")
            Write-Host "==> Added $normalizedPrefix to user PATH"
        } catch {
            Write-Host "==> Could not persist PATH automatically: $_"
            Write-Host "    Add manually: `$env:PATH = `"$normalizedPrefix`$([IO.Path]::PathSeparator)`$env:PATH`""
        }
    } else {
        Write-Host "==> PATH entry already present in user environment"
    }
    $env:PATH = "$normalizedPrefix$([IO.Path]::PathSeparator)$env:PATH"
}

if ($Onboard) {
    Write-Host "==> Running guided onboarding"
    try {
        & $primaryBinary onboard | Out-Host
        if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
            Write-Host "==> Onboarding exited with code $LASTEXITCODE"
            Write-Host "==> You can run 'loong onboard' later to complete setup"
        }
    } catch {
        Write-Host "==> Onboarding encountered an error: $_"
        Write-Host "==> You can run 'loong onboard' later to complete setup"
    }
}

Write-Host ""
Write-Host "Done. Try:"
Write-Host "  loong --help"
