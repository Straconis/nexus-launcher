param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$Installer = (Resolve-Path $Installer).Path
$installRoot = Join-Path $env:RUNNER_TEMP ("nexus-installer-" + [guid]::NewGuid())
$shortcutPath = Join-Path ([Environment]::GetFolderPath("Programs")) "Straconis Software/Straconis Software Dev Nexus Launcher.lnk"
$exe = Join-Path $installRoot "nexus-launcher.exe"
$catalog = Join-Path $installRoot "catalog/apps.toml"
$stateRoot = Join-Path ([Environment]::GetFolderPath("ApplicationData")) "Straconis/nexus-launcher"
$sentinel = Join-Path $stateRoot "data/apps/installer-smoke-test/keep.txt"
New-Item -ItemType Directory -Force -Path (Split-Path $sentinel) | Out-Null
Set-Content $sentinel "Preserve installed tools and user data"
function Install-Launcher {
    $process = Start-Process -FilePath $Installer -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/SP-", "/NORESTART", ('/DIR="{0}"' -f $installRoot), ('/LOG="{0}"' -f (Join-Path $env:RUNNER_TEMP "nexus-install.log"))) -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
}
Install-Launcher
if (-not (Test-Path $exe)) { throw "Installed binary missing" }
if (-not (Test-Path $shortcutPath)) { throw "Requested Start Menu entry missing" }
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
if ($shortcut.TargetPath -ne $exe) { throw "Shortcut has the wrong target" }
if ($shortcut.WorkingDirectory -ne $installRoot) { throw "Shortcut has the wrong working directory" }
if ($shortcut.IconLocation -notlike "$(Join-Path $installRoot 'nexus-launcher.ico')*") { throw "Shortcut has the wrong icon" }
$process = Start-Process -FilePath $exe -ArgumentList "--check" -WorkingDirectory $env:RUNNER_TEMP -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Installed CLI catalog smoke check failed" }
Add-Content $catalog "`n# installer-upgrade-preserve-check"
Install-Launcher
if ((Get-Content $catalog -Raw) -notmatch "installer-upgrade-preserve-check") { throw "Reinstall overwrote the edited catalog" }
$uninstaller = Join-Path $installRoot "unins000.exe"
$process = Start-Process -FilePath $uninstaller -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART") -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Uninstaller failed: $($process.ExitCode)" }
if (Test-Path $exe) { throw "Uninstall left the binary behind" }
if (Test-Path $shortcutPath) { throw "Uninstall left the Start Menu entry behind" }
if (-not (Test-Path $sentinel)) { throw "Uninstall removed user data" }
if ((Get-Content $catalog -Raw) -notmatch "installer-upgrade-preserve-check") { throw "Uninstall removed the edited catalog" }
Write-Output "Verified install, exact Start Menu entry, catalog lookup, reinstall preservation, and uninstall cleanup."
