param(
    [switch]$SkipInstall,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

$logDir = Join-Path $PWD 'field-test-logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

function Run-NativeStep {
    param(
        [Parameter(Mandatory=$true)][string]$Name,
        [Parameter(Mandatory=$true)][string]$CommandLine
    )
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    $log = Join-Path $logDir (($Name -replace '[^A-Za-z0-9_-]', '_') + '.log')
    cmd.exe /d /s /c "$CommandLine 2>&1" | Tee-Object -FilePath $log
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Write-Host "FAIL: $Name" -ForegroundColor Red
        Write-Host "Log: $log" -ForegroundColor Yellow
        throw "$Name exited with code $code"
    }
    Write-Host "PASS: $Name" -ForegroundColor Green
}

function Require-Tool([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name is required but was not found on PATH."
    }
}

foreach ($tool in @('node','npm','rustc','cargo','git')) { Require-Tool $tool }
Run-NativeStep 'versions' 'node --version && npm --version && rustc --version && cargo --version && git --version'
if (-not $SkipInstall) { Run-NativeStep 'npm_install' 'npm install --no-audit' }
Run-NativeStep 'frontend_typecheck' 'npm run typecheck'
Run-NativeStep 'frontend_tests' 'npm test'
Run-NativeStep 'frontend_build' 'npm run build'
Run-NativeStep 'rust_format' 'cargo fmt --all'
Run-NativeStep 'rust_fmt_check' 'cargo fmt --all -- --check'
Run-NativeStep 'workspace_check' 'cargo check --workspace --all-targets'
Run-NativeStep 'rust_clippy' 'cargo clippy --workspace --all-targets -- -D warnings'
Run-NativeStep 'workspace_tests' 'cargo test --workspace --all-targets'
Run-NativeStep 'cli_doctor' 'cargo run -p reprodeck-cli -- doctor'

Write-Host "`nVERIFICATION PASSED" -ForegroundColor Green
Write-Host 'Frontend typecheck/tests/build, Rust fmt/check/clippy/tests and CLI doctor are green.' -ForegroundColor Green

if ($Release) {
    & (Join-Path $PSScriptRoot 'release-windows.ps1') -SkipVerify
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} else {
    Write-Host 'Run desktop app: npm run tauri dev' -ForegroundColor Cyan
    Write-Host 'Build NSIS installer: .\scripts\release-windows.ps1' -ForegroundColor Cyan
}
