param([switch]$SkipFrontend)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

function Run-Step {
  param([string]$Name,[scriptblock]$Command)
  Write-Host ""
  Write-Host "GATE: $Name" -ForegroundColor Cyan
  & $Command
  if ($LASTEXITCODE -ne 0) {
    Write-Host "GATE_FAILED: $Name (exit $LASTEXITCODE)" -ForegroundColor Red
    exit $LASTEXITCODE
  }
  Write-Host "GATE_OK: $Name" -ForegroundColor Green
}

Push-Location $Root
try {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "cargo is not on PATH" -ForegroundColor Red
    exit 127
  }
  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host "git is not on PATH" -ForegroundColor Red
    exit 127
  }

  Run-Step "cargo check" {
    cargo check --quiet --workspace --all-targets --all-features
  }
  Run-Step "cargo fmt --check" {
    cargo fmt --all -- --check
  }
  Run-Step "cargo clippy -D warnings" {
    cargo clippy --quiet --workspace --all-targets --all-features -- -D warnings
  }

  # IMPORTANT: these are REAL test executions. Never replace with --no-run.
  Run-Step "cargo test" {
    cargo test --quiet --workspace --all-features
  }
  Run-Step "cargo test serial" {
    cargo test --quiet --workspace --all-features -- --test-threads=1
  }

  Run-Step "git diff --check" {
    git diff --check
  }

  if (-not $SkipFrontend -and
      (Test-Path "package.json") -and
      (Get-Command npm -ErrorAction SilentlyContinue)) {
    $pkg = Get-Content "package.json" -Raw | ConvertFrom-Json
    $names = @($pkg.scripts.PSObject.Properties.Name)
    foreach ($scriptName in @("typecheck","lint","build")) {
      if ($names -contains $scriptName) {
        Run-Step "npm run $scriptName" {
          npm run --silent $scriptName
        }
      }
    }
  }

  Write-Host ""
  Write-Host "QUALITY_GATE_PASS" -ForegroundColor Green
  exit 0
}
finally {
  Pop-Location
}