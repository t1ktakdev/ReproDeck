$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

Write-Host 'Installing/updating frontend dependencies…' -ForegroundColor Cyan
npm install
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host 'Starting ReproDeck desktop development build…' -ForegroundColor Green
npm run tauri dev
