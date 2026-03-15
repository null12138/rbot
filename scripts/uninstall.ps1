$ErrorActionPreference = "Stop"

$RbotHome = if ($env:RBOT_HOME) { $env:RBOT_HOME } else { Join-Path $env:USERPROFILE ".rbot" }
$BinDir = if ($env:RBOT_BIN_DIR) { $env:RBOT_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "rbot\bin" }
$KeepConfig = if ($env:RBOT_KEEP_CONFIG) { $true } else { $false }

$cmdPath = Join-Path $BinDir "rbot.cmd"
if (Test-Path $cmdPath) { Remove-Item $cmdPath -Force }

$exePath = Join-Path $RbotHome "bin\rbot.exe"
if (Test-Path $exePath) { Remove-Item $exePath -Force }

if ($KeepConfig) {
  Write-Host "Uninstall complete. Kept config/data at $RbotHome (RBOT_KEEP_CONFIG set)."
  exit 0
}

if (Test-Path $RbotHome) { Remove-Item $RbotHome -Recurse -Force }

Write-Host "Uninstall complete."
