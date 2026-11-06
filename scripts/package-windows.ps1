param([string]$Binary = "target/release/nexus-launcher.exe")
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not (Test-Path $Binary)) { throw "Missing launcher binary: $Binary" }
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Cannot read Cargo metadata" }
    $version = ($metadata.packages | Where-Object name -EQ "nexus-launcher").version
    $numeric = ($version -split '-')[0] + ".0"
    $name = "nexus-launcher-windows-x86_64"
    $stage = Join-Path "dist" $name
    New-Item -ItemType Directory -Force -Path (Join-Path $stage "catalog") | Out-Null
    Copy-Item $Binary (Join-Path $stage "nexus-launcher.exe")
    Copy-Item "assets/branding/nexus-launcher.ico" (Join-Path $stage "nexus-launcher.ico")
    Copy-Item "catalog/apps.toml" (Join-Path $stage "catalog/apps.toml")
    Copy-Item "README.md","LICENSE-MIT","LICENSE-APACHE","packaging/PORTABLE.txt" $stage
    Copy-Item "assets/fonts/OFL.txt" (Join-Path $stage "OFL-Orbitron.txt")
    Compress-Archive -Path $stage -DestinationPath "dist/$name.zip" -Force
    $command = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
    $iscc = if ($env:NEXUS_ISCC_PATH) { $env:NEXUS_ISCC_PATH } elseif ($command) { $command.Source } else { "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
    if (-not (Test-Path $iscc)) { throw "Inno Setup 6 is required to build the installer" }
    & $iscc "/DAppVersion=$version" "/DVersionNumbers=$numeric" "packaging/nexus-launcher.iss"
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed" }
    Get-Item "dist/$name.zip", "dist/nexus-launcher-setup-$version-windows-x86_64.exe" | Select-Object Name,Length
} finally { Pop-Location }
