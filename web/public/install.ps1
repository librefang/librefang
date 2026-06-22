# LibreFang installer for Windows
# Usage: iwr -useb https://librefang.ai/install.ps1 | iex
#   or:  powershell -c "irm https://librefang.ai/install.ps1 | iex"
#
# Flags (via environment variables):
#   $env:LIBREFANG_INSTALL_DIR         = custom install directory (default: ~/.librefang/bin)
#   $env:LIBREFANG_VERSION             = specific version tag (e.g. "v0.1.0")
#   $env:LIBREFANG_AUTO_START          = auto-start daemon after install (default: 1)
#                                        accepts: 1/true/yes/on (others disable)
#   $env:LIBREFANG_INSTALLER_SOURCE_ONLY = test hook; do not auto-run Install-LibreFang

$ErrorActionPreference = 'Stop'

$Repo = "librefang/librefang"
$DefaultInstallDir = Join-Path $env:USERPROFILE ".librefang\bin"
$InstallDir = if ($env:LIBREFANG_INSTALL_DIR) { $env:LIBREFANG_INSTALL_DIR } else { $DefaultInstallDir }

function Write-Banner {
    Write-Host ""
    Write-Host "  LibreFang Installer" -ForegroundColor Cyan
    Write-Host "  ===================" -ForegroundColor Cyan
    Write-Host ""
}

function Test-Enabled {
    param([string]$Value)
    if ([string]::IsNullOrWhiteSpace($Value)) { return $false }
    switch ($Value.Trim().ToLowerInvariant()) {
        "1" { return $true }
        "true" { return $true }
        "yes" { return $true }
        "on" { return $true }
        default { return $false }
    }
}

function Start-DaemonIfNeeded {
    param([string]$InstalledExe)

    $startOutput = & $InstalledExe start 2>&1
    $startExitCode = $LASTEXITCODE

    if ($startOutput) {
        $startOutput | ForEach-Object { Write-Host $_ }
    }

    if ($startExitCode -eq 0) {
        return $true
    }

    $startOutputText = ($startOutput | Out-String)
    if ($startOutputText -match '(?i)already running') {
        Write-Host "  Daemon already running; leaving it as-is." -ForegroundColor Yellow
        return $true
    }

    return $false
}

function Get-Architecture {
    # Try multiple detection methods — piped iex can break some approaches
    $arch = ""

    # Method 1: .NET RuntimeInformation
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {}

    # Method 2: PROCESSOR_ARCHITECTURE env var
    if (-not $arch -or $arch -eq "") {
        try { $arch = $env:PROCESSOR_ARCHITECTURE } catch {}
    }

    # Method 3: WMI
    if (-not $arch -or $arch -eq "") {
        try {
            $wmiArch = (Get-CimInstance Win32_Processor).Architecture
            if ($wmiArch -eq 9) { $arch = "AMD64" }
            elseif ($wmiArch -eq 12) { $arch = "ARM64" }
        } catch {}
    }

    # Method 4: pointer size fallback (64-bit = 8 bytes)
    if (-not $arch -or $arch -eq "") {
        if ([IntPtr]::Size -eq 8) { $arch = "X64" }
    }

    $archUpper = "$arch".ToUpper().Trim()
    switch ($archUpper) {
        { $_ -in "X64", "AMD64", "X86_64" }     { return "x86_64" }
        { $_ -in "ARM64", "AARCH64", "ARM" }     { return "aarch64" }
        default {
            Write-Host "  Unsupported architecture: $arch (detection may have failed)" -ForegroundColor Red
            Write-Host "  Try: cargo install --git https://github.com/$Repo librefang-cli" -ForegroundColor Yellow
            exit 1
        }
    }
}

# Probe the .zip and .sha256 for a version+target; returns $false for "stuck" releases (assets still uploading).
function Test-AssetExists {
    param([string]$Version, [string]$Target)
    $base = "https://github.com/$Repo/releases/download/$Version/librefang-$Target.zip"
    try {
        Invoke-WebRequest -Uri $base -Method Head -UseBasicParsing -ErrorAction Stop | Out-Null
        Invoke-WebRequest -Uri "$base.sha256" -Method Head -UseBasicParsing -ErrorAction Stop | Out-Null
        return $true
    }
    catch {
        return $false
    }
}

# Resolve the version to install: LIBREFANG_VERSION is a hard pin; LIBREFANG_PREFERRED_VERSION is a soft hint with fallback.
function Resolve-InstallableVersion {
    param([string]$Target)

    if ($env:LIBREFANG_VERSION) {
        Write-Host "  Using specified version: $($env:LIBREFANG_VERSION)"
        return $env:LIBREFANG_VERSION
    }

    $preferred = $env:LIBREFANG_PREFERRED_VERSION
    if ($preferred -and (Test-AssetExists -Version $preferred -Target $Target)) {
        return $preferred
    }

    Write-Host "  Fetching latest release..."
    try {
        $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=30"
    }
    catch {
        return $null
    }

    $zip = "librefang-$Target.zip"
    $scanned = 0
    foreach ($rel in $releases) {
        if ($rel.draft) { continue }
        $scanned++
        if ($scanned -gt 10) { break }
        $tag = $rel.tag_name
        if (-not $tag) { continue }
        $names = @($rel.assets | ForEach-Object { $_.name })
        if (($names -contains $zip) -and ($names -contains "$zip.sha256")) {
            if ($preferred -and ($tag -ne $preferred)) {
                Write-Host "  Release $preferred has no $Target package yet; falling back to $tag." -ForegroundColor Yellow
            }
            elseif ($scanned -gt 1) {
                Write-Host "  Newest release has no $Target package yet; using $tag." -ForegroundColor Yellow
            }
            return $tag
        }
    }
    return $null
}

# Atomically replace $Dest with $Source, rolling back on failure; returns $true on success.
function Install-WithRollback {
    param([string]$Source, [string]$Dest)

    $backup = "$Dest.bak"
    $hadExisting = Test-Path $Dest
    if ($hadExisting) {
        if (Test-Path $backup) { Remove-Item -Force $backup -ErrorAction SilentlyContinue }
        Copy-Item -Path $Dest -Destination $backup -Force
    }

    try {
        Copy-Item -Path $Source -Destination $Dest -Force -ErrorAction Stop
    }
    catch {
        if ($hadExisting) { Move-Item -Path $backup -Destination $Dest -Force }
        Write-Host "  Could not install the new binary to $Dest." -ForegroundColor Red
        return $false
    }

    $ok = $false
    try {
        & $Dest --version *> $null
        if ($LASTEXITCODE -eq 0) { $ok = $true }
    }
    catch {
        $ok = $false
    }

    if (-not $ok) {
        if ($hadExisting) {
            Move-Item -Path $backup -Destination $Dest -Force
            Write-Host "  The new binary failed to run; rolled back to the previous version." -ForegroundColor Red
        }
        else {
            Write-Host "  The new binary failed to run." -ForegroundColor Red
        }
        return $false
    }

    if ($hadExisting -and (Test-Path $backup)) { Remove-Item -Force $backup -ErrorAction SilentlyContinue }
    return $true
}

function Install-LibreFang {
    Write-Banner

    $arch = Get-Architecture
    $target = "${arch}-pc-windows-msvc"
    $version = Resolve-InstallableVersion -Target $target
    if (-not $version) {
        Write-Host "  No installable release with a $target package was found." -ForegroundColor Red
        Write-Host "  The latest release may still be building its assets, or none is" -ForegroundColor Yellow
        Write-Host "  published for $Repo yet. Install from source instead:" -ForegroundColor Yellow
        Write-Host "    cargo install --git https://github.com/$Repo librefang-cli"
        exit 1
    }
    $archive = "librefang-${target}.zip"
    $url = "https://github.com/$Repo/releases/download/$version/$archive"
    $checksumUrl = "$url.sha256"

    Write-Host "  Installing LibreFang $version for $target..."

    # Create install directory
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    # Download to temp
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "librefang-install"
    if (Test-Path $tempDir) { Remove-Item -Recurse -Force $tempDir }
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    $archivePath = Join-Path $tempDir $archive
    $checksumPath = Join-Path $tempDir "$archive.sha256"

    try {
        Invoke-WebRequest -Uri $url -OutFile $archivePath -UseBasicParsing
    }
    catch {
        Write-Host "  Download failed. The release may not exist for your platform." -ForegroundColor Red
        Write-Host "  Install from source instead:" -ForegroundColor Yellow
        Write-Host "    cargo install --git https://github.com/$Repo librefang-cli"
        Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
        exit 1
    }

    # Verify checksum if available
    $checksumDownloaded = $false
    try {
        Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath -UseBasicParsing
        $checksumDownloaded = $true
    }
    catch {
        Write-Host "  Checksum file not available, skipping verification." -ForegroundColor Yellow
    }
    if ($checksumDownloaded) {
        $expectedHash = (Get-Content $checksumPath -Raw).Split(" ")[0].Trim().ToLower()
        $actualHash = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
        if ($expectedHash -ne $actualHash) {
            Write-Host "  Checksum verification FAILED!" -ForegroundColor Red
            Write-Host "    Expected: $expectedHash" -ForegroundColor Red
            Write-Host "    Got:      $actualHash" -ForegroundColor Red
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
            exit 1
        }
        Write-Host "  Checksum verified." -ForegroundColor Green
    }

    # Extract
    Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force
    $exePath = Join-Path $tempDir "librefang.exe"
    if (-not (Test-Path $exePath)) {
        # May be nested in a directory
        $found = Get-ChildItem -Path $tempDir -Filter "librefang.exe" -Recurse | Select-Object -First 1
        if ($found) {
            $exePath = $found.FullName
        }
        else {
            Write-Host "  Could not find librefang.exe in archive." -ForegroundColor Red
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
            exit 1
        }
    }

    # Atomic install with rollback: a failing new binary restores the previous version.
    if (-not (Install-WithRollback -Source $exePath -Dest (Join-Path $InstallDir "librefang.exe"))) {
        Write-Host "  Install from source instead:" -ForegroundColor Yellow
        Write-Host "    cargo install --git https://github.com/$Repo librefang-cli"
        Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
        exit 1
    }

    # The Rust Telegram sidecar binary ships inside the same archive since the
    # release pipeline bundles it. Older archives lack it, so install it only
    # when present and stay silent otherwise (backward compatible).
    $sidecarPath = Join-Path $tempDir "librefang-sidecar-telegram.exe"
    if (-not (Test-Path $sidecarPath)) {
        $foundSidecar = Get-ChildItem -Path $tempDir -Filter "librefang-sidecar-telegram.exe" -Recurse | Select-Object -First 1
        if ($foundSidecar) { $sidecarPath = $foundSidecar.FullName } else { $sidecarPath = $null }
    }
    if ($sidecarPath) {
        Copy-Item -Path $sidecarPath -Destination (Join-Path $InstallDir "librefang-sidecar-telegram.exe") -Force
    }

    # Clean up temp
    Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue

    # Add to user PATH if not already present
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($null -eq $currentPath) { $currentPath = "" }
    $userPathEntries = @()
    if (-not [string]::IsNullOrWhiteSpace($currentPath)) {
        $userPathEntries = $currentPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }
    $hasInstallDirInUserPath = @($userPathEntries | Where-Object {
        $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\')
    }).Count -gt 0

    if (-not $hasInstallDirInUserPath) {
        $newUserPath = if ([string]::IsNullOrWhiteSpace($currentPath)) { $InstallDir } else { "$InstallDir;$currentPath" }
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
        Write-Host "  Added $InstallDir to user PATH." -ForegroundColor Green
    }

    $sessionNeedsPathRefresh = -not (($env:Path -split ';') | Where-Object {
        $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\')
    })

    # Verify
    $installedExe = Join-Path $InstallDir "librefang.exe"
    if (Test-Path $installedExe) {
        try {
            $versionOutput = & $installedExe --version 2>&1
            Write-Host ""
            Write-Host "  LibreFang installed successfully! ($versionOutput)" -ForegroundColor Green
        }
        catch {
            Write-Host ""
            Write-Host "  LibreFang binary installed to $installedExe" -ForegroundColor Green
        }
    }

    Write-Host ""
    Write-Host "  Get started now:" -ForegroundColor Cyan
    Write-Host "    $installedExe init"
    if ($sessionNeedsPathRefresh) {
        Write-Host ""
        Write-Host "  To use 'librefang' in this PowerShell session, run:" -ForegroundColor Yellow
        Write-Host ('    $env:Path = "{0};$env:Path"' -f $InstallDir)
        Write-Host "  New terminals will pick it up automatically." -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  After refreshing PATH, you can also run:" -ForegroundColor Cyan
        Write-Host "    librefang init"
    }
    else {
        Write-Host ""
        Write-Host "  Or run:" -ForegroundColor Cyan
        Write-Host "    librefang init"
    }
    Write-Host ""
    Write-Host "  The setup wizard will guide you through provider selection"
    Write-Host "  and configuration."
    Write-Host ""

    # Auto-initialize (sync registry, generate config)
    Write-Host "  Initializing LibreFang..." -ForegroundColor Cyan
    try {
        & $installedExe init 2>&1 | Out-Null
    } catch {}

    $autoStartRaw = if ($env:LIBREFANG_AUTO_START) { $env:LIBREFANG_AUTO_START } else { "1" }
    if (Test-Enabled $autoStartRaw) {
        # Register boot service so LibreFang starts on login/reboot
        Write-Host "  Registering boot service..." -ForegroundColor Cyan
        try { & $installedExe service install 2>&1 | Out-Null } catch {}

        Write-Host "  Starting daemon in background..." -ForegroundColor Cyan
        if (Start-DaemonIfNeeded -InstalledExe $installedExe) {
            Write-Host ""
            Write-Host "  Next steps:" -ForegroundColor Cyan
            Write-Host "    1. Chat:              $installedExe chat"
            Write-Host "    2. Stop daemon:       $installedExe stop"
        }
        else {
            Write-Host ""
            Write-Host "  Warning: automatic daemon start failed." -ForegroundColor Yellow
            Write-Host "  Start it manually with:" -ForegroundColor Yellow
            Write-Host "    $installedExe start"
        }
        Write-Host ""
    }
}

if ($env:LIBREFANG_INSTALLER_SOURCE_ONLY -eq "1") {
    return
}

Install-LibreFang
