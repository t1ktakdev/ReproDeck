param([switch]$SkipVerify)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

if (-not $SkipVerify) {
    & (Join-Path $PSScriptRoot 'verify-windows.ps1')
}

Write-Host "`n=== Windows NSIS production build ===" -ForegroundColor Cyan
cmd.exe /d /s /c 'npm run release:windows 2>&1'
if ($LASTEXITCODE -ne 0) { throw "Tauri NSIS build failed with exit code $LASTEXITCODE" }

$installers = Get-ChildItem -Path '.\target\release\bundle\nsis' -Filter '*.exe' -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending
if (-not $installers) { throw 'Build completed but no NSIS installer was found under target\release\bundle\nsis.' }

Write-Host "`nWindows installer:" -ForegroundColor Green
foreach ($installer in $installers) {
    $hash = Get-FileHash $installer.FullName -Algorithm SHA256
    Write-Host $installer.FullName -ForegroundColor Green
    Write-Host "SHA256: $($hash.Hash)" -ForegroundColor DarkGray
}
