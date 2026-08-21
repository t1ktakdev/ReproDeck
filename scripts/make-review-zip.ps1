param([string]$OutputPath)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Parent = Split-Path -Parent $Root
$Leaf = Split-Path -Leaf $Root
$Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$Out = if ($OutputPath) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath) } else { Join-Path $env:USERPROFILE "Downloads\ReproDeck-review-$Stamp.zip" }
if (-not (Get-Command tar.exe -ErrorAction SilentlyContinue)) {
    throw "tar.exe was not found. Windows 11 normally includes it."
}
& tar.exe -a -c -f $Out `
  --exclude=target `
  --exclude=node_modules `
  --exclude=dist `
  --exclude=build `
  --exclude=.git `
  --exclude=.vite `
  --exclude=coverage `
  --exclude=field-test-logs `
  --exclude=bench-results `
  -C $Parent $Leaf
if ($LASTEXITCODE -ne 0) { throw "tar.exe failed with exit code $LASTEXITCODE" }
Write-Host "Review archive created:" -ForegroundColor Green
Write-Host $Out
