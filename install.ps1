<# 
.SYNOPSIS
    Uteke Windows installer — installs uteke CLI, server, and MCP binaries
.DESCRIPTION
    Downloads and installs the latest uteke release for Windows x64 from GitHub.
    Usage: Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass; irm https://raw.githubusercontent.com/codecoradev/uteke/main/install.ps1 | iex
    Or:    powershell -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/codecoradev/uteke/main/install.ps1 | iex"
#>

[CmdletBinding()]
param(
    [Parameter()]
    [string]$Version = "",
    
    [Parameter()]
    [string]$InstallDir = "$env:USERPROFILE\.local\bin",
    
    [Parameter()]
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"

function Write-Info { param([string]$msg) Write-Host "[INFO] $msg" -ForegroundColor Green }
function Write-Warn { param([string]$msg) Write-Host "[WARN] $msg" -ForegroundColor Yellow }
function Write-ErrorMsg { param([string]$msg) Write-Host "[ERROR] $msg" -ForegroundColor Red; exit 1 }

$REPO = "codecoradev/uteke"
$BINARIES = @("uteke.exe", "uteke-serve.exe", "uteke-mcp.exe")

# Use gh token for authenticated API requests if available
$env:GH_TOKEN = $env:GH_TOKEN
if (-not $env:GH_TOKEN) {
    try { $env:GH_TOKEN = gh auth token 2>$null } catch {}
}

function Invoke-GitHubApi {
    param([string]$Uri)
    $params = @{ Uri = $Uri; UseBasicParsing = $true }
    if ($env:GH_TOKEN) {
        $params.Headers = @{ Authorization = "Bearer $env:GH_TOKEN" }
    }
    return Invoke-WebRequest @params
}

# Detect architecture
$ARCH = [Environment]::GetEnvironmentVariable("PROCESSOR_ARCHITECTURE")
if ($ARCH -eq "AMD64" -or $ARCH -eq "x86_64") {
    $TARGET = "x86_64-pc-windows-msvc"
    $ARTIFACT = "uteke-x86_64-pc-windows-msvc"
} elseif ($ARCH -eq "ARM64") {
    Write-ErrorMsg "ARM64 Windows builds not yet published. Install via cargo: cargo install --path crates/uteke-cli"
} else {
    Write-ErrorMsg "Unsupported architecture: $ARCH"
}

# Get latest version from GitHub
function Get-LatestVersion {
    Write-Info "Fetching latest release from GitHub..."
    try {
        $apiUrl = "https://api.github.com/repos/$REPO/releases?per_page=30"
        $response = Invoke-GitHubApi -Uri $apiUrl
        $releases = $response.Content | ConvertFrom-Json
        foreach ($release in $releases) {
            if ($release.assets.Count -gt 0) {
                foreach ($asset in $release.assets) {
                    if ($asset.name -like "*x86_64-pc-windows-msvc*") {
                        Write-Info "Found version: $($release.tag_name)"
                        return $release.tag_name
                    }
                }
            }
        }
        Write-ErrorMsg "No release with Windows binary found on GitHub."
    } catch {
        Write-Warn "GitHub API failed: $($_.Exception.Message)"
        Write-ErrorMsg "Failed to get latest version. Set -Version v0.10.0 to pin a version."
    }
}

# Download and verify
function Install-Uteke {
    param([string]$Version)
    
    Write-Info "Detected: Windows $ARCH"
    Write-Info "Target: $TARGET"
    Write-Info "Version: $Version"
    
    $archiveName = "${ARTIFACT}-${Version}.zip"
    $downloadUrl = "https://github.com/$REPO/releases/download/$Version/$archiveName"
    $checksumsUrl = "https://github.com/$REPO/releases/download/$Version/checksums-sha256.txt"
    
    $tempDir = [System.IO.Path]::GetTempPath()
    $archivePath = Join-Path $tempDir $archiveName
    $checksumPath = Join-Path $tempDir "checksums-sha256.txt"
    $extractDir = Join-Path $tempDir "uteke-extract"
    
    Write-Info "Downloading from: $downloadUrl"
    try {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath -ErrorAction Stop
    } catch {
        Write-ErrorMsg "Failed to download $archiveName. Check version exists: https://github.com/$REPO/releases/tag/$Version"
    }
    
    # Download and verify checksum
    Write-Info "Downloading checksums..."
    try {
        Invoke-WebRequest -Uri $checksumsUrl -OutFile $checksumPath -ErrorAction Stop
        $checksums = Get-Content $checksumPath -Raw
        # Split on whitespace (handles both spaces and tabs)
        $expectedHash = ($checksums -split "`n" | Where-Object { $_ -like "*$archiveName*" } | ForEach-Object { $_ -split '\s+' })[0]
        if ($expectedHash) {
            Write-Info "Verifying SHA256 checksum..."
            $actualHash = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLower()
            if ($actualHash -ne $expectedHash.ToLower()) {
                Write-ErrorMsg "Checksum mismatch! Expected: $expectedHash, Got: $actualHash"
                exit 1
            }
            Write-Info "Checksum verified: $expectedHash"
        } else {
            Write-Warn "Checksum for $archiveName not found in checksums file"
        }
    } catch {
        Write-Warn "Failed to download/verify checksums - skipping verification"
    }
    
    # Extract
    Write-Info "Extracting..."
    if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
    New-Item -ItemType Directory -Path $extractDir | Out-Null
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force
    
    # Install binaries
    Write-Info "Installing to $InstallDir..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    
    foreach ($binary in $BINARIES) {
        $src = Join-Path $extractDir $binary
        $dest = Join-Path $InstallDir $binary
        if (Test-Path $src) {
            Copy-Item $src $dest -Force
            Write-Info "Installed $binary"
        } else {
            Write-Warn "$binary not found in archive"
        }
    }

    # Install bundled ONNX Runtime DLLs (required for embeddings).
    # Release zips ship onnxruntime.dll + providers DLLs alongside the exes.
    # Without these, `uteke remember` fails with "ONNX Runtime library not found".
    Get-ChildItem $extractDir -Filter "onnxruntime*.dll" -ErrorAction SilentlyContinue | ForEach-Object {
        Copy-Item $_.FullName (Join-Path $InstallDir $_.Name) -Force
        Write-Info ("Installed {0}" -f $_.Name)
    }
    
    # Cleanup
    Remove-Item $archivePath -Force -ErrorAction SilentlyContinue
    Remove-Item $checksumPath -Force -ErrorAction SilentlyContinue
    Remove-Item $extractDir -Recurse -Force -ErrorAction SilentlyContinue
}

# Update PATH
function Update-Path {
    param([string]$InstallDir)
    
    if ($NoPathUpdate) {
        Write-Warn 'Skipping PATH update (--NoPathUpdate specified)'
        return
    }
    
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -split ";" | Where-Object { $_ -eq $InstallDir }) {
        Write-Info "$InstallDir already in user PATH"
        return
    }
    
    Write-Info "Adding $InstallDir to user PATH..."
    $newPath = $currentPath + ";" + $InstallDir
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Warn "PATH updated. Restart your terminal or run: `$env:PATH = [Environment]::GetEnvironmentVariable('Path','User')"
}

# Verify installation
function Verify-Install {
    param([string]$InstallDir)
    
    Write-Info "Verifying installation..."
    $versionFlags = @{
        "uteke.exe" = "--version"
        "uteke-serve.exe" = "--help"
        "uteke-mcp.exe" = "--help"
    }
    foreach ($binary in $BINARIES) {
        $path = Join-Path $InstallDir $binary
        if (Test-Path $path) {
            try {
                $flag = $versionFlags[$binary]
                $version = & $path $flag 2>&1 | Select-Object -First 1
                Write-Info ("  {0}: {1}" -f $binary, $version)
            } catch {
                Write-Info ("  {0}: installed" -f $binary)
            }
        } else {
            Write-Warn ("  {0}: NOT FOUND" -f $binary)
        }
    }
}

# Main
Write-Info "Installing uteke..."

if ($Version) {
    Write-Info "Using pinned version: $Version"
} else {
    $Version = Get-LatestVersion
    if (-not $Version) { Write-ErrorMsg "Could not determine version" }
}

Install-Uteke -Version $Version
Update-Path -InstallDir $InstallDir
Verify-Install -InstallDir $InstallDir

Write-Host ""
Write-Info "Installation complete!"
Write-Host "Run 'uteke --help' to get started." -ForegroundColor Cyan
Write-Host ""
Write-Host "Quick start:" -ForegroundColor Cyan
Write-Host "  uteke remember \"Deploy v2.1 to staging at 3pm\""
Write-Host "  uteke recall \"when do we deploy?\""
Write-Host ""
Write-Host "MCP Server (for AI agents):" -ForegroundColor Cyan
Write-Host "  uteke-mcp"