$ErrorActionPreference = "Stop"

$Repo = if ($env:RBOT_REPO) { $env:RBOT_REPO } else { "null12138/rbot" }
$Version = if ($env:RBOT_VERSION) { $env:RBOT_VERSION } else { "latest" }
$RbotHome = if ($env:RBOT_HOME) { $env:RBOT_HOME } else { Join-Path $env:USERPROFILE ".rbot" }
$BinDir = if ($env:RBOT_BIN_DIR) { $env:RBOT_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "rbot\bin" }
$AppBin = Join-Path $RbotHome "bin"

New-Item -ItemType Directory -Force $RbotHome, $BinDir, $AppBin, (Join-Path $RbotHome "config"), (Join-Path $RbotHome "skills"), (Join-Path $RbotHome "data"), (Join-Path $RbotHome "memory") | Out-Null

$arch = $env:PROCESSOR_ARCHITECTURE
switch ($arch) {
  "AMD64" { $target = "x86_64-pc-windows-msvc" }
  "ARM64" { throw "windows arm64 releases are not available yet" }
  default { throw "unsupported arch: $arch" }
}

$apiUrl = "https://api.github.com/repos/$Repo/releases/tags/$Version"

$release = Invoke-RestMethod -Uri $apiUrl -Headers @{"User-Agent"="rbot-install"}
$assetName = "rbot-$target.zip"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if (-not $asset) {
  throw "no release asset for $target"
}

$temp = Join-Path $env:TEMP ("rbot-install-" + [guid]::NewGuid())
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

if (-not (Test-Path (Join-Path $RbotHome "config\config.toml"))) {
  Write-Host "Running rbot init..."
  Push-Location $RbotHome
  & (Join-Path $AppBin "rbot.exe") init
  Pop-Location
}

$pathParts = $env:PATH -split ";"
if ($pathParts -notcontains $BinDir) {
  Write-Host "NOTE: add $BinDir to your PATH to use 'rbot'"
}

Write-Host "Install complete. Run: rbot"
