# Pin the compiler and verify its immutable upstream release asset.
$ErrorActionPreference = "Stop"
$version = "6.4.3"
$toolsRoot = Join-Path $env:RUNNER_TEMP "nexus-inno-$version"
$download = Join-Path $env:RUNNER_TEMP "innosetup-$version.exe"
Invoke-WebRequest "https://github.com/jrsoftware/issrc/releases/download/is-6_4_3/innosetup-6.4.3.exe" -OutFile $download
$expected = "f3c42116542c4cc57263c5ba6c4feabfc49fe771f2f98a79d2f7628b8762723b"
if ((Get-FileHash $download -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw "Inno Setup compiler download checksum mismatch" }
$process = Start-Process $download -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/SP-", "/NORESTART", ('/DIR="{0}"' -f $toolsRoot)) -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Compiler setup failed: $($process.ExitCode)" }
$env:NEXUS_ISCC_PATH = Join-Path $toolsRoot "ISCC.exe"
if (-not (Test-Path $env:NEXUS_ISCC_PATH)) { throw "Inno Setup compiler missing after setup" }
if ($env:GITHUB_ENV) { "NEXUS_ISCC_PATH=$env:NEXUS_ISCC_PATH" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append }
Write-Output "Verified Inno Setup $version compiler ready."
