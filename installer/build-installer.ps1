# Build the riviv NSIS installer (#26).
# Usage: .\build-installer.ps1 [-Lang Chinese|English] [-SkipBuild]
#   -Lang: installer language (default Chinese, upstream's default)
#   -SkipBuild: skip cargo build --release (reuse target/release/riviv.exe)
# Requires NSIS 3.x (makensis.exe) — e.g. `winget install NSIS.NSIS`.
param(
    [string]$Lang = "Chinese",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
if ($Lang -ne "Chinese" -and $Lang -ne "English") {
    # riviv.nsi maps only the exact value "Chinese" to the Chinese build;
    # anything else silently builds English — fail the typo loudly.
    throw "Lang must be 'Chinese' or 'English' (got '$Lang')"
}
$repo = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$nsisDir = Join-Path $repo "installer\nsis"

if (-not $SkipBuild) {
    Write-Host "== cargo build --release" -ForegroundColor Cyan
    Push-Location $repo
    try { cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build failed" } }
    finally { Pop-Location }
}

$makensis = Get-Command makensis.exe -ErrorAction SilentlyContinue
if (-not $makensis) {
    $candidate = "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
    if (Test-Path $candidate) { $makensis = $candidate } else { throw "makensis.exe not found (winget install NSIS.NSIS)" }
}

Write-Host "== makensis ($Lang)" -ForegroundColor Cyan
& $makensis "/DLANG=$Lang" (Join-Path $nsisDir "riviv.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
Write-Host "Done: installer\nsis\riviv-*-Setup.exe" -ForegroundColor Green
