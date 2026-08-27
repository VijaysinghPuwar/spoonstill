# spoonstill - one-line installer for Windows (D-087).
#
#   irm https://raw.githubusercontent.com/VijaysinghPuwar/spoonstill/master/scripts/install.ps1 | iex
#
# What it does, in order, and nothing else:
#   1. downloads the x64 build and the checksum published beside it, and verifies
#   2. installs still.exe under %LOCALAPPDATA%\Programs\spoonstill\bin
#   3. puts that folder on the user PATH, if it is not there already
#   4. checks for FFmpeg, and offers to install it through winget
#
# It never needs administrator, never writes outside the user profile, and never
# downloads FFmpeg itself - D-012 forbids a runtime binary download, and D-062
# forbids shipping the GPL build this project develops against.

$ErrorActionPreference = 'Stop'

$Repo       = if ($env:SPOONSTILL_REPO) { $env:SPOONSTILL_REPO } else { 'VijaysinghPuwar/spoonstill' }
$Version    = if ($env:SPOONSTILL_VERSION) { $env:SPOONSTILL_VERSION } else { 'latest' }
$InstallDir = if ($env:SPOONSTILL_INSTALL_DIR) { $env:SPOONSTILL_INSTALL_DIR }
              else { Join-Path $env:LOCALAPPDATA 'Programs\spoonstill\bin' }

function Step($m) { Write-Host "==> " -ForegroundColor Cyan -NoNewline; Write-Host $m }
function Ok($m)   { Write-Host "OK   " -ForegroundColor Green -NoNewline; Write-Host $m }
function Warn($m) { Write-Host "!    $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "!    $m" -ForegroundColor Red; exit 1 }

if ([System.Environment]::Is64BitOperatingSystem -ne $true) {
  Die "spoonstill needs 64-bit Windows."
}
$Target = 'x86_64-pc-windows-msvc'

Step "Installing spoonstill for Windows ($Target)"

# --- 1. download and verify --------------------------------------------------

if ($Version -eq 'latest') {
  try {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                             -Headers @{ 'User-Agent' = 'spoonstill-installer' }
    $Tag = $rel.tag_name
  } catch {
    Die "No published release found at https://github.com/$Repo/releases.
     Until one exists, build from source:  cargo build --release -p spoonstill-cli"
  }
} else {
  $Tag = $Version
}

$Asset = "still-$Tag-$Target.zip"
$Base  = "https://github.com/$Repo/releases/download/$Tag"
$Tmp   = Join-Path ([System.IO.Path]::GetTempPath()) ("spoonstill-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp -Force | Out-Null

try {
  Step "Downloading $Asset"
  $zip = Join-Path $Tmp $Asset
  Invoke-WebRequest -Uri "$Base/$Asset" -OutFile $zip -UseBasicParsing
  $sumFile = "$zip.sha256"
  Invoke-WebRequest -Uri "$Base/$Asset.sha256" -OutFile $sumFile -UseBasicParsing

  Step "Verifying checksum"
  $expected = (Get-Content $sumFile -Raw).Trim().Split(' ')[0].ToLower()
  $actual   = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
  if ($expected -ne $actual) {
    Die "Checksum mismatch. The download is not the published build - nothing installed."
  }

  # --- 2. install ------------------------------------------------------------

  Expand-Archive -Path $zip -DestinationPath $Tmp -Force
  $exe = Join-Path $Tmp 'still.exe'
  if (-not (Test-Path $exe)) { Die "The archive did not contain still.exe." }

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item $exe (Join-Path $InstallDir 'still.exe') -Force
  # Unsigned until M5: clear the download mark so the first run is not a dialog
  # the operator has no way to interpret.
  Unblock-File (Join-Path $InstallDir 'still.exe')

  Ok "Installed $InstallDir\still.exe"
} finally {
  Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# --- 3. PATH -----------------------------------------------------------------

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$InstallDir*") {
  Step "Adding $InstallDir to your PATH"
  $joined = if ([string]::IsNullOrEmpty($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
  [Environment]::SetEnvironmentVariable('Path', $joined, 'User')
  $env:Path = "$env:Path;$InstallDir"
  Warn "Open a new terminal for 'still' to be found there too."
}

# --- 4. FFmpeg ---------------------------------------------------------------

$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
if ($ffmpeg) {
  Ok "Found $($ffmpeg.Source)"
} else {
  Step "FFmpeg is missing - spoonstill cannot render a frame without it"
  if (Get-Command winget -ErrorAction SilentlyContinue) {
    Write-Host "     Installing it with winget..."
    winget install --id Gyan.FFmpeg --source winget --accept-package-agreements --accept-source-agreements
    Warn "Open a new terminal so ffmpeg is on PATH, then you are done."
  } else {
    Warn "winget is not available. Install FFmpeg by hand from https://www.gyan.dev/ffmpeg/builds/
     and put its bin folder on your PATH."
  }
}

if (-not (Get-Command edge-tts -ErrorAction SilentlyContinue)) {
  Write-Host "Optional: 'pipx install edge-tts' if you want text read aloud by a neural voice." -ForegroundColor DarkGray
}

Write-Host ""
Write-Host "Ready." -ForegroundColor Green -NoNewline
Write-Host " Make a film out of a folder you already have:"
Write-Host ""
Write-Host "  still new C:\holiday C:\Users\you\Pictures\trip"
Write-Host "  still validate C:\holiday"
Write-Host "  still render C:\holiday --out C:\holiday.mp4"
Write-Host ""
Write-Host "  still --help            every command"
