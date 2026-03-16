$ErrorActionPreference = "Stop"

param(
  [Parameter(Position = 0)]
  [string]$Command = "install",
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Args
)

$Repo = if ($env:RBOT_REPO) { $env:RBOT_REPO } else { "null12138/rbot" }
$Version = if ($env:RBOT_VERSION) { $env:RBOT_VERSION } else { "latest" }
$RbotHome = if ($env:RBOT_HOME) { $env:RBOT_HOME } else { Join-Path $env:USERPROFILE ".rbot" }
$BinDir = if ($env:RBOT_BIN_DIR) { $env:RBOT_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "rbot\bin" }
$AppBin = Join-Path $RbotHome "bin"
$KeepConfig = $env:RBOT_KEEP_CONFIG

function Show-Usage {
@"
Usage: rbot.ps1 <command> [args]

Commands:
  install      Install rbot (default)
  update       Update rbot
  uninstall    Uninstall rbot
  run [args]   Run rbot in foreground
  help         Show this help

Env:
  RBOT_REPO, RBOT_VERSION, RBOT_HOME, RBOT_BIN_DIR, RBOT_KEEP_CONFIG
"@ | Write-Host
}

function Ensure-Dirs {
  New-Item -ItemType Directory -Force $RbotHome, $BinDir, $AppBin, (Join-Path $RbotHome "config"), (Join-Path $RbotHome "skills"), (Join-Path $RbotHome "data"), (Join-Path $RbotHome "memory") | Out-Null
}

function Get-Target {
  $arch = $env:PROCESSOR_ARCHITECTURE
  switch ($arch) {
    "AMD64" { return "x86_64-pc-windows-msvc" }
    "ARM64" { throw "windows arm64 releases are not available yet" }
    default { throw "unsupported arch: $arch" }
  }
}

function Get-Release {
  $apiUrl = "https://api.github.com/repos/$Repo/releases/tags/$Version"
  return Invoke-RestMethod -Uri $apiUrl -Headers @{"User-Agent"="rbot-install"}
}

function Install-OrUpdate([string]$Mode) {
  Ensure-Dirs
  $target = Get-Target
  $release = Get-Release
  $assetName = "rbot-$target.zip"
  $asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
  if (-not $asset) {
    throw "no release asset for $target (tag: $Version)"
  }

  $temp = Join-Path $env:TEMP ("rbot-" + $Mode + "-" + [guid]::NewGuid())
  New-Item -ItemType Directory -Force $temp | Out-Null
  $zipPath = Join-Path $temp $assetName

  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath
  Expand-Archive -Path $zipPath -DestinationPath $temp -Force

  $exePath = Join-Path $temp "rbot-$target.exe"
  if (-not (Test-Path $exePath)) {
    $exePath = Join-Path $temp "rbot.exe"
  }
  if (-not (Test-Path $exePath)) {
    throw "extracted binary not found"
  }

  Copy-Item $exePath (Join-Path $AppBin "rbot.exe") -Force

  $cmdPath = Join-Path $BinDir "rbot.cmd"
@"
@echo off
if "%RBOT_HOME%"=="" set RBOT_HOME=$RbotHome
cd /d "%RBOT_HOME%"
"%RBOT_HOME%\bin\rbot.exe" %*
"@ | Set-Content -Path $cmdPath -Encoding ASCII

  if ($Mode -eq "install" -and -not (Test-Path (Join-Path $RbotHome "config\config.toml"))) {
    Write-Host "Running rbot init..."
    Push-Location $RbotHome
    & (Join-Path $AppBin "rbot.exe") init
    Pop-Location
  }

  $pathParts = $env:PATH -split ";"
  if ($pathParts -notcontains $BinDir) {
    Write-Host "NOTE: add $BinDir to your PATH to use 'rbot'"
  }

  if ($Mode -eq "install") {
    Write-Host "Install complete. Run: rbot"
  } else {
    Write-Host "Update complete. Run: rbot"
  }
}

function Uninstall-Rbot {
  $cmdPath = Join-Path $BinDir "rbot.cmd"
  if (Test-Path $cmdPath) { Remove-Item $cmdPath -Force }
  $exePath = Join-Path $AppBin "rbot.exe"
  if (Test-Path $exePath) { Remove-Item $exePath -Force }

  if ($KeepConfig) {
    Write-Host "Uninstall complete. Kept config/data at $RbotHome (RBOT_KEEP_CONFIG set)."
    return
  }
  if (Test-Path $RbotHome) { Remove-Item $RbotHome -Recurse -Force }
  Write-Host "Uninstall complete."
}

function Run-Rbot {
  $exePath = Join-Path $AppBin "rbot.exe"
  if (-not (Test-Path $exePath)) {
    throw "rbot is not installed at $exePath"
  }
  Push-Location $RbotHome
  & $exePath @Args
  Pop-Location
}

switch ($Command) {
  "install" { Install-OrUpdate "install" }
  "update" { Install-OrUpdate "update" }
  "uninstall" { Uninstall-Rbot }
  "run" { Run-Rbot }
  "help" { Show-Usage }
  "-h" { Show-Usage }
  "--help" { Show-Usage }
  default {
    Write-Host "unknown command: $Command"
    Show-Usage
    exit 1
  }
}
