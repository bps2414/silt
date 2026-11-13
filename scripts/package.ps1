# Builds Silt and packs it the way a release does: dist\Silt-<version>-portable.zip and
# dist\Silt-Setup-<version>.exe (Inno Setup). Local only: the real release comes from a `vX.Y.Z`
# tag through .github/workflows/release.yml.
# Usage: pwsh -File scripts\package.ps1 [-SkipTests] [-SkipBuild]
param([switch]$SkipTests, [switch]$SkipBuild)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Silt $version"

if (-not $SkipBuild) {
  cargo build --release --workspace
  if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
if (-not $SkipTests) {
  cargo test --release --workspace
  if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}

# What the user gets: the app, the CLI, the licenses and the readme.
$src = "dist\Silt"
if (Test-Path $src) { Remove-Item $src -Recurse -Force }
New-Item -ItemType Directory -Force $src | Out-Null
Copy-Item target\release\silt.exe, target\release\silt-cli.exe, LICENSE-MIT, LICENSE-APACHE $src
Copy-Item README.md "$src\README.md"

# Portable zip.
$zip = "dist\Silt-$version-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$src\*" -DestinationPath $zip
Write-Host "Portable:  $zip"

# Installer.
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "C:\Program Files (x86)\Inno Setup 6\ISCC.exe", "C:\Program Files\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found (winget install JRSoftware.InnoSetup)" }
& $iscc /Q "/DAppVersion=$version" "/DSrcDir=..\dist\Silt" installer\silt.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
$setup = "dist\Silt-Setup-$version.exe"
Write-Host "Installer: $setup"
foreach ($f in @($zip, $setup)) {
  Write-Host ("  {0}  sha256 {1}" -f (Split-Path $f -Leaf), (Get-FileHash $f -Algorithm SHA256).Hash)
}
