$ErrorActionPreference = "Stop"

$Repo = if ($env:RBOT_REPO) { $env:RBOT_REPO } else { "null12138/rbot" }
$Version = if ($env:RBOT_VERSION) { $env:RBOT_VERSION } else { "latest" }
$RbotHome = if ($env:RBOT_HOME) { $env:RBOT_HOME } else { Join-Path $env:USERPROFILE ".rbot" }
$BinDir = if ($env:RBOT_BIN_DIR) { $env:RBOT_BIN_DIR } else { Join-Path $env:LOCALAPPDATA "rbot\bin" }
$AppBin = Join-Path $RbotHome "bin"
function Ensure-SectionKey {
  param(
    [string]$Path,
    [string]$Section,
    [string]$Key,
    [string]$Value
  )
  $lines = Get-Content -Path $Path
  $secLine = "[$Section]"
  $secStart = -1
  for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i].Trim() -eq $secLine) { $secStart = $i; break }
  }
  if ($secStart -eq -1) {
    $lines += ""
    $lines += $secLine
    $lines += "$Key = $Value"
    Set-Content -Path $Path -Value $lines -Encoding UTF8
    return
  }
  $hasKey = $false
  $secEnd = $lines.Count
  for ($i = $secStart + 1; $i -lt $lines.Count; $i++) {
    if ($lines[$i].Trim().StartsWith("[")) { $secEnd = $i; break }
    if ($lines[$i] -match "^\s*$([regex]::Escape($Key))\s*=") { $hasKey = $true; break }
  }
  if ($hasKey) { return }
  $before = $lines[0..($secEnd - 1)]
  $after = @()
  if ($secEnd -lt $lines.Count) { $after = $lines[$secEnd..($lines.Count - 1)] }
  $newLines = @()
  $newLines += $before
  $newLines += "$Key = $Value"
  if ($after.Count -gt 0) { $newLines += $after }
  Set-Content -Path $Path -Value $newLines -Encoding UTF8
}

function Update-Config {
  $cfg = Join-Path $RbotHome "config\config.toml"
  if (-not (Test-Path $cfg)) { return }
  $bak = "$cfg.bak.$(Get-Date -Format yyyyMMddHHmmss)"
  Copy-Item $cfg $bak -Force
  Ensure-SectionKey -Path $cfg -Section "llm" -Key "overall_timeout_secs" -Value "600"
  Ensure-SectionKey -Path $cfg -Section "tools.shell" -Key "mode" -Value "\"blocklist\""
  Ensure-SectionKey -Path $cfg -Section "tools.shell" -Key "blocklist" -Value "[\"rm\", \"sudo\", \"shutdown\", \"reboot\", \"mkfs\", \"dd\"]"
}

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

$temp = Join-Path $env:TEMP ("rbot-update-" + [guid]::NewGuid())
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

Write-Host "Update complete. Run: rbot"

Update-Config
