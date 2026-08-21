param(
    [string]$Destination = (Join-Path ([Environment]::GetFolderPath('Desktop')) 'ReproDeck-Acceptance-Fixture'),
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw 'Git is required for the acceptance fixture.'
}

if (Test-Path $Destination) {
    if (-not $Force) {
        throw "Fixture already exists: $Destination`nRun again with -Force to recreate it."
    }
    Remove-Item $Destination -Recurse -Force
}

New-Item -ItemType Directory -Path $Destination | Out-Null
Set-Location $Destination

git init | Out-Null
git config user.name 'ReproDeck Fixture'
git config user.email 'fixture@reprodeck.invalid'

@'
BAD
'@ | Set-Content -Path 'state.txt' -Encoding ASCII

@'
# ReproDeck acceptance fixture

The original repository begins with `state.txt` containing `BAD`.
The ReproDeck shadow workspace should be changed to `GOOD` while this original repository stays unchanged until Apply.
'@ | Set-Content -Path 'README.md' -Encoding ASCII

git add -- state.txt README.md
git commit -m 'fixture: initial broken state' | Out-Null

Write-Host ''
Write-Host 'Acceptance fixture created.' -ForegroundColor Green
Write-Host "Repository: $Destination" -ForegroundColor Cyan
Write-Host ''
Write-Host 'Use these values in ReproDeck:' -ForegroundColor Yellow
Write-Host '  Title:             Shadow apply acceptance'
Write-Host '  Expected:          state.txt contains GOOD'
Write-Host '  Actual:            state.txt contains BAD'
Write-Host '  Executable:        git'
Write-Host '  Arguments:         grep -q GOOD -- state.txt'
Write-Host '  Expected exit:     0'
Write-Host ''
Write-Host 'Test flow:' -ForegroundColor Yellow
Write-Host '  1. Create the session with this repository.'
Write-Host '  2. Run BEFORE -> it must be Failed / exit 1.'
Write-Host '  3. Open the isolated workspace and change state.txt from BAD to GOOD.'
Write-Host '  4. Changes -> Checkpoint changes.'
Write-Host '  5. Run AFTER -> it must be Passed / exit 0.'
Write-Host '  6. Verification must say Verified fix.'
Write-Host '  7. Confirm the ORIGINAL state.txt is still BAD.'
Write-Host '  8. Changes -> Apply.'
Write-Host '  9. The ORIGINAL state.txt must now be GOOD, with no commit created by ReproDeck.'
