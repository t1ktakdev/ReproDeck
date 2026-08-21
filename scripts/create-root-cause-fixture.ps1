param(
    [string]$Path = "$HOME\Desktop\ReproDeck-RootCause-Fixture",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

if (Test-Path $Path) {
    if (-not $Force) {
        throw "Fixture already exists: $Path. Re-run with -Force to recreate it."
    }
    Remove-Item -Recurse -Force $Path
}

New-Item -ItemType Directory -Force -Path $Path, "$Path\src", "$Path\tests" | Out-Null

$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
function Write-Utf8NoBom([string]$Target, [string]$Content) {
    [System.IO.File]::WriteAllText($Target, $Content, $Utf8NoBom)
}

$packageJson = @'
{
  "name": "reprodeck-root-cause-fixture",
  "version": "1.0.0",
  "private": true,
  "description": "Dependency-free fixture for ReproDeck Investigation Case and causal experiment flow.",
  "scripts": {
    "test": "node tests/state.test.js"
  }
}
'@
Write-Utf8NoBom "$Path\package.json" $packageJson

$state = @'
BAD
'@
Write-Utf8NoBom "$Path\src\state.txt" $state

$test = @'
const fs = require("node:fs");
const state = fs.readFileSync("src/state.txt", "utf8").trim();
if (state !== "GOOD") {
  console.error(`AssertionError: expected state GOOD but received ${state} at src/state.txt:1:1`);
  process.exit(1);
}
console.log("state criterion passed");
'@
Write-Utf8NoBom "$Path\tests\state.test.js" $test

$readme = @'
# ReproDeck Root Cause fixture

Intentional baseline: `src/state.txt` contains `BAD`, so `npm test` exits 1.

For a controlled Fix Workspace intervention, change only `src/state.txt` from `BAD` to `GOOD`, checkpoint that change, and rerun the exact same criterion. The original repository must remain `BAD` throughout the Investigation Case.
'@
Write-Utf8NoBom "$Path\README.md" $readme

Push-Location $Path
try {
    git init | Out-Null
    git config user.name "ReproDeck Fixture"
    git config user.email "fixture@reprodeck.invalid"
    git config core.autocrlf false
    git add .
    git commit -m "fixture: failing causal baseline" | Out-Null
} finally {
    Pop-Location
}

Write-Host "Created Root Cause fixture: $Path" -ForegroundColor Green
Write-Host "Baseline: npm test -> FAIL (state BAD)"
Write-Host "Controlled intervention inside Fix Workspace: src/state.txt BAD -> GOOD"
Write-Host "Expected experiment: PASS + original repo still BAD -> Supports hypothesis"
