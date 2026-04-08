param(
    [string]$Prefix = "$HOME/.local/bin",
    [switch]$Onboard,
    [string]$Version = $(if ($env:LOONGCLAW_INSTALL_VERSION) { $env:LOONGCLAW_INSTALL_VERSION } else { "latest" }),
    [switch]$Source,
    [string]$Repository = $(if ($env:LOONGCLAW_INSTALL_REPO) { $env:LOONGCLAW_INSTALL_REPO } else { "loongclaw-ai/loongclaw" })
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$Prefix = [IO.Path]::GetFullPath(($Prefix -replace '^~', $HOME))
$ReleaseBaseUrl = if ($env:LOONGCLAW_INSTALL_RELEASE_BASE_URL) { $env:LOONGCLAW_INSTALL_RELEASE_BASE_URL } else { "https://github.com/$Repository/releases" }
$BinName = "loong"
$LegacyBinName = "loongclaw"

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

function Install-CompatibilityBinaries([string]$SourceBinary) {
    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    $primaryBinary = Join-Path $Prefix "$BinName.exe"
    $legacyBinary = Join-Path $Prefix "$LegacyBinName.exe"
    Copy-Item -Force $SourceBinary $primaryBinary
    Copy-Item -Force $SourceBinary $legacyBinary
    return @{
        Primary = $primaryBinary
        Legacy = $legacyBinary
    }
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

    Write-Host "==> Building loong from source (release)"
    Push-Location $repoRoot
    $hadReleaseBuild = Test-Path Env:LOONGCLAW_RELEASE_BUILD
    $previousReleaseBuild = $env:LOONGCLAW_RELEASE_BUILD
    try {
        $env:LOONGCLAW_RELEASE_BUILD = "1"
        cargo build -p loongclaw --bin $BinName --release --locked | Out-Host
    } finally {
        if ($hadReleaseBuild) {
            $env:LOONGCLAW_RELEASE_BUILD = $previousReleaseBuild
        } elseif (Test-Path Env:LOONGCLAW_RELEASE_BUILD) {
            Remove-Item Env:LOONGCLAW_RELEASE_BUILD
        }
        Pop-Location
    }

    $sourceBinary = Join-Path $repoRoot "target/release/$BinName.exe"
    if (-not (Test-Path $sourceBinary)) {
        throw "built binary not found at $sourceBinary"
    }

    return Install-CompatibilityBinaries -SourceBinary $sourceBinary
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
        $sourceBinary = Join-Path $extractRoot "$BinName.exe"
        if (-not (Test-Path $sourceBinary)) {
            throw "extracted binary not found at $sourceBinary"
        }

        return Install-CompatibilityBinaries -SourceBinary $sourceBinary
    } finally {
        if (Test-Path $tmpRoot) {
            Remove-Item -Recurse -Force $tmpRoot
        }
    }
}

$installResult = if ($Source) { Install-FromSource } else { Install-FromRelease }

Write-Host "==> Installed loong to $($installResult.Primary)"
Write-Host "==> Installed compatible loongclaw command to $($installResult.Legacy)"

$normalizedPrefix = $Prefix
$pathItems = ($env:PATH -split [IO.Path]::PathSeparator) |
    Where-Object { $_ } |
    ForEach-Object { [IO.Path]::GetFullPath($_) }
$alreadyInSessionPath = $pathItems | Where-Object { $_ -ieq $normalizedPrefix }
if (-not $alreadyInSessionPath) {
    $currentUserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    $userPathItems = if ($currentUserPath) {
        ($currentUserPath -split [IO.Path]::PathSeparator) |
            Where-Object { $_ } |
            ForEach-Object { [IO.Path]::GetFullPath($_) }
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
        & $installResult.Primary onboard | Out-Host
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
