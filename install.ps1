# BlamePrompt installer for Windows
# Usage: irm https://blameprompt.com/install.ps1 | iex
$ErrorActionPreference = "Stop"

$Version = "0.1.0"
$Repo = "ekaanth/blameprompt"
$BinaryName = "blameprompt"

# Install directory
$InstallDir = if ($env:BLAMEPROMPT_INSTALL_DIR) { $env:BLAMEPROMPT_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }

function Write-Info($msg) { Write-Host "  [info] " -ForegroundColor Green -NoNewline; Write-Host $msg }
function Write-Err($msg) { Write-Host "  [error] " -ForegroundColor Red -NoNewline; Write-Host $msg; exit 1 }

Write-Host ""
Write-Host "  BlamePrompt Installer" -ForegroundColor Cyan -NoNewline
Write-Host " v$Version" -ForegroundColor DarkGray
Write-Host "  Track AI-generated code in Git" -ForegroundColor DarkGray
Write-Host ""

# Detect architecture
$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
switch ($Arch) {
    "X64"   { $Target = "x86_64-pc-windows-msvc" }
    "Arm64" { $Target = "aarch64-pc-windows-msvc" }
    default { Write-Err "Unsupported architecture: $Arch" }
}

Write-Info "Detected platform: windows/$Arch"

# Build download URL
$Tarball = "$BinaryName-v$Version-$Target.tar.gz"
$Url = "https://github.com/$Repo/releases/download/v$Version/$Tarball"

# Download
$TmpDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "blameprompt-install-$(Get-Random)")
$TarPath = Join-Path $TmpDir $Tarball

Write-Info "Downloading $Tarball..."
try {
    Invoke-WebRequest -Uri $Url -OutFile $TarPath -UseBasicParsing
} catch {
    Write-Err "Download failed. Check https://github.com/$Repo/releases for available builds."
}

# Extract
Write-Info "Extracting..."
tar -xzf $TarPath -C $TmpDir

# Install
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$BinaryPath = Join-Path $InstallDir "$BinaryName.exe"
Move-Item -Force (Join-Path $TmpDir "$BinaryName.exe") $BinaryPath

Write-Info "Installed to $BinaryPath"

# Clean up
Remove-Item -Recurse -Force $TmpDir

# Check if InstallDir is in PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host ""
    Write-Host "  Add to your PATH:" -ForegroundColor White
    Write-Host "    [Environment]::SetEnvironmentVariable('Path', `"$InstallDir;`$env:Path`", 'User')" -ForegroundColor Cyan
    Write-Host ""

    $AddToPath = Read-Host "  Add to PATH now? [Y/n]"
    if ($AddToPath -ne "n") {
        [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$UserPath", "User")
        $env:Path = "$InstallDir;$env:Path"
        Write-Info "Added $InstallDir to PATH"
    }
}

# Run global init
Write-Host ""
Write-Info "Running global setup..."
& $BinaryPath init --global

Write-Host ""
Write-Host "  Done!" -ForegroundColor Green -NoNewline
Write-Host " BlamePrompt v$Version is ready."
Write-Host ""
