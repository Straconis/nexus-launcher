$ErrorActionPreference = 'Stop'
$fixture = Join-Path $env:RUNNER_TEMP "nexus-stat-tracker-$([guid]::NewGuid())"
$setup = Join-Path $fixture 'StatTrackerSetup-1.0.0.exe'
$env:NEXUS_TEST_STAT_TRACKER_DIR = Join-Path $fixture 'Custom Stat Tracker Directory'
New-Item -ItemType Directory -Path $fixture | Out-Null
try {
    Invoke-WebRequest 'https://github.com/Straconis/stat_tracker/releases/download/v1.0.0/StatTrackerSetup-1.0.0.exe' -OutFile $setup
    $expected = 'cb65aa959213c4a70e0d7a738a5aec5f87dcda15e9d6d90511b30bdca1f22f56'
    if ((Get-FileHash -Algorithm SHA256 $setup).Hash -ne $expected) {
        throw 'Stat Tracker installer checksum did not match'
    }
    $result = Start-Process -FilePath $setup -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
        "/DIR=`"$env:NEXUS_TEST_STAT_TRACKER_DIR`""
    ) -Wait -PassThru
    if ($result.ExitCode -ne 0) { throw "Stat Tracker setup failed: $($result.ExitCode)" }
    cargo test --locked -p nexus-launcher --lib detects_stat_tracker_after_windows_setup -- --ignored
    if ($LASTEXITCODE -ne 0) { throw 'Stat Tracker detection test failed' }
} finally {
    $uninstall = Join-Path $env:NEXUS_TEST_STAT_TRACKER_DIR 'unins000.exe'
    if (Test-Path $uninstall) {
        Start-Process -FilePath $uninstall -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait
    }
    Remove-Item -Recurse -Force $fixture
}
