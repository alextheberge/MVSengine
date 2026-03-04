param(
  [string]$Repo = $(if ($env:MVS_REPO) { $env:MVS_REPO } else { "alextheberge/MVSengine" }),
  [string]$Version = $(if ($env:MVS_VERSION) { $env:MVS_VERSION } else { "latest" }),
  [string]$InstallDir = $(if ($env:MVS_INSTALL_DIR) { $env:MVS_INSTALL_DIR } else { "$HOME\\.local\\bin" })
)

$ErrorActionPreference = "Stop"
$BinName = "mvs-manager"

$target = "x86_64-pc-windows-msvc"

if ($Version -eq "latest") {
  $latest = Invoke-RestMethod -Method Get -Uri "https://api.github.com/repos/$Repo/releases/latest"
  $Version = $latest.tag_name
}

$versionLabel = $Version.TrimStart("v")
$archiveName = "$BinName-$versionLabel-$target.zip"
$base = "https://github.com/$Repo/releases/download/$Version"
$archiveUrl = "$base/$archiveName"
$checksumsUrl = "$base/checksums.txt"

$workDir = Join-Path $env:TEMP ("mvs-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $workDir | Out-Null

$archivePath = Join-Path $workDir $archiveName
$checksumsPath = Join-Path $workDir "checksums.txt"

Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath
Invoke-WebRequest -Uri $checksumsUrl -OutFile $checksumsPath

$expectedLine = Select-String -Path $checksumsPath -Pattern [regex]::Escape($archiveName)
if (-not $expectedLine) {
  throw "Checksum entry missing for $archiveName"
}
$expected = ($expectedLine.Line -split "\s+")[0]
$actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($expected.ToLowerInvariant() -ne $actual) {
  throw "Checksum mismatch for $archiveName"
}

$extractDir = Join-Path $workDir "extract"
Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -Path (Join-Path $extractDir "$BinName.exe") -Destination (Join-Path $InstallDir "$BinName.exe") -Force

Write-Host "Installed $BinName.exe to $InstallDir"
Write-Host "Run '$BinName --help' in a new terminal to verify installation."
