param(
  [string]$Path = "$HOME\Desktop\ReproDeck-Health-Fixture",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

if (Test-Path $Path) {
  if (-not $Force) {
    throw "Fixture already exists at $Path. Re-run with -Force to recreate it."
  }
  Remove-Item -LiteralPath $Path -Recurse -Force
}

New-Item -ItemType Directory -Path $Path | Out-Null
Set-Content -LiteralPath (Join-Path $Path "package.json") -Encoding ASCII -NoNewline -Value @'
{
  "name": "reprodeck-health-fixture",
  "version": "1.0.0",
  "private": true,
  "scripts": {
    "test": "node test.js"
  }
}
'@
Set-Content -LiteralPath (Join-Path $Path "state.txt") -Encoding ASCII -NoNewline -Value "BAD"
Set-Content -LiteralPath (Join-Path $Path "test.js") -Encoding ASCII -NoNewline -Value @'
const fs = require("node:fs");
const state = fs.readFileSync("state.txt", "utf8").trim();
if (state !== "GOOD") {
  console.error(`health fixture failed: expected GOOD, got ${state}`);
  process.exit(1);
}
console.log("health fixture passed: GOOD");
'@
Set-Content -LiteralPath (Join-Path $Path ".gitignore") -Encoding ASCII -NoNewline -Value "ignored/`n"
New-Item -ItemType Directory -Path (Join-Path $Path "ignored") | Out-Null
Set-Content -LiteralPath (Join-Path $Path "ignored\secret.txt") -Encoding ASCII -NoNewline -Value "THIS MUST NOT MATTER"

& git -C $Path init | Out-Null
& git -C $Path config user.name "ReproDeck Fixture"
& git -C $Path config user.email "fixture@reprodeck.invalid"
& git -C $Path config core.autocrlf false
& git -C $Path add package.json state.txt test.js .gitignore
& git -C $Path commit -m "fixture: failing project health" | Out-Null

Write-Host "Project Health fixture created." -ForegroundColor Green
Write-Host "Repository: $Path"
Write-Host ""
Write-Host "Manual smoke test:" -ForegroundColor Cyan
Write-Host "  1. Open this project in ReproDeck."
Write-Host "  2. Checks -> select test -> Run selected."
Write-Host "  3. Confirm execution. Expected: Failed and one reproduced problem."
Write-Host "  4. Confirm the original state.txt is still BAD and git status is clean."
Write-Host "  5. Change state.txt to GOOD in the ORIGINAL fixture and commit it yourself."
Write-Host "  6. Rescan the project and run Project Health again. Expected: Passed."
Write-Host "  7. Problems should keep the old failure in cleared history, not call it Verified."
