param(
    [string]$Path = "$HOME\Desktop\ReproDeck-BugHunter-Fixture",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

if (Test-Path $Path) {
    if (-not $Force) {
        throw "Fixture already exists: $Path. Re-run with -Force to recreate it."
    }
    Remove-Item -Recurse -Force $Path
}

New-Item -ItemType Directory -Force -Path $Path, "$Path\scripts", "$Path\src", "$Path\tests" | Out-Null

$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
function Write-Utf8NoBom([string]$Target, [string]$Content) {
    [System.IO.File]::WriteAllText($Target, $Content, $Utf8NoBom)
}

$packageJson = @'
{
  "name": "reprodeck-bug-hunter-fixture",
  "version": "1.0.0",
  "private": true,
  "description": "Dependency-free fixture for ReproDeck Smart Bug Hunter planning and failure clustering.",
  "scripts": {
    "check": "node scripts/check.js",
    "test": "node scripts/test.js",
    "build": "node scripts/build.js"
  }
}
'@
Write-Utf8NoBom "$Path\package.json" $packageJson

$checkScript = @'
console.error("error[E0308]: mismatched types at src\\engine.js:18:7");
console.error("expected string, found number");
process.exit(1);
'@
Write-Utf8NoBom "$Path\scripts\check.js" $checkScript

$testScript = @'
console.error("AssertionError: expected refresh state READY but received STALE at tests\\engine.test.js:12:3");
process.exit(1);
'@
Write-Utf8NoBom "$Path\scripts\test.js" $testScript

$buildScript = @'
console.error("error[E0308]: mismatched types at src\\engine.js:27:4");
console.error("build stopped after compiler diagnostic");
process.exit(1);
'@
Write-Utf8NoBom "$Path\scripts\build.js" $buildScript

$engineSource = @'
export function state(value) {
  return value;
}
'@
Write-Utf8NoBom "$Path\src\engine.js" $engineSource

$readme = @'
# ReproDeck Bug Hunter fixture

The project intentionally exposes three deterministic checks with two root failure signatures.
No dependencies are required and no network access should be needed.
'@
Write-Utf8NoBom "$Path\README.md" $readme

Push-Location $Path
try {
    git init | Out-Null
    git config user.name "ReproDeck Fixture"
    git config user.email "fixture@reprodeck.invalid"
    git config core.autocrlf false
    git add .
    git commit -m "fixture: initial failing health surface" | Out-Null
} finally {
    Pop-Location
}

Write-Host "Created Bug Hunter fixture: $Path" -ForegroundColor Green
Write-Host "Expected smart plan order: check -> test -> build"
Write-Host "Expected failure groups: compiler E0308 (check + build), test assertion (test)"
